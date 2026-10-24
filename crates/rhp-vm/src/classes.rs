//! OOP class table: the linked `ClassEntry`s + `Vm` methods to install
//! (flatten + link) `ClassProto`s, resolve `ClassRef`s, allocate objects, and
//! look up methods/static props. See `docs/design/oop.md`.
//!
//! Inheritance is flattened at link time (parent merged first, own members
//! override) for O(1), inline-cacheable dispatch.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use indexmap::IndexMap;
use rustc_hash::FxBuildHasher;

use rhp_bytecode::{ClassProto, ClassRef, ConstInitProto, FunctionProto, PropDefaultProto, Reg};
use rhp_values::{
    ClassFlags, ClassId, ClassKind, ObjectData, ObjectHandle, PhpStr, Thrown, Value, Visibility,
};

use crate::{Frame, Vm};

/// A linked class: all tables flattened from the inheritance chain.
pub struct ClassEntry {
    pub name: PhpStr,
    pub kind: ClassKind,
    pub flags: ClassFlags,
    pub parent: Option<ClassId>,
    /// self + all transitive parents + interfaces (the instanceof set).
    pub ancestors: Vec<ClassId>,
    /// Flattened vtable, keyed by lowercase method name.
    pub methods: HashMap<Vec<u8>, ResolvedMethod>,
    /// Flattened instance properties in declaration order.
    pub props: IndexMap<PhpStr, PropEntry>,
    /// Flattened class constants (case-sensitive).
    pub constants: HashMap<Vec<u8>, ClassConst>,
    /// Flattened static-property slots (shared cells; inherited ones alias).
    pub static_props: IndexMap<Vec<u8>, StaticProp>,
    /// Enum case singletons in declaration order (empty for non-enums).
    pub enum_cases: Vec<ObjectHandle>,
    /// Backing type of a backed enum (`None` for pure enums / non-enums).
    pub enum_backing: Option<rhp_bytecode::EnumBacking>,
}

#[derive(Clone)]
pub struct ResolvedMethod {
    pub proto: Rc<FunctionProto>,
    pub defining_class: ClassId,
    pub visibility: Visibility,
    pub is_static: bool,
}

#[derive(Clone)]
pub struct PropEntry {
    pub default: Value,
    pub visibility: Visibility,
    pub declaring_class: ClassId,
}

#[derive(Clone)]
pub struct StaticProp {
    pub cell: Rc<RefCell<Value>>,
    pub visibility: Visibility,
    pub declaring_class: ClassId,
}

#[derive(Clone)]
pub struct ClassConst {
    pub value: Value,
    pub visibility: Visibility,
    pub declaring_class: ClassId,
}

fn lc(bytes: &[u8]) -> Vec<u8> {
    bytes.to_ascii_lowercase()
}

