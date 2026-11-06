//! What the engine does with `#[\Deprecated]` and `#[\NoDiscard]` at run
//! time (zend_execute.c's zend_deprecated_function, zend_nodiscard_function,
//! zend_deprecated_class_constant, zend_deprecated_constant and
//! zend_use_of_deprecated_trait), and `#[\SensitiveParameter]` in traces.

use std::rc::Rc;

use rhp_bytecode::{AttributeProto, ConstInitProto, FunctionProto, ProtoFlags};
use rhp_values::{ClassId, ObjectData, PhpStr, Thrown, Value};

use crate::{deref, Frame, UnitFn, UnitId, Vm};

/// An attributed global constant's attributes (`DeclareAttrConst`).
pub(crate) struct ConstAttrs {
    pub attrs: Rc<Vec<AttributeProto>>,
    pub unit: UnitId,
}

fn find<'a>(attrs: &'a [AttributeProto], lc: &[u8]) -> Option<&'a AttributeProto> {
    attrs
        .iter()
        .find(|a| a.name.as_bytes().eq_ignore_ascii_case(lc))
}

impl Vm {
    /// get_deprecation_suffix_from_attribute /
    /// get_nodiscard_suffix_from_attribute: build the attribute object (its
    /// constructor checks the arguments) and render
    /// `" since <since>, <message>"`. `Err` when building it threw.
    pub(crate) fn attribute_suffix(
        &mut self,
        attrs: &[AttributeProto],
        class: &str,
        scope: Option<ClassId>,
        unit: UnitId,
    ) -> Result<Vec<u8>, Thrown> {
        let Some(a) = find(attrs, class.as_bytes()) else {
            return Ok(Vec::new());
        };
        let args = match &a.args {
            None => return Ok(Vec::new()),
            Some(ConstInitProto::Const(v)) => v.clone(),
            Some(ConstInitProto::Thunk(proto)) => {
                let f = UnitFn {
                    proto: proto.clone(),
                    unit,
                };
                self.run_proto_sync(f, None, scope, scope, Vec::new(), None)?
            }
        };
        if matches!(&args, Value::Array(a) if a.is_empty()) {
            return Ok(Vec::new());
        }
        let obj = self.new_attribute_object(class.as_bytes(), &args)?;
        let read = |k: &str| -> Vec<u8> {
            let b = obj.borrow();
            match b.properties.get(&PhpStr::from(k)) {
                Some(Value::Str(s)) => s.as_bytes().to_vec(),
                _ => Vec::new(),
            }
        };
        let message = read("message");
        let since = if class == "Deprecated" {
            read("since")
        } else {
            Vec::new()
        };
        let mut out = Vec::new();
        if !since.is_empty() {
            out.extend_from_slice(b" since ");
            out.extend_from_slice(&since);
        }
        if !message.is_empty() {
            out.extend_from_slice(b", ");
            out.extend_from_slice(&message);
        }
        Ok(out)
    }

    /// object_init_with_constructor for an attribute class: `new C(...$args)`
    /// with the argument array's string keys as named arguments, checked in
    /// the calling frame's `strict_types` mode.
    pub(crate) fn new_attribute_object(
        &mut self,
        class: &[u8],
        args: &Value,
    ) -> Result<rhp_values::ObjectHandle, Thrown> {
        let cid = self.resolve_class_by_name(class)?;
        self.update_class_constants(cid)?;
        let obj = self.new_object(cid);
        if let Some(m) = self.lookup_method(cid, b"__construct") {
            let (argv, named) = self.split_call_args(args)?;
            let argv = self.place_named_args(&m.proto, Some(m.defining_class), argv, named)?;
            let strict = self.caller_strict();
            self.call_sync_mode(&m, Some(obj.clone()), cid, argv, Some(strict))?;
        }
        Ok(obj)
    }

