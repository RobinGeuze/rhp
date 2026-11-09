//! The linked-class cache: php's inheritance cache (zend_inheritance.c,
//! `zend_inheritance_cache_get` / `_add`) for the requests a thread
//! serves.
//!
//! With opcache php links a class once and keeps the result immutable in
//! shared memory; a later request that declares the same class against
//! the same parent, interfaces and traits takes the linked copy, with
//! fresh static members. rhp does the same per thread: a user class
//! linked without side effects is kept here as a snapshot taken right
//! after linking, and a later request whose link of that declaration
//! sees the same inputs installs a copy of it instead of linking again.
//!
//! The inputs that must match:
//! - the declaration (`ClassProto`, by address; the entry keeps it alive),
//!   its unit, whether it is early-bound, and the id it gets;
//! - the stamp and state of its parent, interfaces and traits. A stamp
//!   names one linked entry: a fresh link gets a new one, a copy from this
//!   cache (or of the linked prelude) keeps the cached one. The state
//!   is a digest of the changes a request made to a class's constants,
//!   property defaults and static initializers, values included, which
//!   linking copies from;
//! - every other class the inheritance checks looked up by name (php's
//!   `dependencies`), which must resolve to the same entry, or still to
//!   none.
//!
//! A link that raised a diagnostic, autoloaded, saw a class still loading
//! its dependencies or left variance obligations is not cached, so a
//! copy never skips something php would do. Neither is one whose
//! snapshot would hold objects, references or resources.
//!
//! The cache starts with the thread's second request (as the linked
//! prelude does), so a CLI process pays nothing for it, and keeps a link
//! the second time its declaration links, so code compiled afresh for
//! each request (opcache off, eval) costs no snapshots.

use std::cell::RefCell;
use std::hash::{BuildHasher, Hasher};
use std::rc::Rc;

use indexmap::IndexMap;
use rustc_hash::{FxBuildHasher, FxHashMap};

use rhp_bytecode::ClassProto;
use rhp_values::{ClassId, ClassKind, Value};

use crate::classes::{ClassEntry, StaticProp};
use crate::{UnitId, Vm};

/// A recorded lookup of the class being linked itself.
const SELF_STAMP: u64 = u64::MAX;
/// Links (or first sightings) stored between sweeps for declarations
/// nothing else holds any more (a recompiled file, an eval, a script
/// opcache doesn't keep).
const SWEEP_EVERY: usize = 4096;
/// Links kept: past this, after a sweep, the cache is emptied.
const MAX_ENTRIES: usize = 1 << 15;
/// Links kept per declaration (one per distinct set of inputs).
const MAX_VARIANTS: usize = 4;

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
    /// The next stamp a fresh link gets.
    static NEXT_STAMP: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
    /// (classes installed from the cache, links stored) on this thread.
    static STATS: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
}

/// This thread's linked-class cache use so far: (classes installed from
/// the cache, links stored in it).
pub fn link_cache_stats() -> (u64, u64) {
    STATS.with(|s| s.get())
}

/// A new stamp for a freshly linked class.
pub(crate) fn fresh_stamp() -> u64 {
    NEXT_STAMP.with(|n| {
        let s = n.get();
        n.set(s + 1);
        s
    })
}

#[derive(Default)]
struct Cache {
    /// Keyed by the declaration's address.
    by_proto: FxHashMap<usize, Vec<Rc<Linked>>>,
    /// Declarations linked once, not yet cached: a link is cached the
    /// second time its declaration links, so code compiled afresh for
    /// every request (no opcache, eval) costs no snapshots.
    seen: FxHashMap<usize, std::rc::Weak<ClassProto>>,
    entries: usize,
    stored_since_sweep: usize,
}

impl Cache {
    /// Drop the links of declarations only the cache still holds (each
    /// link holds its declaration twice: `proto` and `entry.proto`).
    fn sweep(&mut self) {
        self.by_proto.retain(|_, v| {
            v.first()
                .is_some_and(|l| Rc::strong_count(&l.proto) > 2 * v.len())
        });
        self.seen.retain(|_, w| w.strong_count() > 0);
        self.entries = self.by_proto.values().map(Vec::len).sum();
        self.stored_since_sweep = 0;
        if self.entries >= MAX_ENTRIES {
            *self = Cache::default();
        }
    }
}

