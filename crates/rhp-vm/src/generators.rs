//! Generators (Zend/zend_generators.c): suspended frames, the `yield from`
//! delegation tree, destruction and the `Generator` methods.
//!
//! A suspended generator's frame lives in its [`GenState`]; resuming pushes
//! it on the stack and runs it until it yields or ends. `yield from` over a
//! Generator links the two in php's tree: the delegating generator is a
//! child of the one it yields from, values and resumption go to the root
//! (the innermost generator), and the driver loop [`Vm::gen_resume`] moves
//! between nodes without nesting Rust calls per level.

use std::cell::RefCell;
use std::rc::Rc;

use rhp_bytecode::{FunctionProto, Reg};
use rhp_values::{ArrayKey, ObjectData, ObjectHandle, PhpArray, PhpStr, Thrown, Value};

use crate::calls::native_method_count_error;
use crate::errors::{FinallyCont, UnwindResult};
use crate::{deref, Frame, Vm};

thread_local! {
    /// Handles freed while generators exist: a freed generator's state is
    /// dropped at the next [`Vm::drop_freed_generators`].
    static FREED: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    /// This thread's generator table is not empty: frees are noted.
    static LIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Set once any engine made a generator: until then a free notes nothing.
static WATCH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The engine's object free hook: the builtins' per-handle state, and the
/// handle noted for the generator table.
pub(crate) fn object_freed(id: u32) {
    rhp_builtins::object_freed(id);
    if WATCH.load(std::sync::atomic::Ordering::Relaxed) && LIVE.try_with(|l| l.get()) == Ok(true) {
        let _ = FREED.try_with(|f| {
            if let Ok(mut f) = f.try_borrow_mut() {
                f.push(id);
            }
        });
    }
}

/// A new engine: nothing noted.
pub(crate) fn reset_freed() {
    let _ = LIVE.try_with(|l| l.set(false));
    let _ = FREED.try_with(|f| f.borrow_mut().clear());
}

/// Status of a generator.
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum GenStatus {
    NotStarted,
    Suspended,
    Running,
    /// Closed: returned, threw, or destroyed (php's NULL execute_data).
    Finished,
}

/// What a generator frame handed its driver when it stopped running.
pub(crate) enum YieldOut {
    /// `yield`: the key, the value and the register a sent value goes to.
    Value(Value, Value, Option<Reg>),
    /// `yield from` attached it to another generator.
    From,
}

/// How one run of a generator frame ended.
enum Step {
    Yielded,
    YieldedFrom,
    Returned,
    Escaped(Thrown),
}

/// A generator's execution state (php's zend_generator).
pub(crate) struct GenState {
    /// The suspended frame (taken out while running). `None` while running/finished.
    pub(crate) frame: Option<Frame>,
    pub(crate) status: GenStatus,
    /// The current key and value (`Undef` before the first yield).
    current_key: Value,
    current_val: Value,
    /// The returned value (`None`: has not returned).
    retval: Option<Value>,
    /// The largest integer key used so far, plus one.
    auto_key: i64,
    /// Register in the frame that receives the `send()` value on resume.
    send_target: Option<Reg>,
    /// php's ZEND_GENERATOR_AT_FIRST_YIELD: started, and not resumed since
    /// (only then can it be rewound).
    at_first_yield: bool,
    /// ZEND_GENERATOR_FORCED_CLOSE: destroyed while suspended; its finally
    /// blocks run and may not yield.
    forced_close: bool,
    /// ZEND_GENERATOR_DO_INIT: it just attached to a generator by `yield
    /// from`, which is not advanced if it already has a value.
    do_init: bool,
    /// Its destructor ran (IS_OBJ_DESTRUCTOR_CALLED).
    destroyed: bool,
    /// An exception to raise at the suspended point when resumed
    /// (Generator::throw, or one passed on from a delegate).
    pending_throw: Option<Thrown>,
    /// The generator this one yields from (php's `node.parent`), held.
    parent: Option<(u32, ObjectHandle)>,
    /// The generators yielding from this one.
    children: Vec<u32>,
    /// For a leaf of a tree: its root, when known (`node.ptr.root`).
    root: Option<u32>,
    /// For a root: the leaf it was last resumed through (`node.ptr.leaf`).
    leaf: Option<u32>,
    /// The register of the `yield from` result while delegating.
    yf_dst: Reg,
    /// The generator function and its class (Generator::__debugInfo).
    func: (Rc<FunctionProto>, Option<rhp_values::ClassId>),
}

impl Vm {
    /// Wrap a prepared (but not pushed) frame as a `Generator` object; the body
    /// runs lazily under the generator protocol.
    pub(crate) fn make_generator(&mut self, mut frame: Frame) -> Value {
        frame.ret_dst = Reg::MAX;
        let func = (frame.proto.clone(), frame.defining_class);
        let cls = self
            .resolve_class_by_name(b"Generator")
            .expect("Generator prelude class");
        let id = self.alloc_object_id();
        let obj = Rc::new(RefCell::new(ObjectData::new(
            cls,
            id,
            indexmap::IndexMap::with_hasher(rustc_hash::FxBuildHasher),
        )));
        // php's generator holds its Closure until the generator object is
        // freed, also after it finished: the object's payload keeps it.
        if let Some(c) = frame.closure.clone() {
            obj.borrow_mut().set_payload(Rc::new(c));
        }
        WATCH.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = LIVE.try_with(|l| l.set(true));
        // A free of an earlier object with this handle is not this
        // generator's.
        let _ = FREED.try_with(|f| {
            if let Ok(mut f) = f.try_borrow_mut() {
                f.retain(|&x| x != id);
            }
        });
        // Its destructor (zend_generator_dtor_storage) only has work when
        // the function has finally blocks to run.
        if frame
            .proto
            .exception_table
            .iter()
            .any(|r| r.finally.is_some())
        {
            rhp_values::gc::set_flag(id, rhp_values::gc::TRACKED, true);
            self.destructibles.push(obj.clone());
        }
        self.generators.insert(
            id,
            GenState {
                frame: Some(frame),
                status: GenStatus::NotStarted,
                current_key: Value::Undef,
                current_val: Value::Undef,
                retval: None,
                auto_key: 0,
                send_target: None,
                at_first_yield: false,
                forced_close: false,
                do_init: false,
                destroyed: false,
                pending_throw: None,
                parent: None,
                children: Vec::new(),
                root: None,
                leaf: None,
                yf_dst: Reg::MAX,
                func,
            },
        );
        Value::Object(obj)
    }

    fn gen(&mut self, id: u32) -> &mut GenState {
        self.generators.get_mut(&id).expect("generator state")
    }

    /// Not closed (php's `execute_data != NULL`).
    fn gen_alive(&self, id: u32) -> bool {
        self.generators
            .get(&id)
            .is_some_and(|g| g.status != GenStatus::Finished)
    }

    fn gen_parent(&self, id: u32) -> Option<u32> {
        self.generators.get(&id)?.parent.as_ref().map(|p| p.0)
    }

    // -- the delegation tree (zend_generator_node) --

    /// clear_link_to_leaf: a root forgets its leaf (returned).
    fn gen_clear_link_to_leaf(&mut self, id: u32) -> Option<u32> {
        let leaf = self.gen(id).leaf.take()?;
        if let Some(l) = self.generators.get_mut(&leaf) {
            l.root = None;
        }
        Some(leaf)
    }

    /// clear_link_to_root: a leaf forgets its root.
    fn gen_clear_link_to_root(&mut self, id: u32) {
        if let Some(root) = self.gen(id).root.take() {
            if let Some(r) = self.generators.get_mut(&root) {
                r.leaf = None;
            }
        }
    }

    fn gen_remove_child(&mut self, parent: u32, child: u32) {
        if let Some(p) = self.generators.get_mut(&parent) {
            if let Some(i) = p.children.iter().position(|&c| c == child) {
                p.children.remove(i);
            }
        }
    }

    /// zend_generator_yield_from: `id` delegates to `from`.
    fn gen_yield_from(&mut self, id: u32, from: u32, h: ObjectHandle) {
        let leaf = self.gen_clear_link_to_leaf(id);
        if let Some(leaf) = leaf {
            let f = &self.generators[&from];
            if f.parent.is_none() && f.leaf.is_none() {
                self.gen(from).leaf = Some(leaf);
                self.gen(leaf).root = Some(from);
            }
        }
        let g = self.gen(id);
        g.parent = Some((from, h));
        g.do_init = true;
        self.gen(from).children.push(id);
    }

    /// zend_generator_update_root.
    fn gen_update_root(&mut self, id: u32) -> u32 {
        let mut root = self.gen_parent(id).expect("a delegating generator");
        while let Some(p) = self.gen_parent(root) {
            root = p;
        }
        self.gen_clear_link_to_leaf(root);
        self.gen(root).leaf = Some(id);
        self.gen(id).root = Some(root);
        root
    }

    /// get_new_root: the node to run next once `root` closed.
    fn gen_new_root(&self, id: u32, mut root: u32) -> u32 {
        while !self.gen_alive(root) && self.generators[&root].children.len() == 1 {
            root = self.generators[&root].children[0];
        }
        if self.gen_alive(root) {
            return root;
        }
        let mut g = id;
        while let Some(p) = self.gen_parent(g) {
            if !self.gen_alive(p) {
                break;
            }
            g = p;
        }
        g
    }

    /// zend_generator_get_current: the generator that runs when `id` is
    /// resumed (the root of its tree). `exc`: an exception is pending, so
    /// a closed delegate passes on no return value.
    pub(crate) fn gen_current(&mut self, id: u32, exc: bool) -> Result<u32, Thrown> {
        let g = &self.generators[&id];
        if g.parent.is_none() {
            return Ok(id);
        }
        let root = match g.root {
            Some(r) => r,
            None => self.gen_update_root(id),
        };
        if self.gen_alive(root) {
            return Ok(root);
        }
        self.gen_update_current(id, exc)
    }

    /// The root of `id`'s tree, without updating anything.
    fn gen_peek_root(&self, id: u32) -> u32 {
        let mut g = id;
        while let Some(p) = self.gen_parent(g) {
            g = p;
        }
        g
    }

    /// zend_generator_update_current: the root closed; the node below it
    /// on the way to leaf `id` takes over, receiving the return value as
    /// the result of its `yield from` (or ClosedGeneratorException when
    /// the root was aborted).
    fn gen_update_current(&mut self, id: u32, exc: bool) -> Result<u32, Thrown> {
        let old_root = self.generators[&id].root.expect("a known root");
        let new_root = self.gen_new_root(id, old_root);
        self.gen(id).root = Some(new_root);
        self.gen(new_root).leaf = Some(id);
        self.gen(old_root).leaf = None;
        let parent = self.gen_parent(new_root).expect("the closed root");
        self.gen_remove_child(parent, new_root);
        if !exc && !self.generators[&id].destroyed {
            match self.generators[&parent].retval.clone() {
                None => {
                    let e = self.in_gen_context(new_root, id, |vm| {
                        vm.materialize(
                            Thrown::error(
                                "ClosedGeneratorException",
                                "Generator yielded from aborted, no return value available",
                            ),
                            None,
                        )
                    });
                    self.gen(new_root).pending_throw = Some(e);
                    if self.generators[&old_root].status != GenStatus::Running {
                        let p = self.gen(new_root).parent.take();
                        drop(p);
                        self.gen_resume(id)?;
                        return self.gen_current(id, false);
                    }
                }
                Some(rv) => {
                    let v = self.generators[&parent].current_val.clone();
                    let g = self.gen(new_root);
                    g.current_val = v;
                    let dst = g.yf_dst;
                    if let Some(f) = g.frame.as_mut() {
                        if let Some(slot) = f.regs.get_mut(dst as usize) {
                            *slot = rv;
                        }
                    }
                }
            }
        }
        let p = self.gen(new_root).parent.take();
        drop(p);
        Ok(new_root)
    }

    /// Run `f` with generator `id`'s suspended frame on the stack, as php
    /// sets EG(current_execute_data) to it (resumed through leaf `orig`).
    fn in_gen_context<R>(&mut self, id: u32, orig: u32, f: impl FnOnce(&mut Self) -> R) -> R {
        let Some(frame) = self.gen(id).frame.take() else {
            return f(self);
        };
        let at = self.stack.len();
        self.stack.push(frame);
        self.gen_resumes.push((at, id, orig));
        let r = f(self);
        self.gen_resumes.pop();
        let frame = self.stack.pop().expect("the generator frame");
        self.gen(id).frame = Some(frame);
        r
    }

    // -- running --

    /// zend_generator_resume: run the generator `orig` resumes (the root
    /// of its tree) to its next `yield`, moving along the tree as
    /// delegates attach, return or throw.
    pub(crate) fn gen_resume(&mut self, orig: u32) -> Result<(), Thrown> {
        let mut gen = self.gen_current(orig, false)?;
        if !self.gen_alive(gen) {
            return Ok(());
        }
        let mut delegator = orig;
        loop {
            if self.generators[&gen].status == GenStatus::Running {
                return Err(Thrown::error(
                    "Error",
                    "Cannot resume an already running generator",
                ));
            }
            if std::mem::take(&mut self.gen(delegator).do_init)
                && !matches!(self.generators[&gen].current_val, Value::Undef)
            {
                // An already started delegate is not advanced on first
                // resumption.
                return Ok(());
            }
            self.gen(orig).at_first_yield = false;
            match self.gen_run(gen, orig)? {
                Step::Escaped(t) => {
                    if gen == orig {
                        return Err(t);
                    }
                    // Rethrown into the generator that delegated to it.
                    gen = self.gen_current(orig, true)?;
                    if self.generators[&gen].status == GenStatus::Running {
                        return Err(Thrown::error(
                            "Error",
                            "Cannot resume an already running generator",
                        ));
                    }
                    self.gen(gen).pending_throw = Some(t);
                    delegator = orig;
                }
                Step::YieldedFrom => {
                    delegator = gen;
                    gen = self.gen_current(orig, false)?;
                }
                Step::Returned if gen != orig && self.generators[&gen].retval.is_some() => {
                    delegator = orig;
                    gen = self.gen_current(orig, false)?;
                }
                _ => return Ok(()),
            }
        }
    }

    /// Run generator `id`'s frame (resumed through leaf `orig`) until it
    /// yields, delegates, returns or an exception leaves it.
    fn gen_run(&mut self, id: u32, orig: u32) -> Result<Step, Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        rhp_builtins::zend::call_stack::check()?;
        let g = self.gen(id);
        let Some(mut frame) = g.frame.take() else {
            return Ok(Step::Returned);
        };
        let mut throw = g.pending_throw.take();
        g.status = GenStatus::Running;
        frame.capture = true;
        let base = self.stack.len();
        self.stack.push(frame);
        self.gen_stack.push(id);
        self.gen_resumes.push((base, id, orig));
        let saved_yield = self.yield_out.take();
        let result = loop {
            let step = match throw.take() {
                Some(t) => Err(t),
                None => self.run_frames(),
            };
            match step {
                Ok(v) => {
                    let g = self.generators.get_mut(&id).unwrap();
                    match self.yield_out.take() {
                        Some(out) => {
                            let frame = self.stack.pop().unwrap();
                            g.frame = Some(frame);
                            g.status = GenStatus::Suspended;
                            match out {
                                YieldOut::Value(k, val, dst) => {
                                    g.current_key = k;
                                    g.current_val = val;
                                    g.send_target = dst;
                                    break Ok(Step::Yielded);
                                }
                                YieldOut::From => {
                                    g.send_target = None;
                                    break Ok(Step::YieldedFrom);
                                }
                            }
                        }
                        None => {
                            // ZEND_GENERATOR_RETURN (a forced close ends
                            // with no return value).
                            g.status = GenStatus::Finished;
                            if !std::mem::take(&mut self.gen_abort) {
                                g.retval = Some(v);
                            }
                            break Ok(Step::Returned);
                        }
                    }
                }
                Err(thrown) => {
                    if !thrown.is_catchable() {
                        self.gen(id).status = GenStatus::Finished;
                        break Err(thrown);
                    }
                    match self.unwind(thrown, base) {
                        Ok(UnwindResult::Caught) => {}
                        Ok(UnwindResult::Escaped(t)) => {
                            self.gen(id).status = GenStatus::Finished;
                            break Ok(Step::Escaped(t));
                        }
                        Err(e) => {
                            self.gen(id).status = GenStatus::Finished;
                            break Err(e);
                        }
                    }
                }
            }
        };
        self.gen_resumes.pop();
        self.gen_stack.pop();
        self.yield_out = saved_yield;
        if self.generators[&id].status == GenStatus::Finished {
            self.gen_release_current(id);
        }
        result
    }

    /// A closed generator's current key and value: php keeps them until
    /// the object is freed, which rhp notes only later, so they go now
    /// unless a delegating generator still takes the value over.
    fn gen_release_current(&mut self, id: u32) {
        let g = self.gen(id);
        if g.children.is_empty() {
            let kv = (
                std::mem::replace(&mut g.current_key, Value::Null),
                std::mem::replace(&mut g.current_val, Value::Null),
            );
            drop(kv);
        }
    }

    /// zend_generator_ensure_initialized: run a fresh generator to its
    /// first `yield`.
    pub(crate) fn gen_ensure_started(&mut self, id: u32) -> Result<(), Thrown> {
        let g = &self.generators[&id];
        if matches!(g.current_val, Value::Undef)
            && g.status != GenStatus::Finished
            && g.parent.is_none()
        {
            let r = self.gen_resume(id);
            if let Some(g) = self.generators.get_mut(&id) {
                g.at_first_yield = true;
            }
            r?;
        }
        Ok(())
    }

    /// zend_generator_rewind: start the generator; one resumed past its
    /// first `yield` cannot go back.
    pub(crate) fn gen_rewind(&mut self, id: u32) -> Result<(), Thrown> {
        self.gen_ensure_started(id)?;
        if self.generators.get(&id).is_some_and(|g| !g.at_first_yield) {
            return Err(Thrown::error(
                "Exception",
                "Cannot rewind a generator that was already run",
            ));
        }
        Ok(())
    }

    /// zend_generator_get_iterator's first check: a finished generator
    /// cannot be iterated again.
    pub(crate) fn gen_check_open(&self, id: u32) -> Result<(), Thrown> {
        if !self.gen_alive(id) {
            return Err(Thrown::error(
                "Exception",
                "Cannot traverse an already closed generator",
            ));
        }
        Ok(())
    }

    /// The iterator's move_forward: advance generator `id`.
    pub(crate) fn advance_gen(&mut self, id: u32) -> Result<(), Thrown> {
        // The common case: a started generator outside any tree.
        let g = self.gen(id);
        if g.parent.is_none()
            && g.status == GenStatus::Suspended
            && !g.do_init
            && !matches!(g.current_val, Value::Undef)
        {
            g.at_first_yield = false;
            return match self.gen_run(id, id)? {
                Step::Escaped(t) => Err(t),
                Step::YieldedFrom => self.gen_resume(id),
                _ => Ok(()),
            };
        }
        self.gen_ensure_started(id)?;
        self.gen_resume(id)
    }

    /// The iterator's valid / get_current_data / get_current_key: the
    /// current key and value (by reference for a by-reference generator),
    /// or None once it is closed.
    pub(crate) fn gen_fetch(&mut self, id: u32) -> Result<Option<(Value, Value)>, Thrown> {
        let g = &self.generators[&id];
        if g.parent.is_none() && !matches!(g.current_val, Value::Undef) {
            if g.status == GenStatus::Finished {
                return Ok(None);
            }
            return Ok(Some((g.current_key.clone(), g.current_val.clone())));
        }
        self.gen_ensure_started(id)?;
        let root = self.gen_current(id, false)?;
        if !self.gen_alive(id) {
            return Ok(None);
        }
        let g = &self.generators[&root];
        let undef_null = |v: &Value| match v {
            Value::Undef => Value::Null,
            v => v.clone(),
        };
        Ok(Some((
            undef_null(&g.current_key),
            undef_null(&g.current_val),
        )))
    }

    /// The current value or key of a started generator (Generator::current /
    /// key / send): the root's, NULL once closed.
    fn gen_current_of(&mut self, id: u32, key: bool) -> Result<Value, Thrown> {
        let root = self.gen_current(id, false)?;
        if !self.gen_alive(id) {
            return Ok(Value::Null);
        }
        let g = &self.generators[&root];
        Ok(match if key { &g.current_key } else { &g.current_val } {
            Value::Undef => Value::Null,
            v => deref(v),
        })
    }

    /// [`Self::generator_method`] called from frame `top` on `h`: the call
    /// shows in traces as `Generator->name()`, and a generator it resumes
    /// as called from `[internal function]`.
    /// Takes `h` over: the call holds the object (php's This with
    /// ZEND_CALL_RELEASE_THIS), which a suspended fiber's collection sees
    /// through the native call.
    pub(crate) fn generator_method_call(
        &mut self,
        top: usize,
        h: ObjectHandle,
        name_lc: &[u8],
        args: &[Value],
    ) -> Result<Value, Thrown> {
        let name = generator_method_name(name_lc).expect("a Generator method");
        let id = h.borrow().id;
        let named: &[(PhpStr, Value)] = &[];
        self.native_calls.push(crate::calls::NativeCall {
            name,
            args: args as *const [Value],
            shown_str: 0,
            caller: top,
            frameless_icall: false,
            named: named as *const [(PhpStr, Value)],
            method_of: Some(("Generator", h)),
        });
        let r = self
            .generator_method(id, name_lc, args)
            .expect("a Generator method");
        // An engine error is created with the call still on the trace.
        let r = match r {
            Err(e @ Thrown::Error { .. }) => Err(self.materialize(e, None)),
            r => r,
        };
        drop(self.native_calls.pop());
        r
    }

    /// Dispatch a built-in `Generator` method. Returns `None` if `name` is not a
    /// generator method (so the caller falls through to normal dispatch).
    pub(crate) fn generator_method(
        &mut self,
        id: u32,
        name_lc: &[u8],
        args: &[Value],
    ) -> Option<Result<Value, Thrown>> {
        if matches!(
            name_lc,
            b"current"
                | b"key"
                | b"valid"
                | b"next"
                | b"send"
                | b"rewind"
                | b"getreturn"
                | b"throw"
                | b"__debuginfo"
        ) {
            if let Some(e) = native_method_count_error("Generator", name_lc, args.len()) {
                return Some(Err(e));
            }
        }
        Some(match name_lc {
            b"current" => (|| {
                self.gen_ensure_started(id)?;
                self.gen_current_of(id, false)
            })(),
            b"key" => (|| {
                self.gen_ensure_started(id)?;
                self.gen_current_of(id, true)
            })(),
            b"valid" => (|| {
                self.gen_ensure_started(id)?;
                self.gen_current(id, false)?;
                Ok(Value::Bool(self.gen_alive(id)))
            })(),
            b"next" => (|| {
                self.advance_gen(id)?;
                Ok(Value::Null)
            })(),
            b"send" => (|| {
                let v = args.first().cloned().unwrap_or(Value::Null);
                self.gen_ensure_started(id)?;
                if !self.gen_alive(id) {
                    return Ok(Value::Null);
                }
                let root = self.gen_current(id, false)?;
                let g = self.gen(root);
                if let (Some(dst), GenStatus::Suspended, Some(f)) =
                    (g.send_target, g.status, g.frame.as_mut())
                {
                    f.regs[dst as usize] = v;
                }
                self.gen_resume(id)?;
                self.gen_current_of(id, false)
            })(),
            b"rewind" => (|| {
                self.gen_rewind(id)?;
                Ok(Value::Null)
            })(),
            b"getreturn" => (|| {
                self.gen_ensure_started(id)?;
                match &self.generators[&id].retval {
                    Some(v) => Ok(v.clone()),
                    None => Err(Thrown::error(
                        "Exception",
                        "Cannot get return value of a generator that hasn't returned",
                    )),
                }
            })(),
            b"throw" => (|| {
                let e = match args.first().map(deref) {
                    Some(Value::Object(e))
                        if self.class_implements(e.borrow().class, b"Throwable") =>
                    {
                        e
                    }
                    other => {
                        let v = other.unwrap_or(Value::Null);
                        let given = rhp_builtins::zval_type_name(self, &v);
                        return Err(Thrown::type_error(format!(
                            "Generator::throw(): Argument #1 ($exception) must be of type Throwable, {given} given"
                        )));
                    }
                };
                self.gen_ensure_started(id)?;
                if !self.gen_alive(id) {
                    // Closed: thrown in the caller's context.
                    return Err(Thrown::Object(e));
                }
                let root = self.gen_current(id, false)?;
                // zend_generator_throw_exception
                if self.generators[&root].status == GenStatus::Running {
                    return Err(Thrown::error(
                        "Error",
                        "Cannot resume an already running generator",
                    ));
                }
                self.gen(root).pending_throw = Some(Thrown::Object(e));
                self.gen_resume(id)?;
                self.gen_current_of(id, false)
            })(),
            b"__debuginfo" => Ok(Value::Array(self.generator_debug_info(id))),
            _ => return None,
        })
    }

    /// Generator::__debugInfo: `["function" => name]`, `Class::name` for
    /// a function with a class scope.
    pub(crate) fn generator_debug_info(&self, id: u32) -> PhpArray {
        let mut a = PhpArray::new();
        if let Some(g) = self.generators.get(&id) {
            let (proto, class) = &g.func;
            let name = match class {
                Some(c) => {
                    let mut n = self.class(*c).name.as_bytes().to_vec();
                    n.extend_from_slice(b"::");
                    n.extend_from_slice(proto.name.as_bytes());
                    PhpStr::from_bytes(n)
                }
                None => proto.name.clone(),
            };
            a.insert(ArrayKey::Str(PhpStr::from("function")), Value::Str(name));
        }
        a
    }

    /// The generator running in the current frame.
    fn running_gen(&self) -> u32 {
        *self.gen_stack.last().expect("yield outside a generator")
    }

    /// `Yield`: hand (key, value, resume register) to the generator driver in
    /// `yield_out`; the caller then suspends the frame.
    pub(crate) fn exec_yield(
        &mut self,
        top: usize,
        proto: &FunctionProto,
        dst: Reg,
        key: Reg,
        value: Reg,
    ) -> Result<(), Thrown> {
        let id = self.running_gen();
        if self.generators[&id].forced_close {
            return Err(Thrown::error(
                "Error",
                "Cannot yield from finally in a force-closed generator",
            ));
        }
        // A by-reference generator (`function &gen()`) yields a
        // variable as a reference to it (php's ZEND_YIELD with
        // ZEND_ACC_RETURN_REFERENCE), so `foreach (gen() as &$v)`
        // writes reach it; any other value is yielded as is.
        let val = if proto.returns_ref && value < proto.num_cvs {
            Value::Ref(self.promote(top, value))
        } else if proto.returns_ref {
            // A reference made by MakeRef/MakeRefLval, or a plain
            // value (php: "Only variable references…" notice).
            let v = self.stack[top].regs[value as usize].clone();
            if !matches!(v, Value::Ref(_)) {
                self.diag(
                    rhp_builtins::E_NOTICE,
                    "Only variable references should be yielded by reference",
                )?;
            }
            v
        } else {
            self.reg_r(top, value)?
        };
        let key_val = if key == Reg::MAX {
            let g = self.gen(id);
            let k = Value::Int(g.auto_key);
            g.auto_key += 1;
            k
        } else {
            let kv = self.reg_r(top, key)?;
            if let Value::Int(n) = &kv {
                let g = self.gen(id);
                if *n >= g.auto_key {
                    g.auto_key = n.saturating_add(1);
                }
            }
            kv
        };
        // The result is NULL unless a value is sent.
        self.stack[top].regs[dst as usize] = Value::Null;
        self.yield_out = Some(YieldOut::Value(key_val, val, Some(dst)));
        Ok(())
    }

    /// `YieldValues`: a `yield from` over an array or a non-generator
    /// Traversable yields an element (its own key; the generator's integer
    /// keys and send target are left alone).
    pub(crate) fn exec_yield_values(
        &mut self,
        top: usize,
        key: Reg,
        value: Reg,
    ) -> Result<(), Thrown> {
        let k = self.reg_r(top, key)?;
        let v = self.reg_r(top, value)?;
        self.yield_out = Some(YieldOut::Value(k, v, None));
        Ok(())
    }

    /// ZEND_YIELD_FROM. Over a Generator: attach to it (true: the frame
    /// suspends, resuming at `end`), or take the return value of one that
    /// already returned. Over an array or a Traversable: `dst` is NULL and
    /// the loop after this instruction yields its elements.
    pub(crate) fn exec_yield_from(
        &mut self,
        top: usize,
        dst: Reg,
        src: Reg,
        end: u32,
    ) -> Result<bool, Thrown> {
        let id = self.running_gen();
        if self.generators[&id].forced_close {
            return Err(Thrown::error(
                "Error",
                "Cannot use \"yield from\" in a force-closed generator",
            ));
        }
        let v = self.reg_r(top, src)?;
        match &v {
            Value::Array(_) => {}
            Value::Object(h) => {
                let oid = h.borrow().id;
                if self.generators.contains_key(&oid) {
                    if !self.gen_alive(oid) {
                        return Err(Thrown::error(
                            "Error",
                            "Generator passed to yield from was aborted without proper return and is unable to continue",
                        ));
                    }
                    if let Some(rv) = self.generators[&oid].retval.clone() {
                        self.reg_set(top, dst, rv);
                        self.stack[top].pc = end as usize;
                        return Ok(false);
                    }
                    if self.gen_current(oid, false)? == id {
                        return Err(Thrown::error(
                            "Error",
                            "Impossible to yield from the Generator being currently run",
                        ));
                    }
                    self.gen_yield_from(id, oid, h.clone());
                    self.gen(id).yf_dst = dst;
                    self.reg_set(top, dst, Value::Null);
                    self.stack[top].pc = end as usize;
                    self.yield_out = Some(YieldOut::From);
                    return Ok(true);
                }
                let cls = h.borrow().class;
                if !self.class_implements(cls, b"Traversable") {
                    return Err(Thrown::error(
                        "Error",
                        "Can use \"yield from\" only with arrays and Traversables",
                    ));
                }
            }
            _ => {
                return Err(Thrown::error(
                    "Error",
                    "Can use \"yield from\" only with arrays and Traversables",
                ))
            }
        }
        self.reg_set(top, dst, Value::Null);
        Ok(false)
    }

    /// `FinallyEnd` of a block a destroyed generator runs (the fast_call
    /// with neither return nor exception): the next enclosing finally runs
    /// too, an exception a running finally kept goes on, and with none
    /// left the generator closes without a return value.
    pub(crate) fn finally_forced_close(
        &mut self,
        top: usize,
        proto: &FunctionProto,
        pc: u32,
    ) -> Result<Option<Value>, Thrown> {
        for r in &proto.exception_table {
            let Some(f) = r.finally else {
                continue;
            };
            if pc >= r.start && pc < r.end {
                self.push_pending(top, f, FinallyCont::ForcedClose);
                self.stack[top].pc = f as usize;
                return Ok(None);
            }
            if pc >= f && pc < r.finally_end {
                let pend = &mut self.stack[top].pending;
                if let Some(i) = pend.iter().position(|(k, _)| *k == f) {
                    if let (_, FinallyCont::Rethrow(t)) = pend.remove(i) {
                        return Err(t);
                    }
                }
            }
        }
        self.gen_abort = true;
        Ok(self.do_return(Value::Null))
    }

    /// zend_generator_dtor_storage: a generator object is destroyed. It
    /// leaves its `yield from` tree; suspended inside a try with a finally,
    /// that finally (and the ones around it) runs, force-closed.
    pub(crate) fn gen_dtor(&mut self, id: u32) -> Result<(), Thrown> {
        if !self.generators.contains_key(&id) {
            return Ok(());
        }
        // Running in a fiber: its finally blocks run when the fiber is
        // destroyed, and may not yield.
        let cur = self.gen_peek_root(id);
        if self.generators[&cur].status == GenStatus::Running {
            self.gen(id).forced_close = true;
            return Ok(());
        }
        self.gen(id).destroyed = true;
        let mut released = Ok(());
        if let Some((pid, ph)) = self.gen(id).parent.take() {
            self.gen_remove_child(pid, id);
            self.gen_clear_link_to_root(id);
            drop(ph);
            if !self.destructibles.is_empty() {
                released = self.collect_released_now();
            }
        } else {
            self.gen_clear_link_to_leaf(id);
        }
        let g = &self.generators[&id];
        if g.status != GenStatus::Suspended {
            self.gen_close(id);
            return released;
        }
        let Some(frame) = g.frame.as_ref() else {
            return released;
        };
        let proto = frame.proto.clone();
        let op = (frame.pc as u32).saturating_sub(1);
        for r in &proto.exception_table {
            let Some(f) = r.finally else {
                continue;
            };
            if op >= r.start && op < r.end {
                // Go to the finally block, freeing what the try left
                // live (zend_generator_cleanup_unfinished_execution).
                let mut frame = self.gen(id).frame.take().expect("suspended frame");
                self.forget_unfinished_news(&frame);
                frame.pending.retain(|(k, _)| *k != f);
                frame.pending.push((f, FinallyCont::ForcedClose));
                frame.pc = f as usize;
                let at = self.stack.len();
                self.stack.push(frame);
                let freed = self.free_try_temps(at, r.temps);
                let frame = self.stack.pop().expect("the generator frame");
                let g = self.gen(id);
                g.frame = Some(frame);
                g.forced_close = true;
                if freed && !self.destructibles.is_empty() {
                    released = released.and(self.collect_released_now());
                }
                released?;
                return self.gen_resume(id);
            }
            if op >= f && op < r.finally_end {
                // A running finally block: what it was finishing goes.
                if let Some(fr) = self.gen(id).frame.as_mut() {
                    fr.pending.retain(|(k, _)| *k != f);
                }
            }
        }
        self.gen_close(id);
        released
    }

    /// zend_generator_close: the frame goes (its variables are released).
    fn gen_close(&mut self, id: u32) {
        let g = self.gen(id);
        g.status = GenStatus::Finished;
        let frame = g.frame.take();
        self.gen_release_current(id);
        if let Some(f) = &frame {
            self.forget_unfinished_news(f);
        }
        drop(frame);
    }
}