    /// The checks ZEND_DO_FCALL makes before entering a `#[\Deprecated]` or
    /// `#[\NoDiscard]` function: run by `bind_frame` for a proto with either
    /// flag.
    #[cold]
    pub(crate) fn call_attr_checks(
        &mut self,
        frame: &Frame,
        args_first: Option<&Value>,
    ) -> Result<(), Thrown> {
        let proto = frame.proto.clone();
        let deprecated = proto.flags.contains(ProtoFlags::DEPRECATED);
        let nodiscard =
            proto.flags.contains(ProtoFlags::NODISCARD) && self.caller_discards();
        if !deprecated && !nodiscard {
            return Ok(());
        }
        let scope = frame.defining_class;
        let name = self.callee_display_name(&proto, scope, args_first);
        if deprecated {
            let suffix =
                self.attribute_suffix(&proto.meta.attributes, "Deprecated", scope, frame.unit)?;
            let msg = match &name {
                (Some(c), f) => cat(format!("Method {c}::{f}() is deprecated"), &suffix),
                (None, f) => cat(format!("Function {f}() is deprecated"), &suffix),
            };
            let level = if proto.is_internal() {
                rhp_builtins::E_DEPRECATED
            } else {
                rhp_builtins::E_USER_DEPRECATED
            };
            self.diag(level, &msg)?;
        }
        if nodiscard {
            let suffix =
                self.attribute_suffix(&proto.meta.attributes, "NoDiscard", scope, frame.unit)?;
            let msg = match &name {
                (Some(c), f) => cat(format!("The return value of method {c}::{f}() should either be used or intentionally ignored by casting it as (void)"), &suffix),
                (None, f) => cat(format!("The return value of function {f}() should either be used or intentionally ignored by casting it as (void)"), &suffix),
            };
            let level = if proto.is_internal() {
                rhp_builtins::E_WARNING
            } else {
                rhp_builtins::E_USER_WARNING
            };
            self.diag(level, &msg)?;
        }
        Ok(())
    }

    /// The calling user frame is at a call whose result it frees.
    fn caller_discards(&self) -> bool {
        let Some(f) = self.stack.last() else {
            return false;
        };
        if f.proto.is_internal() || f.pc == 0 {
            return false;
        }
        let pc = f.pc - 1;
        if f.proto
            .discard_calls
            .binary_search(&(pc as rhp_bytecode::Pc))
            .is_err()
        {
            return false;
        }
        // Not a callback of a builtin that call runs, except
        // `call_user_func` compiled as a user call (ZEND_INIT_USER_CALL).
        match f.proto.code[pc] {
            rhp_bytecode::Instr::Call { flags, .. }
                if flags.contains(rhp_bytecode::CallFlags::USER_CALL) =>
            {
                true
            }
            _ => !self.native_active_from(self.stack.len() - 1),
        }
    }

    /// The class (for a method) and function name php prints for a callee.
    /// A `__call` / `__callStatic` frame stands for the trampoline of the
    /// method it was called as.
    fn callee_display_name(
        &self,
        proto: &FunctionProto,
        scope: Option<ClassId>,
        first: Option<&Value>,
    ) -> (Option<String>, String) {
        let class =
            scope.map(|c| String::from_utf8_lossy(self.class(c).name.as_bytes()).into_owned());
        let mut fname = String::from_utf8_lossy(proto.name.as_bytes()).into_owned();
        if class.is_some()
            && (fname.eq_ignore_ascii_case("__call") || fname.eq_ignore_ascii_case("__callStatic"))
        {
            if let Some(Value::Str(s)) = first.map(deref) {
                fname = String::from_utf8_lossy(s.as_bytes()).into_owned();
            }
        }
        (class, fname)
    }