/// One cached link.
struct Linked {
    /// Keeps the declaration (and so its address) alive.
    proto: Rc<ClassProto>,
    id: ClassId,
    unit: UnitId,
    early: bool,
    /// The prelude's stamp (class 0).
    prelude: u64,
    /// (stamp, state) of the parent, then the interfaces and the traits in
    /// declaration order.
    deps: Vec<(u64, u64)>,
    /// Other classes looked up by lowercase name, with the stamp each
    /// resolved to (0: none, [`SELF_STAMP`]: the class itself).
    lookups: Vec<(Vec<u8>, u64)>,
    entry: ClassEntry,
    /// The static properties that alias the parent's cells.
    inherited_statics: Vec<bool>,
}

/// What one link in progress records.
pub(crate) struct LinkRec {
    id: ClassId,
    lookups: Vec<(Vec<u8>, u64)>,
    tainted: bool,
    diag_seq: u64,
}

/// The VM's side of the cache.
#[derive(Default)]
pub(crate) struct LinkCacheState {
    /// The thread served a request before this one.
    pub(crate) on: bool,
    /// The links in progress, innermost last.
    recs: RefCell<Vec<LinkRec>>,
}

impl LinkCacheState {
    /// The link in progress cannot be cached.
    pub(crate) fn taint(&self) {
        if let Some(r) = self.recs.borrow_mut().last_mut() {
            r.tainted = true;
        }
    }
}

/// A value a snapshot may keep across requests: no objects, references or
/// resources, at any depth.
fn plain(v: &Value) -> bool {
    match v {
        Value::Object(_) | Value::Closure(_) | Value::Resource(_) | Value::Ref(_) => false,
        Value::Array(a) => a.iter().all(|(_, v)| plain(v)),
        _ => true,
    }
}

fn fresh_cell(c: &Rc<RefCell<Value>>) -> Rc<RefCell<Value>> {
    Rc::new(RefCell::new(c.borrow().copy_slot()))
}

/// A copy of linked entry `e` with its own static cells; `stamp` and
/// `state` set, enum cases left out.
fn copy_entry(
    e: &ClassEntry,
    static_props: IndexMap<Vec<u8>, StaticProp, FxBuildHasher>,
) -> ClassEntry {
    ClassEntry {
        name: e.name.clone(),
        kind: e.kind,
        flags: e.flags,
        parent: e.parent,
        ancestors: e.ancestors.clone(),
        interfaces: e.interfaces.clone(),
        traits: e.traits.clone(),
        methods: e.methods.clone(),
        props: e.props.clone(),
        layout: e.layout.clone(),
        prop_visible: e.prop_visible.clone(),
        own_private: e.own_private.clone(),
        constants: e.constants.clone(),
        static_props,
        enum_cases: Vec::new(),
        enum_backing: e.enum_backing,
        updated: false,
        proto: e.proto.clone(),
        unit: e.unit,
        decl_file: e.decl_file.clone(),
        decl_line: e.decl_line,
        handlers: e.handlers,
        create_object: e.create_object,
        hooked: e.hooked,
        hooked_iter: e.hooked_iter,
        stringable: e.stringable,
        checked_statics: e.checked_statics,
        stamp: e.stamp,
        state: 0,
        cache: Default::default(),
    }
}

impl Vm {
    /// Whether a link of `proto` takes part in the cache.
    fn link_cacheable(&self, proto: &ClassProto) -> bool {
        self.link_cache.on && self.internal_classes > 0 && !proto.decl_file.is_empty()
    }

    /// (stamp, state) of the classes `proto` names as parent, interfaces
    /// and traits, all linked.
    fn link_deps(&self, proto: &ClassProto) -> Vec<(u64, u64)> {
        let names = proto
            .parent
            .iter()
            .chain(&proto.interfaces)
            .chain(&proto.traits);
        names
            .map(|n| {
                let id = crate::classes::with_lc(n.as_bytes(), |k| self.class_ids.get(k).copied());
                id.map_or((0, 0), |id| {
                    let e = &self.classes[id as usize];
                    (e.stamp, e.state)
                })
            })
            .collect()
    }

    fn prelude_stamp(&self) -> u64 {
        self.classes.first().map_or(0, |e| e.stamp)
    }

    /// A class looked up by its lowercase name while linking: recorded, so
    /// a cached link is reused only while the name resolves alike.
    pub(crate) fn link_lookup(&self, key: &[u8]) -> Option<ClassId> {
        let id = self.class_ids.get(key).copied();
        if self.link_cache.on {
            if let Some(r) = self.link_cache.recs.borrow_mut().last_mut() {
                let stamp = match id {
                    Some(id) if id == r.id => SELF_STAMP,
                    Some(id) => self.classes[id as usize].stamp,
                    None => 0,
                };
                r.lookups.push((key.to_vec(), stamp));
            }
        }
        id
    }