impl Vm {
    /// Drop the state of generators whose object was freed (php frees the
    /// frame with the generator; its variables are released then).
    pub(crate) fn drop_freed_generators(&mut self) {
        let ids = FREED.with(|f| {
            let mut f = f.borrow_mut();
            if f.is_empty() {
                Vec::new()
            } else {
                std::mem::take(&mut *f)
            }
        });
        for id in ids {
            if let Some(g) = self.generators.get(&id) {
                if g.status != GenStatus::Running {
                    if let Some(g) = self.gen_remove(id) {
                        if let Some(f) = &g.frame {
                            self.forget_unfinished_news(f);
                        }
                        drop(g);
                    }
                }
            }
        }
        self.gen_table_changed();
    }

    /// Take generator `id`'s state out of the table, unlinking it from its
    /// tree (no dangling handle may stay: handles are reused).
    pub(crate) fn gen_remove(&mut self, id: u32) -> Option<GenState> {
        let mut g = self.generators.remove(&id)?;
        if let Some(root) = g.root.take() {
            if let Some(r) = self.generators.get_mut(&root) {
                r.leaf = None;
            }
        }
        if let Some(leaf) = g.leaf.take() {
            if let Some(l) = self.generators.get_mut(&leaf) {
                l.root = None;
            }
        }
        if let Some((pid, _)) = &g.parent {
            self.gen_remove_child(*pid, id);
        }
        for c in std::mem::take(&mut g.children) {
            if let Some(c) = self.generators.get_mut(&c) {
                c.parent = None;
            }
        }
        Some(g)
    }

