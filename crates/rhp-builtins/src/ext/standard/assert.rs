//! ext/standard/assert.c: `assert`, `assert_options` and the assert.* ini.

use std::cell::{Cell, RefCell};

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::main::ini::ini_parse_bool;
use crate::{Host, Thrown};

/// assert.c's module globals (ASSERTG).
struct Globals {
    active: Cell<bool>,
    bail: Cell<bool>,
    warning: Cell<bool>,
    exception: Cell<bool>,
    /// ASSERTG(callback): set by assert_options() or a runtime ini change;
    /// `None` falls back to the startup assert.callback (ASSERTG(cb)).
    callback: RefCell<Option<Value>>,
}

thread_local! {
    static G: Globals = const {
        Globals {
            active: Cell::new(true),
            bail: Cell::new(false),
            warning: Cell::new(true),
            exception: Cell::new(true),
            callback: RefCell::new(None),
        }
    };
}

/// Back to the defaults (a new request).
pub(crate) fn reset_state() {
    G.with(|g| {
        g.active.set(true);
        g.bail.set(false);
        g.warning.set(true);
        g.exception.set(true);
        g.callback.replace(None);
    });
}

/// The assert.* OnModify handlers: the new value into ASSERTG.
pub(crate) fn ini_update(name: &[u8], v: Option<&[u8]>) {
    let on = |d: bool| v.map_or(d, ini_parse_bool);
    G.with(|g| match name {
        b"assert.active" => g.active.set(on(true)),
        b"assert.bail" => g.bail.set(on(false)),
        b"assert.warning" => g.warning.set(on(true)),
        b"assert.exception" => g.exception.set(on(true)),
        // OnChangeCallback
        _ => {
            g.callback
                .replace(v.filter(|v| !v.is_empty()).map(|v| Value::str(v.to_vec())));
        }
    });
}

/// The deprecation an assert.* handler raises for value `v` (8.3).
pub(crate) fn ini_deprecation(name: &[u8], v: &[u8]) -> Option<String> {
    let deprecated = match name {
        b"assert.active" | b"assert.warning" | b"assert.exception" => !ini_parse_bool(v),
        b"assert.bail" => ini_parse_bool(v),
        b"assert.callback" => !v.is_empty(),
        _ => false,
    };
    deprecated.then(|| {
        format!(
            "{} INI setting is deprecated",
            String::from_utf8_lossy(name)
        )
    })
}

fn callback() -> Option<Value> {
    G.with(|g| g.callback.borrow().clone()).or_else(|| {
        crate::main::ini::ini_default_value(b"assert.callback")
            .flatten()
            .filter(|v| !v.is_empty())
            .map(Value::str)
    })
}

/// `assert($assertion, $description = null)`. A call the compiler sees
/// is skipped when zend.assertions is off and gets the exported
/// expression as its description.
fn fn_assert(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (active, bail, warning, exception) = G.with(|g| {
        (
            g.active.get(),
            g.bail.get(),
            g.warning.get(),
            g.exception.get(),
        )
    });
    if !active || crate::zend::assertions() <= 0 {
        return Ok(Value::Bool(true));
    }
    let description = match args.get(1).map(deref) {
        None | Some(Value::Null) => None,
        Some(Value::Object(o)) => {
            let v = Value::Object(o.clone());
            let cls = host.object_class_name(&v).unwrap_or_default();
            if host.reflect_is_a(&cls, b"Throwable") {
                if conv::to_bool(&deref(&args[0])) {
                    return Ok(Value::Bool(true));
                }
                return Err(Thrown::Object(o));
            }
            match crate::zend::object_handlers::to_string(host, &v)? {
                Some(s) => Some(s),
                None => return Err(description_type_error(host, &v)),
            }
        }
        Some(Value::Str(s)) => Some(s.as_bytes().to_vec()),
        // Scalars convert in coercive mode only.
        Some(v @ (Value::Int(_) | Value::Float(_) | Value::Bool(_))) => {
            if host.caller_strict_types() {
                return Err(description_type_error(host, &v));
            }
            Some(conv::to_string_bytes(&v))
        }
        Some(v) => return Err(description_type_error(host, &v)),
    };
    if conv::to_bool(&deref(&args[0])) {
        return Ok(Value::Bool(true));
    }
    let mut pending = None;
    if let Some(cb) = callback() {
        let file = host.executing_file().unwrap_or_else(|| b"Unknown".to_vec());
        let line = host.executing_line();
        let mut cargs = vec![Value::str(file), Value::Int(i64::from(line)), Value::Null];
        if let Some(d) = &description {
            cargs.push(Value::str(d.clone()));
        }
        let r = match host.check_callable(&cb)? {
            // zend_call_function: "Invalid callback".
            Some(why) => Err(Thrown::error(
                "Error",
                format!(
                    "Invalid callback {}, {why}",
                    String::from_utf8_lossy(&crate::ext::libxml::io::callable_name(host, &cb))
                ),
            )),
            None => host.call_callable(&cb, &cargs),
        };
        match r {
            Ok(_) => {}
            Err(t) if t.is_catchable() => pending = Some(t),
            Err(t) => return Err(t),
        }
    }
    let message = || {
        description
            .as_deref()
            .map(|d| String::from_utf8_lossy(d).into_owned())
    };
    if exception {
        let err = Thrown::error("AssertionError", message().unwrap_or_default());
        let err = match pending.take() {
            Some(prev) => host.chain_thrown(err, prev),
            None => err,
        };
        if bail {
            // The exception is not catchable: reported as uncaught at once.
            host.exception_error(err, 1);
            return Err(Thrown::Exit(255));
        }
        return Err(err);
    }
    if warning {
        let msg = format!("{} failed", message().unwrap_or_else(|| "Assertion".into()));
        crate::diag::warn(host, "assert", &msg)?;
    }
    if bail {
        if let Some(t) = pending.take() {
            host.exception_error(t, crate::E_WARNING);
        }
        return Err(Thrown::Exit(0));
    }
    match pending {
        Some(t) => Err(t),
        None => Ok(Value::Bool(false)),
    }
}