impl Vm {
    /// Link every `ClassProto` into the class table, in dependency order so that
    /// `extends`/`implements`/`use` (which may be forward references) resolve.
    pub(crate) fn install_classes(&mut self, protos: &[ClassProto]) -> Result<(), Thrown> {
        let mut pending: Vec<&ClassProto> = protos.iter().collect();
        loop {
            if pending.is_empty() {
                // All classes linked: now evaluate deferred initializers (class
                // consts / static props / property defaults that referenced
                // other classes, enum cases, etc.) in each one's scope.
                return self.eval_pending_inits();
            }
            let mut progressed = false;
            let mut i = 0;
            while i < pending.len() {
                if self.deps_ready(pending[i]) {
                    let proto = pending.remove(i);
                    self.link_class(proto)?;
                    progressed = true;
                } else {
                    i += 1;
                }
            }
            if !progressed {
                // A remaining class names an unlinked dependency (missing or cyclic).
                let p = pending[0];
                let missing = self.first_missing_dep(p);
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Class \"{}\" not found",
                        String::from_utf8_lossy(&missing.unwrap_or_else(|| p.name.as_bytes().to_vec()))
                    ),
                ));
            }
        }
    }

    fn deps_ready(&self, p: &ClassProto) -> bool {
        let known = |n: &PhpStr| self.class_ids.contains_key(&lc(n.as_bytes()));
        p.parent.as_ref().map_or(true, known)
            && p.interfaces.iter().all(known)
            && p.traits.iter().all(known)
    }

    fn first_missing_dep(&self, p: &ClassProto) -> Option<Vec<u8>> {
        let missing = |n: &PhpStr| {
            if self.class_ids.contains_key(&lc(n.as_bytes())) {
                None
            } else {
                Some(n.as_bytes().to_vec())
            }
        };
        p.parent
            .as_ref()
            .and_then(missing)
            .or_else(|| p.interfaces.iter().find_map(missing))
            .or_else(|| p.traits.iter().find_map(missing))
    }

    fn link_class(&mut self, proto: &ClassProto) -> Result<(), Thrown> {
        let id = self.classes.len() as ClassId;
        let parent = match &proto.parent {
            Some(n) => Some(self.class_ids[&lc(n.as_bytes())]),
            None => None,
        };

        // Methods: parent's table, then traits, then own (override by lc name).
        let mut methods: HashMap<Vec<u8>, ResolvedMethod> =
            parent.map(|p| self.classes[p as usize].methods.clone()).unwrap_or_default();
        for tname in &proto.traits {
            let tid = self.class_ids[&lc(tname.as_bytes())];
            for (k, m) in &self.classes[tid as usize].methods {
                // Trait methods adopt the using class as their defining class.
                methods.insert(k.clone(), ResolvedMethod { defining_class: id, ..m.clone() });
            }
        }
        for m in &proto.methods {
            methods.insert(
                lc(m.name.as_bytes()),
                ResolvedMethod {
                    proto: m.proto.clone(),
                    defining_class: id,
                    visibility: m.visibility,
                    is_static: m.is_static,
                },
            );
        }

        // Instance properties, flattened (override replaces in place).
        let mut props: IndexMap<PhpStr, PropEntry> =
            parent.map(|p| self.classes[p as usize].props.clone()).unwrap_or_default();
        for tname in &proto.traits {
            let tid = self.class_ids[&lc(tname.as_bytes())];
            for (k, pe) in self.classes[tid as usize].props.clone() {
                props.insert(k, PropEntry { declaring_class: id, ..pe });
            }
        }
        for pp in &proto.props {
            let default = match &pp.default {
                PropDefaultProto::Const(v) => v.clone(),
                PropDefaultProto::None => Value::Null,
                PropDefaultProto::Init(thunk) => {
                    self.pending_inits.push(PendingInit {
                        class_id: id,
                        thunk: thunk.clone(),
                        target: PendingTarget::Prop(pp.name.clone()),
                    });
                    Value::Null // placeholder until eval_pending_inits runs
                }
            };
            props.insert(
                pp.name.clone(),
                PropEntry { default, visibility: pp.visibility, declaring_class: id },
            );
        }

        // Static properties (inherited slots alias; redeclared get a fresh cell).
        let mut static_props: IndexMap<Vec<u8>, StaticProp> =
            parent.map(|p| self.classes[p as usize].static_props.clone()).unwrap_or_default();
        for tname in &proto.traits {
            let tid = self.class_ids[&lc(tname.as_bytes())];
            for (k, sp) in self.classes[tid as usize].static_props.clone() {
                // Each using class gets its own cell, seeded from the trait's.
                let v = sp.cell.borrow().clone();
                static_props.insert(
                    k,
                    StaticProp {
                        cell: Rc::new(RefCell::new(v)),
                        visibility: sp.visibility,
                        declaring_class: id,
                    },
                );
            }
        }
        for sp in &proto.static_props {
            let cell = Rc::new(RefCell::new(Value::Null));
            match &sp.init {
                ConstInitProto::Const(v) => *cell.borrow_mut() = v.clone(),
                ConstInitProto::Thunk(thunk) => self.pending_inits.push(PendingInit {
                    class_id: id,
                    thunk: thunk.clone(),
                    target: PendingTarget::Static(cell.clone()),
                }),
            }
            static_props.insert(
                sp.name.as_bytes().to_vec(),
                StaticProp { cell, visibility: sp.visibility, declaring_class: id },
            );
        }

        // Class constants.
        let mut constants: HashMap<Vec<u8>, ClassConst> =
            parent.map(|p| self.classes[p as usize].constants.clone()).unwrap_or_default();
        for tname in &proto.traits {
            let tid = self.class_ids[&lc(tname.as_bytes())];
            for (k, c) in self.classes[tid as usize].constants.clone() {
                constants.insert(k, ClassConst { declaring_class: id, ..c });
            }
        }
        for c in &proto.constants {
            let v = match &c.value {
                ConstInitProto::Const(v) => v.clone(),
                ConstInitProto::Thunk(thunk) => {
                    self.pending_inits.push(PendingInit {
                        class_id: id,
                        thunk: thunk.clone(),
                        target: PendingTarget::Const(c.name.as_bytes().to_vec()),
                    });
                    Value::Null // placeholder until eval_pending_inits runs
                }
            };
            constants.insert(
                c.name.as_bytes().to_vec(),
                ClassConst { value: v, visibility: c.visibility, declaring_class: id },
            );
        }

        // Ancestors = self + parent's + each interface's closure.
        let mut ancestors = vec![id];
        if let Some(p) = parent {
            ancestors.extend(self.classes[p as usize].ancestors.iter().copied());
        }
        for iname in &proto.interfaces {
            let iid = self.class_ids[&lc(iname.as_bytes())];
            ancestors.push(iid);
            ancestors.extend(self.classes[iid as usize].ancestors.iter().copied());
        }
        ancestors.sort_unstable();
        ancestors.dedup();

        self.class_ids.insert(lc(proto.name.as_bytes()), id);
        self.classes.push(ClassEntry {
            name: proto.name.clone(),
            kind: proto.kind,
            flags: proto.flags,
            parent,
            ancestors,
            methods,
            props,
            constants,
            static_props,
            enum_cases: Vec::new(),
            enum_backing: proto.enum_backing,
        });

        // Enum cases: materialize one singleton object per case and expose it as
        // a public class constant, so `E::Case` resolves through `FetchClassConst`.
        if proto.kind == ClassKind::Enum {
            for case in &proto.enum_cases {
                let obj = self.new_enum_case(id, &case.name, case.value.clone());
                self.classes[id as usize].constants.insert(
                    case.name.as_bytes().to_vec(),
                    ClassConst {
                        value: Value::Object(obj.clone()),
                        visibility: Visibility::Public,
                        declaring_class: id,
                    },
                );
                self.classes[id as usize].enum_cases.push(obj);
            }
        }
        Ok(())
    }

    /// Build an enum case's singleton object: an instance of the enum class with
    /// a read-only `name` property (and `value` for a backed enum).
    fn new_enum_case(&mut self, class_id: ClassId, name: &PhpStr, value: Option<Value>) -> ObjectHandle {
        let oid = self.next_object_id;
        self.next_object_id += 1;
        let mut properties: IndexMap<PhpStr, Value, FxBuildHasher> =
            IndexMap::with_hasher(FxBuildHasher);
        properties.insert(PhpStr::from("name"), Value::Str(name.clone()));
        if let Some(v) = value {
            properties.insert(PhpStr::from("value"), v);
        }
        Rc::new(RefCell::new(ObjectData { class: class_id, id: oid, properties }))
    }

    /// Resolve a `ClassRef` (in the context of frame `top`) to a `ClassId`.
    pub(crate) fn resolve_class(&self, top: usize, class: ClassRef) -> Result<ClassId, Thrown> {
        match class {
            ClassRef::Name(nidx) => {
                let proto = self.stack[top].proto.clone();
                self.resolve_class_by_name(proto.names[nidx as usize].as_bytes())
            }
            ClassRef::SelfClass => self.stack[top]
                .defining_class
                .ok_or_else(|| Thrown::error("Error", "Cannot use \"self\" outside class context")),
            ClassRef::ParentClass => {
                let d = self.stack[top]
                    .defining_class
                    .ok_or_else(|| Thrown::error("Error", "Cannot use \"parent\" outside class"))?;
                self.classes[d as usize]
                    .parent
                    .ok_or_else(|| Thrown::error("Error", "Cannot use \"parent\" when no parent"))
            }
            ClassRef::StaticClass => self.stack[top]
                .called_class
                .ok_or_else(|| Thrown::error("Error", "Cannot use \"static\" outside class context")),
            ClassRef::Dynamic(reg) => {
                let v = self.reg_get(top, reg);
                match v {
                    Value::Str(s) => self.resolve_class_by_name(s.as_bytes()),
                    Value::Object(h) => Ok(h.borrow().class),
                    _ => Err(Thrown::type_error("Class name must be a string or object")),
                }
            }
        }
    }

    /// Like [`resolve_class`], but on a miss for a by-name reference it runs the
    /// registered autoloaders and retries (php's autoloading). `self`/`parent`/
    /// `static` and object refs never autoload.
    pub(crate) fn resolve_class_autoload(
        &mut self,
        top: usize,
        class: ClassRef,
    ) -> Result<ClassId, Thrown> {
        match self.resolve_class(top, class) {
            Ok(id) => Ok(id),
            Err(e) => {
                let name: Option<Vec<u8>> = match class {
                    ClassRef::Name(nidx) => {
                        Some(self.stack[top].proto.names[nidx as usize].as_bytes().to_vec())
                    }
                    ClassRef::Dynamic(reg) => match self.reg_get(top, reg) {
                        crate::Value::Str(s) => Some(s.as_bytes().to_vec()),
                        _ => None,
                    },
                    _ => None,
                };
                match name {
                    Some(n) if self.try_autoload(&n)? => self.resolve_class_by_name(&n),
                    _ => Err(e),
                }
            }
        }
    }

    /// Run the registered autoloaders for `name` (php passes the FQCN without a
    /// leading `\`) until the class resolves. Returns whether it became defined.
    pub(crate) fn try_autoload(&mut self, name: &[u8]) -> Result<bool, Thrown> {
        let fqcn = name.strip_prefix(b"\\").unwrap_or(name).to_vec();
        if self.resolve_class_by_name(&fqcn).is_ok() {
            return Ok(true);
        }
        for loader in self.autoloaders.clone() {
            self.call_callable_value(&loader, vec![crate::Value::str(fqcn.clone())])?;
            if self.resolve_class_by_name(&fqcn).is_ok() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn resolve_class_by_name(&self, name: &[u8]) -> Result<ClassId, Thrown> {
        // Allow a leading namespace separator / take the last segment.
        let seg = name.rsplit(|&b| b == b'\\').next().unwrap_or(name);
        self.class_ids.get(&lc(seg)).copied().ok_or_else(|| {
            Thrown::error("Error", format!("Class \"{}\" not found", String::from_utf8_lossy(seg)))
        })
    }

    pub(crate) fn class(&self, id: ClassId) -> &ClassEntry {
        &self.classes[id as usize]
    }

    /// Is a member with `vis` declared in `declaring` accessible from `caller`?
    pub(crate) fn check_access(
        &self,
        caller: Option<ClassId>,
        declaring: ClassId,
        vis: Visibility,
    ) -> bool {
        match vis {
            Visibility::Public => true,
            Visibility::Private => caller == Some(declaring),
            Visibility::Protected => match caller {
                // Accessible within the class hierarchy (either direction).
                Some(c) => {
                    self.classes[c as usize].ancestors.contains(&declaring)
                        || self.classes[declaring as usize].ancestors.contains(&c)
                }
                None => false,
            },
        }
    }

    pub(crate) fn access_err(&self, what: &str, class: ClassId, member: &[u8], vis: Visibility) -> Thrown {
        let v = match vis {
            Visibility::Private => "private",
            Visibility::Protected => "protected",
            Visibility::Public => "public",
        };
        Thrown::error(
            "Error",
            format!(
                "Cannot access {v} {what} {}::{}{}",
                String::from_utf8_lossy(self.classes[class as usize].name.as_bytes()),
                if what == "property" { "$" } else { "" },
                String::from_utf8_lossy(member),
            ),
        )
    }

    pub(crate) fn static_prop_cell(
        &self,
        class_id: ClassId,
        name: &[u8],
    ) -> Result<Rc<RefCell<Value>>, Thrown> {
        self.classes[class_id as usize].static_props.get(name).map(|sp| sp.cell.clone()).ok_or_else(
            || {
                Thrown::error(
                    "Error",
                    format!(
                        "Access to undeclared static property {}::${}",
                        String::from_utf8_lossy(self.classes[class_id as usize].name.as_bytes()),
                        String::from_utf8_lossy(name)
                    ),
                )
            },
        )
    }

    /// Allocate a fresh instance of `class_id`, materializing property defaults.
    pub(crate) fn new_object(&mut self, class_id: ClassId) -> ObjectHandle {
        let id = self.next_object_id;
        self.next_object_id += 1;
        let mut properties: IndexMap<PhpStr, Value, FxBuildHasher> =
            IndexMap::with_hasher(FxBuildHasher);
        for (name, pe) in &self.classes[class_id as usize].props {
            properties.insert(name.clone(), pe.default.clone());
        }
        Rc::new(RefCell::new(ObjectData { class: class_id, id, properties }))
    }

    /// Look up a method (flattened table) by name on `class_id`.
    pub(crate) fn lookup_method(&self, class_id: ClassId, name: &[u8]) -> Option<ResolvedMethod> {
        self.classes[class_id as usize].methods.get(&lc(name)).cloned()
    }

    /// Push a frame for a method call, binding `$this` and the LSB classes.
    pub(crate) fn enter_method(
        &mut self,
        top: usize,
        method: &ResolvedMethod,
        this: Option<ObjectHandle>,
        called_class: ClassId,
        dst: Reg,
        base: Reg,
        argc: u16,
    ) -> Result<(), Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let args: Vec<Value> =
            (0..argc).map(|i| self.stack[top].regs[(base + i) as usize].clone()).collect();
        self.push_method_frame(method, this, called_class, args, dst)
    }

    /// Push a method frame with pre-collected args (for spread / magic dispatch).
    pub(crate) fn push_method_frame(
        &mut self,
        method: &ResolvedMethod,
        this: Option<ObjectHandle>,
        called_class: ClassId,
        args: Vec<Value>,
        dst: Reg,
    ) -> Result<(), Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let callee = method.proto.clone();
        let mut frame = Frame::new(callee.clone(), dst);
        crate::bind_params(&callee, &args, &mut frame.regs)?;
        if let (Some(this_cv), Some(obj)) = (callee.this_cv, this) {
            frame.regs[this_cv as usize] = Value::Object(obj);
        }
        frame.defining_class = Some(method.defining_class);
        frame.called_class = Some(called_class);
        if callee.is_generator {
            let gen = self.make_generator(frame);
            let caller = self.stack.len() - 1;
            if dst != Reg::MAX {
                self.reg_set(caller, dst, gen);
            }
            return Ok(());
        }
        self.stack.push(frame);
        Ok(())
    }

    /// Evaluate deferred initializer thunks (class consts / static props /
    /// property defaults with non-constant expressions), each in its class's
    /// scope, after all classes are linked.
    fn eval_pending_inits(&mut self) -> Result<(), Thrown> {
        let pending = std::mem::take(&mut self.pending_inits);
        for p in pending {
            let v = self.run_proto_sync(
                p.thunk.clone(),
                None,
                Some(p.class_id),
                Some(p.class_id),
                Vec::new(),
            )?;
            match p.target {
                PendingTarget::Static(cell) => *cell.borrow_mut() = v,
                PendingTarget::Const(name) => {
                    if let Some(c) = self.classes[p.class_id as usize].constants.get_mut(&name) {
                        c.value = v;
                    }
                }
                PendingTarget::Prop(name) => {
                    if let Some(pe) = self.classes[p.class_id as usize].props.get_mut(&name) {
                        pe.default = v;
                    }
                }
            }
        }
        Ok(())
    }
}

/// A deferred initializer: a thunk to evaluate in `class_id`'s scope and where
/// to store the result.
pub(crate) struct PendingInit {
    pub class_id: ClassId,
    pub thunk: Rc<FunctionProto>,
    pub target: PendingTarget,
}

pub(crate) enum PendingTarget {
    Static(Rc<RefCell<Value>>),
    Const(Vec<u8>),
    Prop(PhpStr),
}