    /// After states left the generator table: with none left, frees are
    /// no longer noted.
    pub(crate) fn gen_table_changed(&mut self) {
        if self.generators.is_empty() {
            reset_freed();
        }
    }

    /// A suspended frame is destroyed: objects whose `new` had not called
    /// the constructor yet are never destructed (zend_cleanup_unfinished_calls).
    fn forget_unfinished_news(&mut self, f: &Frame) {
        let pc = f.pc as u32;
        for &(start, end, dst) in &f.proto.new_ranges {
            if pc > start && pc < end {
                if let Some(Value::Object(h)) = f.regs.get(dst as usize) {
                    self.destructibles.retain(|d| !Rc::ptr_eq(d, h));
                }
            }
        }
    }

    /// The values generator `id`'s state holds (zend_generator_get_gc):
    /// the suspended frame's registers, the current and returned values
    /// and the generator it yields from.
    pub(crate) fn gen_gc_children(&self, id: u32, out: &mut Vec<Value>) {
        let Some(g) = self.generators.get(&id) else {
            return;
        };
        if let Some(f) = &g.frame {
            f.gc_children(out);
        }
        for v in [&g.current_key, &g.current_val] {
            if !matches!(v, Value::Undef) {
                out.push(v.clone());
            }
        }
        if let Some(v) = &g.retval {
            out.push(v.clone());
        }
        if let Some((_, h)) = &g.parent {
            out.push(Value::Object(h.clone()));
        }
    }

