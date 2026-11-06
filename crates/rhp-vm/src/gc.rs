//! The cycle collector: php's zend_gc_collect_cycles (Zend/zend_gc.c).
//!
//! The root buffer lives in `rhp_values::gc`; this is the run. php's
//! trial deletion decrements the reference counts along every edge it
//! walks from the roots; a value left above zero is referenced from
//! outside and keeps everything it reaches alive, the rest is garbage.
//! Here the counts are `Rc::strong_count`s, read without changing them:
//! the walk builds the graph reachable from the roots, subtracts the
//! edges inside it and the references php does not have (the destructor
//! list's), and colours the nodes from that. A node's edges are every
//! reference its storage holds (an object's properties, a reference's
//! value, a closure's captures, a generator's frame), so whatever the walk
//! cannot see counts as an outside reference and is kept.
//!
//! The rest follows php step by step: garbage goes into the buffer in
//! php's order, destructors run first (and the collection runs again once
//! afterwards), and the garbage's handles go back in buffer order.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Instant;

use rhp_values::gc::{self, Node, Slot, FIRST_ROOT};
use rhp_values::{ArrayData, ClosureData, ObjectData, PhpArray, Thrown, Value};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::Vm;

/// A node of the walked graph.
#[derive(Clone)]
enum GNode {
    Obj(Weak<RefCell<ObjectData>>),
    Arr(Weak<ArrayData>),
    Ref(Weak<RefCell<Value>>),
    Clo(Weak<ClosureData>),
}

impl GNode {
    fn of(v: &Value) -> Option<(usize, GNode)> {
        Some(match v {
            Value::Object(h) => (Rc::as_ptr(h) as usize, GNode::Obj(Rc::downgrade(h))),
            Value::Array(a) => (
                Rc::as_ptr(a.rc()) as usize,
                GNode::Arr(Rc::downgrade(a.rc())),
            ),
            Value::Ref(r) => (Rc::as_ptr(r) as usize, GNode::Ref(Rc::downgrade(r))),
            Value::Closure(c) => (Rc::as_ptr(c) as usize, GNode::Clo(Rc::downgrade(c))),
            _ => return None,
        })
    }

    fn of_node(n: &Node) -> GNode {
        match n {
            Node::Obj(w) => GNode::Obj(w.clone()),
            Node::Closure(w) => GNode::Clo(w.clone()),
            Node::Arr(w) => GNode::Arr(w.clone()),
        }
    }

    fn upgrade(&self) -> Option<Value> {
        Some(match self {
            GNode::Obj(w) => Value::Object(w.upgrade()?),
            GNode::Arr(w) => Value::Array(PhpArray::from_rc(w.upgrade()?)),
            GNode::Ref(w) => Value::Ref(w.upgrade()?),
            GNode::Clo(w) => Value::Closure(w.upgrade()?),
        })
    }

    fn strong(&self) -> usize {
        match self {
            GNode::Obj(w) => w.strong_count(),
            GNode::Arr(w) => w.strong_count(),
            GNode::Ref(w) => w.strong_count(),
            GNode::Clo(w) => w.strong_count(),
        }
    }

    fn addr(&self) -> usize {
        match self {
            GNode::Obj(w) => w.as_ptr() as usize,
            GNode::Arr(w) => w.as_ptr() as usize,
            GNode::Ref(w) => w.as_ptr() as usize,
            GNode::Clo(w) => w.as_ptr() as usize,
        }
    }
}

struct GInfo {
    n: GNode,
    /// This node's edges: `edges[first..first + len]`.
    first: u32,
    len: u32,
    rc: isize,
    black: bool,
    /// Visited by gc_collect_white.
    collected: bool,
    /// Reached by gc_remove_nested_data_from_buffer (kept for the rerun).
    kept: bool,
}

#[derive(Default)]
struct Graph {
    nodes: Vec<GInfo>,
    index: FxHashMap<usize, u32>,
    edges: Vec<u32>,
}

impl Graph {
    fn intern(&mut self, addr: usize, n: GNode, todo: &mut Vec<u32>) -> u32 {
        if let Some(&i) = self.index.get(&addr) {
            return i;
        }
        let i = self.nodes.len() as u32;
        self.nodes.push(GInfo {
            n,
            first: 0,
            len: 0,
            rc: 0,
            black: false,
            collected: false,
            kept: false,
        });
        self.index.insert(addr, i);
        todo.push(i);
        i
    }

    fn edges(&self, i: u32) -> &[u32] {
        let n = &self.nodes[i as usize];
        &self.edges[n.first as usize..(n.first + n.len) as usize]
    }