    /// Link `proto` from the cache when an earlier request linked it from
    /// the same inputs. `Some(Ok(true))`: installed as the next class.
    pub(crate) fn link_from_cache(
        &mut self,
        proto: &Rc<ClassProto>,
        unit: UnitId,
        early: bool,
    ) -> Option<Result<bool, rhp_values::Thrown>> {
        if !self.link_cacheable(proto) {
            return None;
        }
        let candidates = CACHE.with(|c| {
            c.borrow()
                .by_proto
                .get(&(Rc::as_ptr(proto) as usize))
                .cloned()
        })?;
        let id = self.classes.len() as ClassId;
        let prelude = self.prelude_stamp();
        let deps = self.link_deps(proto);
        let hit = candidates.into_iter().find(|l| {
            l.id == id
                && Rc::ptr_eq(&l.proto, proto)
                && l.unit == unit
                && l.early == early
                && l.prelude == prelude
                && l.deps == deps
                && l.lookups.iter().all(|(k, stamp)| {
                    *stamp == SELF_STAMP
                        || match self.class_ids.get(k) {
                            Some(&c) => self.classes[c as usize].stamp == *stamp,
                            None => *stamp == 0,
                        }
                })
        })?;
        STATS.with(|s| s.set((s.get().0 + 1, s.get().1)));
        Some(self.install_linked(&hit, proto, unit))
    }

    fn install_linked(
        &mut self,
        l: &Linked,
        proto: &Rc<ClassProto>,
        unit: UnitId,
    ) -> Result<bool, rhp_values::Thrown> {
        let id = l.id;
        // Inherited statics alias the parent's cells as they are now; the
        // others start from the snapshot's values.
        let parent = l
            .entry
            .parent
            .map(|p| &self.classes[p as usize].static_props);
        let mut static_props =
            IndexMap::with_capacity_and_hasher(l.entry.static_props.len(), FxBuildHasher);
        for ((k, sp), &inh) in l.entry.static_props.iter().zip(&l.inherited_statics) {
            let sp = match parent.and_then(|p| p.get(k)).filter(|_| inh) {
                Some(psp) => psp.clone(),
                None => StaticProp {
                    cell: fresh_cell(&sp.cell),
                    default: fresh_cell(&sp.default),
                    ..sp.clone()
                },
            };
            static_props.insert(k.clone(), sp);
        }
        let entry = copy_entry(&l.entry, static_props);
        self.class_ids
            .insert(proto.name.as_bytes().to_ascii_lowercase(), id);
        self.class_order.push(self.decl_classes.len());
        self.classes.push(entry);
        if proto.kind == ClassKind::Enum {
            self.materialize_enum_cases(id, proto, unit);
        }
        Ok(true)
    }

    /// Start recording the link of the class that gets id `id`.
    pub(crate) fn link_rec_begin(&self, proto: &ClassProto) -> bool {
        if !self.link_cacheable(proto) {
            return false;
        }
        let mut recs = self.link_cache.recs.borrow_mut();
        // A link inside another one (an autoload from a variance check):
        // the outer one is not cached.
        if let Some(r) = recs.last_mut() {
            r.tainted = true;
        }
        recs.push(LinkRec {
            id: self.classes.len() as ClassId,
            lookups: Vec::new(),
            tainted: false,
            diag_seq: self.diag_seq,
        });
        true
    }