    /// Whether generator `id` still has a frame to destroy (php's
    /// generator `dtor_obj` has work to do).
    pub(crate) fn gen_has_frame(&self, id: u32) -> bool {
        self.generators.get(&id).is_some_and(|g| g.frame.is_some())
    }

    /// The cycle collector destroys generator `id`: its frame and values
    /// go.
    pub(crate) fn gen_gc_close(&mut self, id: u32) {
        let Some(g) = self.generators.get_mut(&id) else {
            return;
        };
        if g.status == GenStatus::Running {
            return;
        }
        let parent = g.parent.take();
        if let Some((pid, _)) = &parent {
            self.gen_remove_child(*pid, id);
            self.gen_clear_link_to_root(id);
        }
        self.gen_close(id);
        let g = self.gen(id);
        let cur = (
            std::mem::replace(&mut g.current_key, Value::Null),
            std::mem::replace(&mut g.current_val, Value::Null),
        );
        drop(cur);
        drop(parent);
    }

    /// The generator object of handle `id` is being freed by the cycle
    /// collector: its state goes with it.
    pub(crate) fn gen_gc_free(&mut self, id: u32) -> Option<GenState> {
        let g = self.generators.get(&id)?;
        if g.status == GenStatus::Running {
            return None;
        }
        let g = self.gen_remove(id)?;
        self.gen_table_changed();
        if let Some(f) = &g.frame {
            self.forget_unfinished_news(f);
        }
        Some(g)
    }

