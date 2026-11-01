//! The `libxml` extension: the shared XML engine (tree, parser, serializer,
//! XPath, XSD — the parts of libxml2 php's XML extensions use) plus php's own
//! libxml glue: the error list behind `libxml_use_internal_errors` /
//! `libxml_get_errors`, the `LibXMLError` class and the `LIBXML_*` constants.

pub mod parser;
pub mod tree;
pub mod xpath;
pub mod xsd;

use std::cell::RefCell;
use std::collections::HashMap;

use rhp_values::{conv, PhpArray, Value};

use crate::{Builtin, Host, Thrown};
pub use parser::XmlError;

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

/// How a diagnostic is shown as a php warning when internal errors are off.
#[derive(Clone, Copy, PartialEq)]
pub enum Ctx {
    /// A parser diagnostic: `msg in <file|Entity>, line: N`.
    Parser,
    /// Anything else (validation, I/O): the bare message.
    Plain,
    /// libxml2's generic error handler (no parser context registered, as
    /// SimpleXML loads): `Entity: line N: parser error : msg`, then the
    /// source line and a caret line, each its own warning.
    Generic,
}

struct State {
    use_internal: bool,
    errors: Vec<XmlError>,
    last: Option<XmlError>,
    entity_loader_disabled: bool,
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State {
        use_internal: false,
        errors: Vec::new(),
        last: None,
        entity_loader_disabled: false,
    }) };
}

pub fn reset_state() {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.use_internal = false;
        s.errors.clear();
        s.last = None;
    });
}

pub fn use_internal() -> bool {
    STATE.with(|s| s.borrow().use_internal)
}

/// Route libxml2 diagnostics the way php's libxml glue does: always record
/// the last error; collect into the error list when internal errors are on,
/// else raise each one as an E_WARNING from `fname`.
pub fn report(
    host: &mut dyn Host,
    fname: &str,
    errors: Vec<XmlError>,
    ctx: Ctx,
) -> Result<(), Thrown> {
    for e in errors {
        let internal = STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.last = Some(e.clone());
            if s.use_internal {
                s.errors.push(e.clone());
                true
            } else {
                false
            }
        });
        if internal {
            continue;
        }
        let msg = e.message.trim_end_matches('\n');
        let text = match ctx {
            Ctx::Parser => {
                let file = match &e.file {
                    Some(f) => String::from_utf8_lossy(f).into_owned(),
                    None => "Entity".to_string(),
                };
                format!("{msg} in {file}, line: {}", e.line)
            }
            Ctx::Plain => msg.to_string(),
            Ctx::Generic => {
                let mut head = String::new();
                match &e.file {
                    Some(f) => {
                        head.push_str(&format!("{}:{}: ", String::from_utf8_lossy(f), e.line))
                    }
                    None if e.line != 0 && e.domain == 1 => {
                        head.push_str(&format!("Entity: line {}: ", e.line))
                    }
                    None => {}
                }
                head.push_str(match e.domain {
                    1 => "parser ",
                    3 => "namespace ",
                    _ => "",
                });
                head.push_str(if e.level == 1 {
                    "warning : "
                } else {
                    "error : "
                });
                head.push_str(msg);
                crate::warn(host, fname, &head)?;
                if let Some((content, caret)) = &e.context {
                    crate::warn(host, fname, &String::from_utf8_lossy(content))?;
                    crate::warn(host, fname, &String::from_utf8_lossy(caret))?;
                }
                continue;
            }
        };
        crate::warn(host, fname, &text)?;
    }
    Ok(())
}

/// A non-parser error record (validation, I/O): no file/column.
pub fn plain_error(level: i32, code: i32, domain: i32, line: i32, msg: String) -> XmlError {
    XmlError {
        domain,
        level,
        code,
        line,
        column: 0,
        message: msg,
        file: None,
        context: None,
    }
}

/// Report php's "I/O warning : failed to load external entity" for `path`.
pub fn report_io(host: &mut dyn Host, fname: &str, path: &[u8]) -> Result<(), Thrown> {
    let p = String::from_utf8_lossy(path).into_owned();
    let e = plain_error(
        1,
        1549,
        8,
        0,
        format!("failed to load external entity \"{p}\"\n"),
    );
    let internal = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.last = Some(e.clone());
        if s.use_internal {
            s.errors.push(e.clone());
            true
        } else {
            false
        }
    });
    if !internal {
        crate::warn(
            host,
            fname,
            &format!("I/O warning : failed to load external entity \"{p}\""),
        )?;
    }
    Ok(())
}

fn error_object(host: &mut dyn Host, e: &XmlError) -> Value {
    let props = vec![
        (b"level".to_vec(), Value::Int(e.level as i64)),
        (b"code".to_vec(), Value::Int(e.code as i64)),
        (b"column".to_vec(), Value::Int(e.column as i64)),
        (
            b"message".to_vec(),
            Value::str(e.message.as_bytes().to_vec()),
        ),
        (
            b"file".to_vec(),
            Value::str(e.file.clone().unwrap_or_default()),
        ),
        (b"line".to_vec(), Value::Int(e.line as i64)),
    ];
    host.instantiate(b"LibXMLError", props)
        .unwrap_or(Value::Null)
}