    /// zend_deprecated_class_constant for constant `name` of `class`, under
    /// php's recursion guard.
    #[cold]
    pub(crate) fn class_const_deprecation(
        &mut self,
        class: ClassId,
        name: &[u8],
        d: &crate::classes::DeprecatedConst,
    ) -> Result<(), Thrown> {
        let mut key = format!("{class}::").into_bytes();
        key.extend_from_slice(name);
        if self.const_guard.iter().any(|k| *k == key) {
            return Ok(());
        }
        self.const_guard.push(key);
        let suffix = self.attribute_suffix(&d.attrs, "Deprecated", Some(class), d.unit);
        self.const_guard.pop();
        let suffix = suffix?;
        let is_case = d.is_case;
        let msg = cat(
            format!(
                "{} {}::{} is deprecated",
                if is_case { "Enum case" } else { "Constant" },
                String::from_utf8_lossy(self.class(class).name.as_bytes()),
                String::from_utf8_lossy(name)
            ),
            &suffix,
        );
        self.diag(rhp_builtins::E_USER_DEPRECATED, &msg)
    }

    /// zend_deprecated_constant for a `#[\Deprecated]` global constant, with
    /// php's recursion guard (its message may name the constant itself).
    #[cold]
    pub(crate) fn deprecated_user_constant(&mut self, key: &[u8]) -> Result<(), Thrown> {
        let Some(ca) = self.const_attrs.get(key) else {
            return Ok(());
        };
        if find(&ca.attrs, b"deprecated").is_none() || self.const_guard.iter().any(|k| k == key) {
            return Ok(());
        }
        let (attrs, unit) = (ca.attrs.clone(), ca.unit);
        self.const_guard.push(key.to_vec());
        let suffix = self.attribute_suffix(&attrs, "Deprecated", None, unit);
        self.const_guard.pop();
        let msg = cat(
            format!("Constant {} is deprecated", String::from_utf8_lossy(key)),
            &suffix?,
        );
        self.diag(rhp_builtins::E_USER_DEPRECATED, &msg)
    }

    /// zend_use_of_deprecated_trait, when `class` uses the trait `t`.
    pub(crate) fn deprecated_trait_use(&mut self, t: ClassId, class: &[u8]) -> Result<(), Thrown> {
        let e = &self.classes[t as usize];
        if find(&e.proto.meta.attributes, b"deprecated").is_none() {
            return Ok(());
        }
        let attrs = e.proto.meta.attributes.clone();
        let unit = e.unit;
        let suffix = self.attribute_suffix(&attrs, "Deprecated", Some(t), unit)?;
        let msg = cat(
            format!(
                "Trait {} used by {} is deprecated",
                String::from_utf8_lossy(self.class(t).name.as_bytes()),
                String::from_utf8_lossy(class)
            ),
            &suffix,
        );
        self.diag(rhp_builtins::E_USER_DEPRECATED, &msg)
    }

    /// A trace argument of a `#[\SensitiveParameter]` parameter: a
    /// SensitiveParameterValue holding it (debug_backtrace_get_args).
    pub(crate) fn sensitive_value(&self, v: Value) -> Value {
        let Some(&cid) = self.class_ids.get(&b"sensitiveparametervalue"[..]) else {
            return v;
        };
        let mut props = indexmap::IndexMap::with_hasher(rustc_hash::FxBuildHasher);
        let name = PhpStr::from("value");
        let key = self
            .class(cid)
            .prop_visible
            .get(&name)
            .cloned()
            .unwrap_or(name);
        props.insert(key, v);
        let id = rhp_values::alloc_object_id();
        Value::Object(Rc::new(std::cell::RefCell::new(ObjectData::new(
            cid, id, props,
        ))))
    }
}

/// Whether trace argument `i` of `proto` is redacted: parameter `i`, or
/// the variadic parameter for the arguments it collects.
pub(crate) fn param_is_sensitive(proto: &FunctionProto, i: usize) -> bool {
    let p = match proto.params.get(i) {
        Some(p) => p,
        None => match proto.params.last() {
            Some(p) if p.variadic => p,
            _ => return false,
        },
    };
    find(&p.attributes, b"sensitiveparameter").is_some()
}

/// `prefix` + `suffix` (php's messages are byte strings).
fn cat(prefix: String, suffix: &[u8]) -> Vec<u8> {
    let mut m = prefix.into_bytes();
    m.extend_from_slice(suffix);
    m
}