fn description_type_error(host: &dyn Host, v: &Value) -> Thrown {
    Thrown::type_error(format!(
        "assert(): Argument #2 ($description) must be of type Throwable|string|null, {} given",
        crate::zend::zpp::zval_value_name(host, v)
    ))
}

/// `assert_options($option, $value = UNKNOWN)`: an option's old value,
/// setting it through its ini directive.
fn fn_assert_options(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let what = crate::zend::zpp::zpp_long(host, args, 0, "assert_options", "option")?;
    let value = args.get(1).map(deref);
    let (key, old): (&[u8], bool) = match what {
        1 => (b"assert.active", G.with(|g| g.active.get())),
        3 => (b"assert.bail", G.with(|g| g.bail.get())),
        4 => (b"assert.warning", G.with(|g| g.warning.get())),
        5 => (b"assert.exception", G.with(|g| g.exception.get())),
        2 => {
            let old = callback().unwrap_or(Value::Null);
            if let Some(v) = value {
                let new = (!matches!(v, Value::Null)).then(|| args[1].clone());
                G.with(|g| g.callback.replace(new));
            }
            return Ok(old);
        }
        _ => {
            return Err(Thrown::error(
                "ValueError",
                "assert_options(): Argument #1 ($option) must be an ASSERT_* constant",
            ))
        }
    };
    if let Some(v) = value {
        let s = match &v {
            Value::Object(_) => match crate::zend::object_handlers::to_string(host, &v)? {
                Some(s) => s,
                None => {
                    let cls = host.object_class_name(&v).unwrap_or_default();
                    return Err(Thrown::error(
                        "Error",
                        format!(
                            "Object of class {} could not be converted to string",
                            String::from_utf8_lossy(&cls)
                        ),
                    ));
                }
            },
            Value::Array(_) => {
                host.emit_diagnostic(crate::E_WARNING, b"Array to string conversion")?;
                b"Array".to_vec()
            }
            v => conv::to_string_bytes(v),
        };
        crate::main::ini::alter_ini_entry_ex(host, key, s, "assert_options", true)?;
    }
    Ok(Value::Int(i64::from(old)))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[("assert", fn_assert), ("assert_options", fn_assert_options)],
    );
}

/// assert.c's `ASSERT_*` (deprecated with assert_options()).
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("ASSERT_ACTIVE", Value::Int(1)),
        ("ASSERT_CALLBACK", Value::Int(2)),
        ("ASSERT_BAIL", Value::Int(3)),
        ("ASSERT_WARNING", Value::Int(4)),
        ("ASSERT_EXCEPTION", Value::Int(5)),
    ]
}

/// The E_DEPRECATED for reading an `ASSERT_*` constant.
pub(crate) fn constant_deprecation(name: &[u8]) -> Option<String> {
    constants()
        .iter()
        .any(|(n, _)| n.as_bytes() == name)
        .then(|| {
            format!(
                "Constant {} is deprecated since 8.3, as assert_options() is deprecated",
                String::from_utf8_lossy(name)
            )
        })
}