    /// End recording; `linked`: the link succeeded, as class `rec.id`.
    pub(crate) fn link_rec_end(
        &mut self,
        proto: &Rc<ClassProto>,
        unit: UnitId,
        early: bool,
        linked: bool,
    ) {
        let Some(rec) = self.link_cache.recs.borrow_mut().pop() else {
            return;
        };
        let key = Rc::as_ptr(proto) as usize;
        let second = CACHE.with(|c| {
            let mut c = c.borrow_mut();
            if c.by_proto.contains_key(&key)
                || c.seen.get(&key).is_some_and(|w| w.strong_count() > 0)
            {
                return true;
            }
            rhp_values::mem::uncounted(|| {
                if c.stored_since_sweep >= SWEEP_EVERY {
                    c.sweep();
                }
                c.stored_since_sweep += 1;
                c.seen.insert(key, Rc::downgrade(proto))
            });
            false
        });
        if !second
            || !linked
            || rec.tainted
            || rec.diag_seq != self.diag_seq
            || self.pending_throw.is_some()
            || self.classes.len() != rec.id as usize + 1
        {
            return;
        }
        let e = &self.classes[rec.id as usize];
        let parent = e.parent.map(|p| &self.classes[p as usize]);
        let case_names: Vec<&[u8]> = proto.enum_cases.iter().map(|c| c.name.as_bytes()).collect();
        let mut constants = e.constants.clone();
        if e.kind == ClassKind::Enum {
            for (k, c) in Rc::make_mut(&mut constants).iter_mut() {
                if case_names.contains(&k.as_slice()) {
                    // Each request makes its own case objects.
                    c.value = Value::Null;
                }
            }
        }
        if !constants.values().all(|c| plain(&c.value))
            || !e.props.values().all(|p| plain(&p.default))
            || !e
                .static_props
                .values()
                .all(|s| plain(&s.cell.borrow()) && plain(&s.default.borrow()))
        {
            return;
        }
        let inherited_statics: Vec<bool> = e
            .static_props
            .iter()
            .map(|(k, sp)| {
                parent
                    .and_then(|p| p.static_props.get(k))
                    .is_some_and(|psp| Rc::ptr_eq(&psp.cell, &sp.cell))
            })
            .collect();
        let snapshot = rhp_values::mem::uncounted(|| {
            let mut static_props =
                IndexMap::with_capacity_and_hasher(e.static_props.len(), FxBuildHasher);
            for (k, sp) in &e.static_props {
                static_props.insert(
                    k.clone(),
                    StaticProp {
                        cell: fresh_cell(&sp.cell),
                        default: fresh_cell(&sp.default),
                        ..sp.clone()
                    },
                );
            }
            let mut entry = copy_entry(e, static_props);
            entry.constants = constants;
            Rc::new(Linked {
                proto: proto.clone(),
                id: rec.id,
                unit,
                early,
                prelude: self.prelude_stamp(),
                deps: self.link_deps(proto),
                lookups: rec.lookups,
                entry,
                inherited_statics,
            })
        });
        rhp_values::mem::uncounted(|| {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                if c.stored_since_sweep >= SWEEP_EVERY || c.entries >= MAX_ENTRIES {
                    c.sweep();
                }
                c.stored_since_sweep += 1;
                let v = c.by_proto.entry(key).or_default();
                let full = v.len() >= MAX_VARIANTS;
                if full {
                    v.remove(0);
                }
                v.push(snapshot);
                c.seen.remove(&key);
                STATS.with(|s| s.set((s.get().0, s.get().1 + 1)));
                if !full {
                    c.entries += 1;
                }
            })
        });
    }
}

thread_local! {
    /// Keys the state digests: random, so no request can make two
    /// different change histories collide on purpose.
    static STATE_KEYS: std::hash::RandomState = std::hash::RandomState::new();
}

fn hash_value(v: &Value, h: &mut impl Hasher) {
    match v {
        Value::Undef | Value::Null => h.write_u8(0),
        Value::Bool(b) => {
            h.write_u8(1);
            h.write_u8(*b as u8);
        }
        Value::Int(n) => {
            h.write_u8(2);
            h.write_i64(*n);
        }
        Value::Float(f) => {
            h.write_u8(3);
            h.write_u64(f.to_bits());
        }
        Value::Str(s) => {
            h.write_u8(4);
            h.write_usize(s.as_bytes().len());
            h.write(s.as_bytes());
        }
        Value::Array(a) => {
            h.write_u8(5);
            h.write_usize(a.len());
            for (k, v) in a.iter() {
                match &k {
                    rhp_values::ArrayKey::Int(n) => {
                        h.write_u8(0);
                        h.write_i64(*n);
                    }
                    rhp_values::ArrayKey::Str(s) => {
                        h.write_u8(1);
                        h.write_usize(s.as_bytes().len());
                        h.write(s.as_bytes());
                    }
                }
                hash_value(v, h);
            }
        }
        Value::Object(o) => {
            h.write_u8(6);
            h.write_usize(Rc::as_ptr(o) as *const u8 as usize);
        }
        Value::Ref(r) => {
            h.write_u8(7);
            hash_value(&r.borrow(), h);
        }
        // A snapshot never holds these, and a class whose tables hold
        // them is not cached; a distinct digest is enough.
        Value::Closure(_) | Value::Resource(_) => h.write_u8(8),
    }
}

impl Vm {
    /// Record a change to what linking copies from class `cls`: `name`
    /// set to `v` (or a step without a value). Links against the class
    /// are reused only after the same changes (`Linked::deps`).
    pub(crate) fn note_class_change(&mut self, cls: ClassId, name: &[u8], v: Option<&Value>) {
        let e = &mut self.classes[cls as usize];
        let mut h = STATE_KEYS.with(|k| k.build_hasher());
        h.write_u64(e.state);
        h.write_usize(name.len());
        h.write(name);
        if let Some(v) = v {
            hash_value(v, &mut h);
        }
        // 0 is the state of an unchanged class.
        e.state = h.finish().max(1);
    }
}