fn fn_use_internal_errors(_h: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let arg = args.first().map(deref);
    let prev = use_internal();
    match arg {
        None | Some(Value::Null) => {}
        Some(v) => {
            let on = conv::to_bool(&v);
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                if !on {
                    s.errors.clear();
                }
                s.use_internal = on;
            });
        }
    }
    Ok(Value::Bool(prev))
}

fn fn_get_errors(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let errs = STATE.with(|s| s.borrow().errors.clone());
    let mut a = PhpArray::new();
    for e in &errs {
        let o = error_object(host, e);
        a.push(o);
    }
    Ok(Value::Array(a))
}

fn fn_get_last_error(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    match STATE.with(|s| s.borrow().last.clone()) {
        Some(e) => Ok(error_object(host, &e)),
        None => Ok(Value::Bool(false)),
    }
}

fn fn_clear_errors(_h: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.errors.clear();
        s.last = None;
    });
    Ok(Value::Null)
}

fn fn_disable_entity_loader(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    crate::deprecated_fn(
        host,
        "libxml_disable_entity_loader",
        "8.0",
        "as external entity loading is disabled by default",
    )?;
    let on = args
        .first()
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(true);
    let prev = STATE.with(|s| std::mem::replace(&mut s.borrow_mut().entity_loader_disabled, on));
    Ok(Value::Bool(prev))
}

fn fn_set_streams_context(_h: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Null)
}

fn fn_set_external_entity_loader(_h: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
}

fn fn_get_external_entity_loader(_h: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Null)
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    }
    put(
        m,
        "libxml_use_internal_errors",
        fn_use_internal_errors,
        0,
        Some(1),
    );
    put(m, "libxml_get_errors", fn_get_errors, 0, Some(0));
    put(m, "libxml_get_last_error", fn_get_last_error, 0, Some(0));
    put(m, "libxml_clear_errors", fn_clear_errors, 0, Some(0));
    put(
        m,
        "libxml_disable_entity_loader",
        fn_disable_entity_loader,
        0,
        Some(1),
    );
    put(
        m,
        "libxml_set_streams_context",
        fn_set_streams_context,
        1,
        Some(1),
    );
    put(
        m,
        "libxml_set_external_entity_loader",
        fn_set_external_entity_loader,
        1,
        Some(1),
    );
    put(
        m,
        "libxml_get_external_entity_loader",
        fn_get_external_entity_loader,
        0,
        Some(0),
    );
}

pub fn constants() -> Vec<(&'static str, Value)> {
    let i = Value::Int;
    vec![
        ("LIBXML_VERSION", i(20913)),
        ("LIBXML_DOTTED_VERSION", Value::str(b"2.9.13".to_vec())),
        ("LIBXML_LOADED_VERSION", Value::str(b"20913".to_vec())),
        ("LIBXML_RECOVER", i(1)),
        ("LIBXML_NOENT", i(2)),
        ("LIBXML_DTDLOAD", i(4)),
        ("LIBXML_DTDATTR", i(8)),
        ("LIBXML_DTDVALID", i(16)),
        ("LIBXML_NOERROR", i(32)),
        ("LIBXML_NOWARNING", i(64)),
        ("LIBXML_NOBLANKS", i(256)),
        ("LIBXML_XINCLUDE", i(1024)),
        ("LIBXML_NSCLEAN", i(8192)),
        ("LIBXML_NOCDATA", i(16384)),
        ("LIBXML_NONET", i(2048)),
        ("LIBXML_PEDANTIC", i(128)),
        ("LIBXML_COMPACT", i(65536)),
        ("LIBXML_NOXMLDECL", i(2)),
        ("LIBXML_PARSEHUGE", i(524288)),
        ("LIBXML_BIGLINES", i(4194304)),
        ("LIBXML_NOEMPTYTAG", i(4)),
        ("LIBXML_SCHEMA_CREATE", i(1)),
        ("LIBXML_HTML_NOIMPLIED", i(8192)),
        ("LIBXML_HTML_NODEFDTD", i(4)),
        ("LIBXML_NO_XXE", i(8388608)),
        ("LIBXML_ERR_NONE", i(0)),
        ("LIBXML_ERR_WARNING", i(1)),
        ("LIBXML_ERR_ERROR", i(2)),
        ("LIBXML_ERR_FATAL", i(3)),
    ]
}

pub const PRELUDE_PHP: &str = r#"
class LibXMLError {
    public int $level;
    public int $code;
    public int $column;
    public string $message;
    public string $file;
    public int $line;
}
"#;

/// Drop diagnostics suppressed by LIBXML_NOWARNING / LIBXML_NOERROR.
pub fn filter_by_flags(errors: Vec<XmlError>, flags: i64) -> Vec<XmlError> {
    errors
        .into_iter()
        .filter(|e| !((e.level == 1 && flags & 64 != 0) || (e.level >= 2 && flags & 32 != 0)))
        .collect()
}