    fn find(&self, addr: usize) -> Option<u32> {
        self.index.get(&addr).copied()
    }
}

/// One run's result.
struct Run {
    count: i64,
    destructors: bool,
}

impl Vm {
    /// gc_collect_cycles(): zend_gc_collect_cycles. Returns how many arrays
    /// and objects were freed.
    pub(crate) fn gc_collect_cycles(&mut self) -> Result<i64, Thrown> {
        let start = Instant::now();
        if gc::active() {
            return Ok(0);
        }
        self.drop_freed_generators();
        let mut total = 0;
        let mut did_rerun = false;
        // Objects whose destructor left them unreferenced: php leaves them
        // in the buffer with a count of zero, and the rerun frees them.
        let mut zombies: Vec<Value> = Vec::new();
        let mut pending: Option<Thrown> = None;
        let mut dtor_ns = 0u64;
        let mut free_ns = 0u64;
        loop {
            if gc::num_roots() == 0 {
                break;
            }
            let run = self.gc_run(&mut zombies, &mut pending, &mut dtor_ns, &mut free_ns);
            let run = match run {
                Ok(r) => r,
                Err(e) => {
                    // An exit from a destructor: what is left stays buffered
                    // as roots.
                    for k in 0..gc::slots().len() as u32 {
                        gc::retag(FIRST_ROOT + k, Slot::Root);
                    }
                    self.gc_unlist_called();
                    gc::set_active(false);
                    return Err(e);
                }
            };
            total += run.count;
            gc::compact();
            if run.destructors && !did_rerun {
                did_rerun = true;
                continue;
            }
            break;
        }
        self.gc_unlist_called();
        drop(zombies);
        let all = start.elapsed().as_nanos() as u64;
        gc::add_times(all, dtor_ns, free_ns);
        match pending {
            Some(e) => Err(e),
            None => Ok(total),
        }
    }

    /// Take the objects whose destructor the collection called off the
    /// destructor list.
    fn gc_unlist_called(&mut self) {
        let called = |h: &rhp_values::ObjectHandle| {
            let id = h.try_borrow().map(|o| o.id).unwrap_or(0);
            gc::flags(id) & gc::CALLED != 0
        };
        if self.destructibles.iter().any(called) {
            let all = std::mem::take(&mut self.destructibles);
            let (gone, kept): (Vec<_>, Vec<_>) = all.into_iter().partition(called);
            self.destructibles = kept;
            for h in &gone {
                let id = h.try_borrow().map(|o| o.id).unwrap_or(0);
                gc::set_flag(id, gc::CALLED, false);
            }
            drop(gone);
        }
    }

    /// The automatic collection php runs when the root buffer is full
    /// (gc_possible_root_when_full), due since a drop found it full.
    #[cold]
    pub(crate) fn gc_auto(&mut self) -> Result<(), Thrown> {
        gc::clear_due();
        if !gc::enabled() || gc::active() {
            gc::flush_deferred();
            return Ok(());
        }
        // The value whose release filled the buffer is held meanwhile
        // (php's GC_ADDREF around the run).
        let held = gc::deferred_values();
        let r = self.gc_collect_cycles();
        let n = match &r {
            Ok(n) => *n,
            Err(_) => 0,
        };
        gc::adjust_threshold(n.max(0) as u32);
        gc::flush_deferred();
        drop(held);
        r.map(|_| ())
    }

