//! zpp for builtin calls: the checks and coercions php's
//! `zend_parse_parameters` applies to a builtin's scalar and array
//! parameters, driven by the stub signatures.

use rhp_builtins as builtins;
use rhp_builtins::object::CastTarget;
use rhp_builtins::signatures::{ScalarParam, Zpp};
use rhp_builtins::E_DEPRECATED;
use rhp_values::{conv, fmt, ArrayKey, Thrown, Value};

use crate::{deref, Vm};

impl Vm {
    /// zpp's checks on a builtin's scalar / array typed parameters (from
    /// php's stubs), in the caller's typing mode (`strict`): each argument
    /// is parsed the way the `Z_PARAM_*` for its type parses it, so the
    /// builtin gets the coerced value (an `int` parameter an int, an
    /// `int|float` one an int or a float, …) or the call fails with php's
    /// TypeError or deprecation. Builtins that stringify the elements of a
    /// `mixed` / array argument (`implode`, `sprintf`, …) get those
    /// converted too. `user`: called from user code (engine-internal prelude
    /// calls only get the object conversions). Past the first parameter
    /// whose checks are the body's (a resource, a callable, …), only the
    /// object, array and null checks run here, so that parameter's error
    /// still comes first. `frameless`: php runs the call's
    /// ZEND_FRAMELESS_FUNCTION variant, whose zpp may differ.
    pub(crate) fn coerce_builtin_args(
        &mut self,
        name: &str,
        sig: Option<&'static builtins::signatures::Func>,
        args: &mut [Value],
        user: bool,
        strict: bool,
        frameless: bool,
    ) -> Result<(), Thrown> {
        let sps =
            sig.and_then(|f| builtins::signatures::scalar_params_for(f, args.len(), frameless));
        let (params, prefix) = match sps {
            Some(sps) => (&sps.params[..], sps.prefix),
            None => (&[][..], 0),
        };
        for sp in params {
            let range = if sp.variadic {
                sp.pos..args.len()
            } else {
                sp.pos..(sp.pos + 1).min(args.len())
            };
            for i in range {
                // A reference in a by-value position (call_user_func_array
                // with `&$x` elements) is read when zpp reaches it: an
                // earlier argument's __toString may have changed it.
                if user && !sp.by_ref && matches!(args[i], Value::Ref(_)) {
                    args[i] = deref(&args[i]);
                }
                let got = if sp.by_ref {
                    // `array &$array` (sort, usort, array_push, …): the
                    // referenced value must already be an array.
                    match deref(&args[i]) {
                        Value::Array(_) => Got::Keep,
                        _ if user => Got::Fail,
                        _ => Got::Keep,
                    }
                } else if user && i < prefix {
                    self.zpp_arg(name, sp, i, &args[i], strict)?
                } else if user {
                    self.zpp_generic_arg(name, sp, i, &args[i])?
                } else {
                    self.zpp_object_arg(sp, &args[i], false)?
                };
                match got {
                    Got::Keep => {}
                    Got::Set(v) => args[i] = v,
                    Got::Fail => {
                        let given = rhp_builtins::zval_value_name(self, &deref(&args[i]));
                        let ty = sp.kind.expected(sp.nullable).unwrap_or(sp.ty);
                        return Err(Thrown::type_error(format!(
                            "{name}(): Argument #{}{} must be of type {ty}, {given} given",
                            i + 1,
                            arg_label(sp),
                        )));
                    }
                }
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

    /// One argument from user code, parsed for its parameter's [`Zpp`] in
    /// the caller's mode (zend_parse_arg_*_weak, or the strict fast paths).
    fn zpp_arg(
        &mut self,
        name: &str,
        sp: &ScalarParam,
        i: usize,
        v: &Value,
        strict: bool,
    ) -> Result<Got, Thrown> {
        use Zpp::*;
        if sp.kind == Class {
            return self.zpp_class(name, sp, i, v);
        }
        if let Value::Null = v {
            if sp.nullable {
                return Ok(Got::Keep);
            }
            if strict || sp.kind == Array {
                return Ok(Got::Fail);
            }
            let msg = format!(
                "{name}(): Passing null to parameter #{}{} of type {} is deprecated",
                i + 1,
                arg_label(sp),
                sp.ty
            );
            self.diag(E_DEPRECATED, &msg)?;
            return Ok(match sp.kind {
                Long | Number | StrOrLong | NumberOrStr | ArrayOrLong => Got::Set(Value::Int(0)),
                Double => Got::Set(Value::Float(0.0)),
                Bool => Got::Set(Value::Bool(false)),
                Str | ArrayOrStr | Array | Class | Other => Got::Keep,
            });
        }
        let ok = match (sp.kind, v) {
            (Str | StrOrLong | NumberOrStr | ArrayOrStr, Value::Str(_)) => true,
            (Long | Number | StrOrLong | NumberOrStr | ArrayOrLong, Value::Int(_)) => true,
            (Double | Number | NumberOrStr, Value::Float(_)) => true,
            (Bool, Value::Bool(_)) => true,
            (Array | ArrayOrStr | ArrayOrLong, Value::Array(_)) => true,
            // SSTH exception: an int is a float in either mode.
            (Double, Value::Int(n)) => return Ok(Got::Set(Value::Float(*n as f64))),
            (Other, _) => return self.zpp_object_arg(sp, v, true),
            _ => false,
        };
        if ok {
            return Ok(Got::Keep);
        }
        if strict {
            return Ok(Got::Fail);
        }
        Ok(match sp.kind {
            Str | ArrayOrStr => match v {
                Value::Float(f) => {
                    self.nan_coerced(*f, "string")?;
                    Got::Keep
                }
                Value::Int(_) | Value::Bool(_) => Got::Keep,
                _ => self.zpp_str_object(v)?,
            },
            Long | ArrayOrLong => self.zpp_long(v)?,
            StrOrLong => match self.zpp_long(v)? {
                Got::Fail => match v {
                    // Not an int (NAN, INF, out of range): a string.
                    Value::Float(f) => {
                        self.nan_coerced(*f, "string")?;
                        Got::Set(Value::str(conv::to_string_bytes(v)))
                    }
                    _ => self.zpp_str_object(v)?,
                },
                got => got,
            },
            Double => match v {
                Value::Bool(b) => Got::Set(Value::Float(*b as i64 as f64)),
                Value::Str(s) => match conv::numeric_string(s.as_bytes()) {
                    Some(conv::Num::Int(n)) => Got::Set(Value::Float(n as f64)),
                    Some(conv::Num::Float(f)) => Got::Set(Value::Float(f)),
                    None => Got::Fail,
                },
                _ => Got::Fail,
            },
            Bool => match v {
                Value::Int(_) | Value::Float(_) | Value::Str(_) => {
                    if let Value::Float(f) = v {
                        self.nan_coerced(*f, "bool")?;
                    }
                    Got::Set(Value::Bool(conv::to_bool(v)))
                }
                _ => Got::Fail,
            },
            Number => match v {
                Value::Bool(b) => Got::Set(Value::Int(*b as i64)),
                Value::Str(s) => match conv::numeric_string(s.as_bytes()) {
                    Some(conv::Num::Int(n)) => Got::Set(Value::Int(n)),
                    Some(conv::Num::Float(f)) => Got::Set(Value::Float(f)),
                    None => Got::Fail,
                },
                _ => Got::Fail,
            },
            NumberOrStr => match v {
                Value::Bool(b) => Got::Set(Value::Int(*b as i64)),
                _ => self.zpp_str_object(v)?,
            },
            Array | Class | Other => Got::Fail,
        })
    }

    /// zend_parse_arg_class: null where the type allows it, else the value
    /// as a string (try_convert_to_string, in either mode), which must name
    /// a class (autoloaded) derived from the parameter's base class when
    /// it has one.
    #[cold]
    fn zpp_class(
        &mut self,
        name: &str,
        sp: &ScalarParam,
        i: usize,
        v: &Value,
    ) -> Result<Got, Thrown> {
        let base = builtins::signatures::class_param_base(name, sp.name);
        if sp.nullable && matches!(v, Value::Null) {
            return Ok(Got::Keep);
        }
        let s = match v {
            Value::Closure(_) => {
                return Err(Thrown::error(
                    "Error",
                    "Object of class Closure could not be converted to string",
                ))
            }
            _ => self.to_string_vm(v)?,
        };
        let found = builtins::Host::resolve_class_autoload(self, &s)?;
        let ok = match (found, base) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(id), Some(base)) => {
                let class = self.class(id).name.as_bytes().to_vec();
                builtins::Host::reflect_is_a(self, &class, base.as_bytes())
            }
        };
        if !ok {
            let or_null = if sp.nullable { " or null" } else { "" };
            let want = match base {
                Some(b) => format!("a class name derived from {b}{or_null}"),
                None => format!("a valid class name{or_null}"),
            };
            return Err(Thrown::type_error(format!(
                "{name}(): Argument #{}{} must be {want}, {} given",
                i + 1,
                arg_label(sp),
                String::from_utf8_lossy(&s)
            )));
        }
        Ok(Got::Set(Value::str(s)))
    }

    /// The checks of a scalar parameter after one the body checks: an
    /// object converts with its string cast where the type takes a string,
    /// arrays only where the type takes one, and null to a non-nullable
    /// scalar is deprecated (to `array`, a TypeError).
    fn zpp_generic_arg(
        &mut self,
        name: &str,
        sp: &ScalarParam,
        i: usize,
        v: &Value,
    ) -> Result<Got, Thrown> {
        Ok(match v {
            Value::Null if !sp.nullable => {
                if sp.has_str || sp.ty != "array" {
                    let msg = format!(
                        "{name}(): Passing null to parameter #{}{} of type {} is deprecated",
                        i + 1,
                        arg_label(sp),
                        sp.ty
                    );
                    self.diag(E_DEPRECATED, &msg)?;
                    Got::Keep
                } else {
                    Got::Fail
                }
            }
            Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Bool(_)
                if sp.ty == "array" || sp.ty == "?array" =>
            {
                Got::Fail
            }
            _ => self.zpp_object_arg(sp, v, true)?,
        })
    }

    /// zend_parse_arg_long_weak for a non-int, non-null value.
    fn zpp_long(&mut self, v: &Value) -> Result<Got, Thrown> {
        Ok(match v {
            Value::Bool(b) => Got::Set(Value::Int(*b as i64)),
            Value::Float(f) => match long_of(*f) {
                None => Got::Fail,
                Some(n) => {
                    if n as f64 != *f {
                        let shown = String::from_utf8_lossy(&fmt::float_serialize(*f)).into_owned();
                        self.diag(
                            E_DEPRECATED,
                            &format!(
                                "Implicit conversion from float {shown} to int loses precision"
                            ),
                        )?;
                    }
                    Got::Set(Value::Int(n))
                }
            },
            Value::Str(s) => match conv::numeric_string(s.as_bytes()) {
                Some(conv::Num::Int(n)) => Got::Set(Value::Int(n)),
                Some(conv::Num::Float(f)) => match long_of(f) {
                    None => Got::Fail,
                    Some(n) => {
                        if n as f64 != f {
                            let msg = format!(
                                "Implicit conversion from float-string \"{}\" to int loses precision",
                                String::from_utf8_lossy(s.as_bytes())
                            );
                            self.diag(E_DEPRECATED, &msg)?;
                        }
                        Got::Set(Value::Int(n))
                    }
                },
                None => Got::Fail,
            },
            _ => Got::Fail,
        })
    }

    /// php 8.5's warning when NAN converts to a string or a bool.
    fn nan_coerced(&mut self, f: f64, to: &str) -> Result<(), Thrown> {
        if f.is_nan() {
            self.diag(
                builtins::E_WARNING,
                &format!("unexpected NAN value was coerced to {to}"),
            )?;
        }
        Ok(())
    }

    /// zend_parse_arg_str_weak for an object: its `cast_object` to string.
    fn zpp_str_object(&mut self, v: &Value) -> Result<Got, Thrown> {
        if let Value::Object(h) = v {
            if let Some(s) = self.cast_object_vm(h, CastTarget::Str)? {
                return Ok(Got::Set(Value::str(conv::to_string_bytes(&s))));
            }
        }
        Ok(Got::Fail)
    }

    /// The checks a prelude call still gets, and (with `arrays`) those of a
    /// parameter whose type is no [`Zpp`] shape: an object converts with its
    /// string cast where the type takes a string and is a TypeError
    /// otherwise; an array where none is accepted is a TypeError.
    fn zpp_object_arg(&mut self, sp: &ScalarParam, v: &Value, arrays: bool) -> Result<Got, Thrown> {
        Ok(match v {
            Value::Object(_) if sp.has_str => self.zpp_str_object(v)?,
            Value::Object(_) | Value::Closure(_) => Got::Fail,
            Value::Array(_) if arrays && !sp.has_arr => Got::Fail,
            _ => Got::Keep,
        })
    }
}

/// What zpp made of one argument.
enum Got {
    Keep,
    Set(Value),
    Fail,
}

/// ` ($name)` after "Argument #n", or nothing for a variadic argument
/// (get_function_arg_name has no name past the declared parameters).
fn arg_label(sp: &ScalarParam) -> String {
    if sp.variadic {
        String::new()
    } else {
        format!(" (${})", sp.name)
    }
}

/// zend_dval_to_lval for a float zpp accepts as an int: `None` for NAN and
/// values outside the int range (ZEND_DOUBLE_FITS_LONG).
fn long_of(f: f64) -> Option<i64> {
    const TWO_63: f64 = 9_223_372_036_854_775_808.0;
    (!f.is_nan() && (-TWO_63..TWO_63).contains(&f)).then_some(f as i64)
}
