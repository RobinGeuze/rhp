//! zpp for builtin calls: the checks and coercions php's
//! `zend_parse_parameters` applies to a builtin's scalar and array
//! parameters, driven by the stub signatures.

use rhp_builtins as builtins;
use rhp_builtins::object::CastTarget;
use rhp_values::{conv, ArrayKey, Thrown, Value};

use crate::types::type_lc;
use crate::{deref, Vm};

impl Vm {
    /// zpp's checks on a builtin's scalar / array typed parameters (from
    /// php's stubs): a `Stringable` object passed to a `string` parameter is
    /// converted with `__toString()`; other objects, and arrays where no
    /// array is accepted, are TypeErrors; null to a non-nullable scalar is
    /// deprecated (to an array, a TypeError). Builtins that stringify the
    /// elements of a `mixed` / array argument (`implode`, `sprintf`, …) get
    /// those converted too. `user`: called from user code (engine-internal
    /// prelude calls only get the conversions).
    pub(crate) fn coerce_builtin_args(
        &mut self,
        name: &str,
        sig: Option<&'static builtins::signatures::Func>,
        args: &mut [Value],
        user: bool,
    ) -> Result<(), Thrown> {
        let params = sig.map_or(&[][..], builtins::signatures::scalar_params);
        for sp in params {
            let (pos, pname, ty) = (sp.pos, sp.name, sp.ty);
            let (nullable, has_str, has_arr) = (sp.nullable, sp.has_str, sp.has_arr);
            let range = if sp.variadic {
                pos..args.len()
            } else {
                pos..(pos + 1).min(args.len())
            };
            if sp.by_ref {
                // `array &$array` (sort, usort, array_push, …): the referenced
                // value must already be an array.
                if user {
                    for i in range {
                        let cur = deref(&args[i]);
                        if !matches!(cur, Value::Array(_)) {
                            return Err(Thrown::type_error(format!(
                                "{name}(): Argument #{} (${pname}) must be of type {ty}, {} given",
                                i + 1,
                                self.zval_value_name(&cur)
                            )));
                        }
                    }
                }
                continue;
            }
            for i in range {
                let given: Vec<u8> = match &args[i] {
                    Value::Object(h) => {
                        // zend_parse_arg_str_weak: the object's cast_object.
                        let h = h.clone();
                        if has_str {
                            if let Some(s) = self.cast_object_vm(&h, CastTarget::Str)? {
                                args[i] = Value::str(conv::to_string_bytes(&s));
                                continue;
                            }
                        }
                        let cls = h.borrow().class;
                        self.class(cls).name.as_bytes().to_vec()
                    }
                    Value::Closure(_) => b"Closure".to_vec(),
                    Value::Array(_) if !has_arr && user => b"array".to_vec(),
                    Value::Null if !nullable && user => {
                        if has_str || ty != "array" {
                            let msg = format!(
                                "{name}(): Passing null to parameter #{} (${pname}) of type {ty} is deprecated",
                                i + 1
                            );
                            self.diag(8192, &msg)?;
                            continue;
                        }
                        b"null".to_vec()
                    }
                    Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Bool(_)
                        if (ty == "array" || ty == "?array") && user =>
                    {
                        type_lc(&args[i]).as_bytes().to_vec()
                    }
                    _ => continue,
                };
                return Err(Thrown::type_error(format!(
                    "{name}(): Argument #{} (${pname}) must be of type {ty}, {} given",
                    i + 1,
                    String::from_utf8_lossy(&given)
                )));
            }
        }
        // Elements stringified inside the builtin (zval_get_string).
        let elems: Option<(usize, bool)> = match name {
            "implode" | "join" => Some((0, true)),
            "sprintf" | "printf" => Some((1, false)),
            "fprintf" => Some((2, false)),
            "vsprintf" | "vprintf" => Some((1, true)),
            "vfprintf" => Some((2, true)),
            _ => None,
        };
        if let Some((from, arrays)) = elems {
            for arg in args.iter_mut().skip(from) {
                if arrays {
                    let Value::Array(a) = &*arg else { continue };
                    let hits: Vec<ArrayKey> = a
                        .iter()
                        .filter(|(_, v)| self.is_stringable(&deref(v)))
                        .map(|(k, _)| k.clone())
                        .collect();
                    if hits.is_empty() {
                        continue;
                    }
                    let mut a = a.clone();
                    for k in hits {
                        let v = deref(a.get(&k).unwrap_or(&Value::Null));
                        let s = self.to_string_vm(&v)?;
                        a.insert(k, Value::str(s));
                    }
                    *arg = Value::Array(a);
                } else if self.is_stringable(arg) {
                    let s = self.to_string_vm(arg)?;
                    *arg = Value::str(s);
                }
            }
        }
        Ok(())
    }

    /// An object with `__toString()` or an internal class's `cast_object`.
    fn is_stringable(&self, v: &Value) -> bool {
        match v {
            Value::Object(h) => {
                self.handlers_of(h).is_some_and(|t| t.cast_object.is_some())
                    || self
                        .lookup_method(h.borrow().class, b"__tostring")
                        .is_some()
            }
            _ => false,
        }
    }
}