    /// The values `v`'s storage holds (php's get_gc handlers).
    fn gc_children(&self, v: &Value, out: &mut Vec<Value>) {
        match v {
            Value::Object(h) => {
                let Ok(o) = h.try_borrow() else {
                    return;
                };
                let internal = self.is_internal_class(o.class)
                    || self
                        .class(o.class)
                        .ancestors
                        .iter()
                        .any(|&a| self.is_internal_class(a));
                for (k, p) in o.properties.iter() {
                    // A prelude class's private `__*` array is its native
                    // storage (a C array or HashTable php's get_gc walks
                    // through, not a counted zend_array; ArrayObject's
                    // `__s` is one). Only when nothing
                    // else reaches it: held once, and not a root itself.
                    if let Value::Array(a) = p {
                        let k = k.as_bytes();
                        if internal
                            && a.ref_count() == 1
                            && gc::slot_of(p) == 0
                            && k.first() == Some(&0)
                            && k[1..].iter().position(|&b| b == 0).is_some_and(|i| {
                                // ArrayObject's storage is a zend_array.
                                let name = &k[i + 2..];
                                name.starts_with(b"__") && name != b"__s"
                            })
                        {
                            out.extend(a.iter().map(|(_, v)| v.clone()));
                            continue;
                        }
                    }
                    out.push(p.clone());
                }
                if let Some(l) = o.lazy() {
                    out.push(l.initializer.clone());
                    if let Some(i) = &l.instance {
                        out.push(Value::Object(i.clone()));
                    }
                }
                let (cls, id) = (o.class, o.id);
                drop(o);
                if self.generators.contains_key(&id) && self.is_generator_class(cls) {
                    self.gen_gc_children(id, out);
                }
                if let Some(get_gc) = self.class(cls).handlers.and_then(|t| t.get_gc) {
                    get_gc(h, out);
                }
            }
            Value::Array(a) => out.extend(a.iter().map(|(_, v)| v.clone())),
            Value::Ref(r) => {
                if let Ok(b) = r.try_borrow() {
                    out.push(b.clone());
                }
            }
            Value::Closure(c) => {
                out.extend(c.captures.iter().cloned());
                if let Some(t) = &c.bound_this {
                    out.push(Value::Object(t.clone()));
                }
                if let Some(w) = &c.wrapped {
                    out.push((**w).clone());
                }
                if let Ok(s) = c.statics.try_borrow() {
                    out.extend(s.iter().map(|(_, r)| Value::Ref(r.clone())));
                }
            }
            _ => {}
        }
    }

    fn is_generator_class(&self, cls: rhp_values::ClassId) -> bool {
        self.class(cls).name.as_bytes() == b"Generator"
    }

    /// Whether garbage object `h` still needs its destructor (php: no
    /// IS_OBJ_DESTRUCTOR_CALLED and a destructor or `dtor_obj`).
    fn gc_needs_dtor(&self, v: &Value, tracked: &FxHashSet<usize>) -> bool {
        let Value::Object(h) = v else {
            return false;
        };
        let Ok(o) = h.try_borrow() else {
            return false;
        };
        let (cls, id) = (o.class, o.id);
        drop(o);
        // On the destructor list and not destructed yet.
        if tracked.contains(&(Rc::as_ptr(h) as usize)) && gc::flags(id) & gc::TRACKED != 0 {
            return true;
        }
        self.is_generator_class(cls) && self.gen_has_frame(id)
    }