    /// The trace frames of the generators between `root` (running) and
    /// the leaf `orig` it runs for: each one's function, called from the
    /// next one's suspended `yield from` (zend_generator_check_placeholder_frame).
    /// Innermost first, ending with `orig`.
    pub(crate) fn gen_chain(&self, root: u32, orig: u32) -> Vec<&Frame> {
        let mut chain = Vec::new();
        let mut g = orig;
        while g != root {
            match self.generators.get(&g) {
                Some(s) => {
                    if let Some(f) = &s.frame {
                        chain.push(f);
                    }
                    match &s.parent {
                        Some((p, _)) => g = *p,
                        None => break,
                    }
                }
                None => break,
            }
        }
        chain.reverse();
        chain
    }
}

impl Frame {
    /// The values a suspended frame holds.
    pub(crate) fn gc_children(&self, out: &mut Vec<Value>) {
        out.extend(self.regs.iter().map(Value::copy_slot));
        out.extend(self.extra_args.iter().cloned());
        out.extend(self.dyn_vars.values().map(|r| Value::Ref(r.clone())));
        if let Some(c) = &self.closure {
            out.push(Value::Closure(c.clone()));
        }
    }
}

/// The `Generator` method `name_lc` names (as a trace shows it).
pub(crate) fn generator_method_name(name_lc: &[u8]) -> Option<&'static str> {
    Some(match name_lc {
        b"current" => "current",
        b"key" => "key",
        b"valid" => "valid",
        b"next" => "next",
        b"send" => "send",
        b"rewind" => "rewind",
        b"getreturn" => "getReturn",
        b"throw" => "throw",
        b"__debuginfo" => "__debugInfo",
        _ => return None,
    })
}