    /// One pass of zend_gc_collect_cycles (up to `rerun_gc`).
    fn gc_run(
        &mut self,
        zombies: &mut Vec<Value>,
        pending: &mut Option<Thrown>,
        dtor_ns: &mut u64,
        free_ns: &mut u64,
    ) -> Result<Run, Thrown> {
        gc::count_run();
        gc::set_active(true);

        // gc_mark_roots: the graph reachable from the roots, with every
        // count minus the edges inside it.
        gc::compact();
        let mut g = Graph::default();
        let mut todo = Vec::new();
        let mut roots = Vec::new();
        for (k, s) in gc::slots().into_iter().enumerate() {
            let idx = FIRST_ROOT + k as u32;
            if let Slot::Root(n) = &s {
                if n.upgrade().is_none() {
                    drop(gc::remove_slot(idx));
                    continue;
                }
                let gn = GNode::of_node(n);
                roots.push(g.intern(gn.addr(), gn, &mut todo));
            }
        }
        let mut kids = Vec::new();
        while let Some(i) = todo.pop() {
            let Some(v) = g.nodes[i as usize].n.upgrade() else {
                continue;
            };
            self.gc_children(&v, &mut kids);
            drop(v);
            let first = g.edges.len() as u32;
            for c in kids.drain(..) {
                if let Some((addr, n)) = GNode::of(&c) {
                    let j = g.intern(addr, n, &mut todo);
                    g.edges.push(j);
                }
            }
            let n = &mut g.nodes[i as usize];
            n.first = first;
            n.len = g.edges.len() as u32 - first;
        }
        // The references php does not count: the destructor list's.
        let tracked: FxHashSet<usize> = self
            .destructibles
            .iter()
            .map(|h| Rc::as_ptr(h) as usize)
            .collect();
        let held: FxHashSet<usize> = self
            .marked_destructed
            .iter()
            .map(|h| Rc::as_ptr(h) as usize)
            .collect();
        let zombie_addrs: FxHashSet<usize> = zombies
            .iter()
            .filter_map(|z| GNode::of(z).map(|(a, _)| a))
            .collect();
        for i in 0..g.nodes.len() {
            let n = &g.nodes[i];
            let mut rc = n.n.strong() as isize;
            let a = n.n.addr();
            if let GNode::Obj(_) = &n.n {
                rc -= tracked.contains(&a) as isize + held.contains(&a) as isize;
            }
            if zombie_addrs.contains(&a) {
                rc -= 1;
            }
            g.nodes[i].rc = rc;
        }
        for e in 0..g.edges.len() {
            let j = g.edges[e] as usize;
            g.nodes[j].rc -= 1;
        }
        // gc_scan_roots: whatever a node with outside references reaches
        // is black, the rest white.
        let mut stack: Vec<u32> = (0..g.nodes.len() as u32)
            .filter(|&i| g.nodes[i as usize].rc > 0)
            .collect();
        for &i in &stack {
            g.nodes[i as usize].black = true;
        }
        while let Some(i) = stack.pop() {
            let n = &g.nodes[i as usize];
            for k in n.first..n.first + n.len {
                let j = g.edges[k as usize];
                if !g.nodes[j as usize].black {
                    g.nodes[j as usize].black = true;
                    stack.push(j);
                }
            }
        }
        let _ = roots;

        // gc_collect_roots: drop the live roots, then collect the white
        // nodes from the remaining ones in buffer order.
        for (k, s) in gc::slots().into_iter().enumerate() {
            let idx = FIRST_ROOT + k as u32;
            if let Slot::Root(n) = &s {
                let live = g.find(n.addr()).is_none_or(|i| g.nodes[i as usize].black);
                if live {
                    drop(gc::remove_slot(idx));
                }
            }
        }
        gc::compact();
        let mut count: i64 = 0;
        let mut has_dtor = false;
        let end = gc::first_unused();
        for idx in FIRST_ROOT..end {
            let Some(n) = gc::slot(idx).node().cloned() else {
                continue;
            };
            gc::retag(idx, Slot::Garbage);
            let Some(i) = g.find(n.addr()) else {
                continue;
            };
            if g.nodes[i as usize].black || g.nodes[i as usize].collected {
                continue;
            }
            count += self.gc_collect_white(&mut g, i, &mut has_dtor, &tracked);
        }

        if gc::num_roots() == 0 {
            gc::set_active(false);
            return Ok(Run {
                count: 0,
                destructors: false,
            });
        }
        let end = gc::first_unused();

        let mut destructors = false;
        if has_dtor {
            destructors = true;
            for idx in FIRST_ROOT..end {
                if let Slot::Garbage(n) = gc::slot(idx) {
                    if let Some(v) = n.upgrade() {
                        if self.gc_needs_dtor(&v, &tracked) {
                            gc::retag(idx, Slot::DtorGarbage);
                        }
                    }
                }
            }
            for idx in FIRST_ROOT..end {
                if let Slot::DtorGarbage(n) = gc::slot(idx) {
                    count -= gc_remove_nested(&mut g, &n);
                }
            }
            let t = Instant::now();
            for idx in FIRST_ROOT..end {
                let Slot::DtorGarbage(n) = gc::slot(idx) else {
                    continue;
                };
                gc::retag(idx, Slot::Root);
                let Some(v) = n.upgrade() else {
                    continue;
                };
                let Value::Object(h) = &v else {
                    continue;
                };
                let (cls, id) = {
                    let o = h.borrow();
                    (o.class, o.id)
                };
                // Not yet destructed (another destructor may have released
                // it meanwhile). It stays on the destructor list, marked,
                // until the collection ends (one pass removes them all).
                let listed = gc::flags(id) & gc::TRACKED != 0;
                let r = if listed {
                    gc::set_flag(id, gc::CALLED, true);
                    self.call_destructor(h.clone())
                } else {
                    if self.is_generator_class(cls) && self.gen_has_frame(id) {
                        self.gen_gc_close(id);
                    }
                    Ok(())
                };
                if let Err(e) = r {
                    if !e.is_catchable() {
                        return Err(e);
                    }
                    *pending = Some(match pending.take() {
                        None => e,
                        Some(prev) => {
                            use rhp_builtins::Host;
                            self.chain_thrown(e, prev)
                        }
                    });
                }
                self.collect_released()?;
                if Rc::strong_count(h) - usize::from(listed) == 1 {
                    zombies.push(v);
                }
            }
            self.gc_unlist_called();
            *dtor_ns += t.elapsed().as_nanos() as u64;
        }

        // Destroy the garbage, in buffer order: each object's storage
        // goes and its handle returns to the free list.
        let t = Instant::now();
        let mut holds = Vec::new();
        for idx in FIRST_ROOT..end {
            let Slot::Garbage(n) = gc::slot(idx) else {
                continue;
            };
            if let Some(v) = n.upgrade() {
                match &v {
                    Value::Object(h) => {
                        let (cls, id) = {
                            let o = h.borrow();
                            (o.class, o.id)
                        };
                        if self.is_generator_class(cls) {
                            drop(self.gen_gc_free(id));
                        }
                        rhp_values::gc_free_object(h);
                    }
                    Value::Closure(c) => {
                        let s = std::mem::take(&mut *c.statics.borrow_mut());
                        drop(s);
                    }
                    _ => {}
                }
                holds.push(v);
            }
            drop(gc::remove_slot(idx));
        }
        // Garbage references go with what holds them (php frees them as
        // the arrays and objects holding them are destroyed).
        for i in 0..g.nodes.len() {
            let n = &g.nodes[i];
            if n.black || n.kept || !n.collected {
                continue;
            }
            if let GNode::Ref(w) = &n.n {
                if let Some(r) = w.upgrade() {
                    let old = std::mem::replace(&mut *r.borrow_mut(), Value::Null);
                    drop(old);
                }
            }
        }
        drop(holds);
        drop(g);
        *free_ns += t.elapsed().as_nanos() as u64;
        self.collect_released_now()?;
        gc::count_collected(count.max(0) as u32);
        gc::set_active(false);
        Ok(Run { count, destructors })
    }

    /// gc_collect_white from node `i`: php's depth-first order (the first
    /// white child next, the others on the stack). Returns the count.
    fn gc_collect_white(
        &self,
        g: &mut Graph,
        start: u32,
        has_dtor: &mut bool,
        tracked: &FxHashSet<usize>,
    ) -> i64 {
        let mut count = 0;
        let mut stack = Vec::new();
        let mut cur = start;
        g.nodes[cur as usize].collected = true;
        loop {
            let n = g.nodes[cur as usize].n.clone();
            if !matches!(n, GNode::Ref(_)) {
                count += 1;
                if let Some(v) = n.upgrade() {
                    if !matches!(v, Value::Ref(_)) && gc::slot_of(&v) == 0 {
                        gc::add_garbage(&v);
                    }
                    if self.gc_needs_dtor(&v, tracked) {
                        *has_dtor = true;
                    }
                }
            }
            let mut next = None;
            let (first, len) = {
                let x = &g.nodes[cur as usize];
                (x.first, x.len)
            };
            for k in first..first + len {
                let j = g.edges[k as usize];
                let c = &mut g.nodes[j as usize];
                if !c.black && !c.collected {
                    c.collected = true;
                    if next.is_none() {
                        next = Some(j);
                    } else {
                        stack.push(j);
                    }
                }
            }
            match next.or_else(|| stack.pop()) {
                Some(j) => cur = j,
                None => return count,
            }
        }
    }
}

/// gc_remove_nested_data_from_buffer from destructor garbage `root`: the
/// garbage it reaches leaves the buffer (it is kept for the rerun) and is
/// not counted. Returns how many it took out, the root included.
fn gc_remove_nested(g: &mut Graph, root: &Node) -> i64 {
    let Some(start) = g.find(root.addr()) else {
        return 1;
    };
    let mut count = 0;
    let mut stack = Vec::new();
    let mut cur = start;
    let mut is_root = true;
    loop {
        let proceed = if is_root {
            is_root = false;
            count += 1;
            g.nodes[cur as usize].kept = true;
            true
        } else {
            let n = &g.nodes[cur as usize];
            let in_buffer = !matches!(n.n, GNode::Ref(_))
                && n.n.upgrade().is_some_and(|v| {
                    let idx = gc::slot_of(&v);
                    idx != 0 && matches!(gc::slot(idx), Slot::Garbage(_))
                });
            if in_buffer {
                let v = n.n.upgrade();
                if let Some(v) = &v {
                    let idx = gc::slot_of(v);
                    drop(gc::remove_slot(idx));
                }
                g.nodes[cur as usize].kept = true;
                count += 1;
                true
            } else if matches!(n.n, GNode::Ref(_)) && !n.kept {
                g.nodes[cur as usize].kept = true;
                // A reference: on to its value.
                match g.edges(cur).first() {
                    Some(&j) => {
                        cur = j;
                        continue;
                    }
                    None => false,
                }
            } else {
                false
            }
        };
        if proceed {
            let edges = g.edges(cur).to_vec();
            if let Some((&first, rest)) = edges.split_first() {
                stack.extend_from_slice(rest);
                cur = first;
                continue;
            }
        }
        match stack.pop() {
            Some(j) => cur = j,
            None => return count,
        }
    }
}
