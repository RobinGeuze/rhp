//! The `fileinfo` extension: a port of ext/fileinfo/fileinfo.c over php's
//! bundled libmagic and magic database, which rhp-csrc compiles. The
//! `finfo` class is declared in PHP ([`PRELUDE_PHP`]) and calls
//! `__finfo` for its methods; the procedural functions are builtins. A
//! `finfo` object's payload is its `magic_set`.

mod hooks;

use std::cell::RefCell;
use std::rc::Rc;

use rhp_csrc::fileinfo::{self as lm, Magic};
use rhp_values::Value;

use crate::zend::object_handlers::{ClassHandlers, CloneObj, ObjectHandlers};
use crate::zend::zpp::{self, deref};
use crate::{Host, Thrown};

pub const PRELUDE_PHP: &str = include_str!("prelude.php");

/// `finfo`: no clone (php's `clone_obj = NULL`).
pub(crate) const CLASSES: &[ClassHandlers] = &[ClassHandlers {
    class: "finfo",
    handlers: &ObjectHandlers {
        clone_obj: CloneObj::None,
        ..ObjectHandlers::STD
    },
    create_object: true,
}];

/// `finfo_object`'s `magic`: none before the constructor ran, or after it
/// failed.
struct Finfo(RefCell<Option<Magic>>);

pub fn constants() -> Vec<(&'static str, Value)> {
    let i = |n: i32| Value::Int(n as i64);
    vec![
        ("FILEINFO_NONE", i(lm::MAGIC_NONE)),
        ("FILEINFO_SYMLINK", i(lm::MAGIC_SYMLINK)),
        ("FILEINFO_MIME", i(lm::MAGIC_MIME)),
        ("FILEINFO_MIME_TYPE", i(lm::MAGIC_MIME_TYPE)),
        ("FILEINFO_MIME_ENCODING", i(lm::MAGIC_MIME_ENCODING)),
        ("FILEINFO_DEVICES", i(lm::MAGIC_DEVICES)),
        ("FILEINFO_CONTINUE", i(lm::MAGIC_CONTINUE)),
        ("FILEINFO_PRESERVE_ATIME", i(lm::MAGIC_PRESERVE_ATIME)),
        ("FILEINFO_RAW", i(lm::MAGIC_RAW)),
        ("FILEINFO_APPLE", i(lm::MAGIC_APPLE)),
        ("FILEINFO_EXTENSION", i(lm::MAGIC_EXTENSION)),
    ]
}

/// PHP_MINFO(fileinfo)'s libmagic version: "5.46".
pub fn libmagic_version() -> String {
    let v = lm::version();
    format!("{}.{}", v / 100, v % 100)
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("finfo_open", fn_finfo_open),
            ("finfo_close", fn_finfo_close),
            ("finfo_set_flags", fn_finfo_set_flags),
            ("finfo_file", fn_finfo_file),
            ("finfo_buffer", fn_finfo_buffer),
            ("mime_content_type", fn_mime_content_type),
        ],
    );
    crate::register_helpers(m, &[("__finfo", fn_method, 2, None)]);
}

/// A finfo call: php's function name for messages, and whether it is the
/// method (which counts its arguments without the object).
struct Call {
    fname: &'static str,
    method: bool,
}

impl Call {
    /// The name of argument `n` (1-based) of this call, as php's
    /// `get_active_function_arg_name` gives it.
    fn arg_name(&self, n: usize, names: &[&'static str]) -> &'static str {
        let i = if self.method { n } else { n.wrapping_sub(1) };
        names.get(i).copied().unwrap_or("")
    }
}

/// `Z_PARAM_OBJECT_OF_CLASS(zv, finfo_class_entry)` at `i`.
fn finfo_arg(host: &dyn Host, args: &[Value], i: usize, f: &str) -> Result<Rc<Finfo>, Thrown> {
    let v = args.get(i).map(deref).unwrap_or(Value::Null);
    let ok = matches!(&v, Value::Object(_))
        && host
            .object_class_name(&v)
            .is_some_and(|c| host.reflect_is_a(&c, b"finfo"));
    match crate::zend::object_handlers::payload::<Finfo>(&v) {
        Some(fi) if ok => Ok(fi),
        _ if ok => Ok(Rc::new(Finfo(RefCell::new(None)))),
        _ => Err(Thrown::type_error(format!(
            "{f}(): Argument #{} ($finfo) must be of type finfo, {} given",
            i + 1,
            zpp::zval_value_name(host, &v)
        ))),
    }
}

/// `Z_PARAM_RESOURCE_OR_NULL` for `$context` at `i` (absent is null).
fn context_param(
    host: &dyn Host,
    call: &Call,
    args: &[Value],
    i: usize,
) -> Result<Option<Value>, Thrown> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => Ok(None),
        Some(v @ Value::Resource(_)) => Ok(Some(v)),
        Some(v) => Err(Thrown::type_error(format!(
            "{}(): Argument #{} ($context) must be of type resource or null, {} given",
            call.fname,
            if call.method { 3 } else { 4 },
            zpp::zval_value_name(host, &v)
        ))),
    }
}

/// `Invalid finfo object`: the constructor never ran or failed.
fn invalid() -> Thrown {
    Thrown::error("Error", "Invalid finfo object")
}

/// `magic_open` and `magic_load` for finfo_open and the constructor:
/// `file` is the database's path as given (empty: the built-in one).
fn open_magic(
    host: &mut dyn Host,
    fname: &str,
    flags: i64,
    file: Option<&[u8]>,
) -> Result<Option<Magic>, Thrown> {
    let file = match file {
        None | Some(b"") => None,
        Some(f) => {
            // php_check_open_basedir, then expand_filepath_with_mode.
            if !crate::main::fopen_wrappers::check(host, fname, f, true)? {
                return Ok(None);
            }
            // CWD_EXPAND: lexical, symlinks kept; virtual_file_ex refuses
            // a path of MAXPATHLEN - 1 bytes or more.
            if f.len() >= libc::PATH_MAX as usize - 1 {
                return Ok(None);
            }
            Some(crate::zend::virtual_cwd::expand_path(f))
        }
    };
    let Some(mut magic) = Magic::open(flags as i32) else {
        crate::warn(host, fname, &format!("Invalid mode '{flags}'."))?;
        return Ok(None);
    };
    if !hooks::run(host, fname, || magic.load(file.as_deref()))? {
        let mut msg = b"Failed to load magic database at \"".to_vec();
        msg.extend_from_slice(file.as_deref().unwrap_or(b"(null)"));
        msg.push(b'"');
        crate::warn(host, fname, &msg)?;
        return Ok(None);
    }
    Ok(Some(magic))
}

fn flags_and_file(
    host: &mut dyn Host,
    args: &[Value],
    off: usize,
    f: &str,
) -> Result<(i64, Option<Vec<u8>>), Thrown> {
    let flags = zpp::int_param(host, args, off, f, "flags", lm::MAGIC_NONE as i64)?;
    let file = match args.get(off + 1).map(deref) {
        None | Some(Value::Null) => None,
        Some(_) => Some(zpp::path_param(host, args, off + 1, f, "magic_database")?),
    };
    Ok((flags, file))
}

/// `finfo_open(int $flags = FILEINFO_NONE, ?string $magic_database = null)`.
fn fn_finfo_open(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "finfo_open";
    let (flags, file) = flags_and_file(host, args, 0, f)?;
    let Some(magic) = open_magic(host, f, flags, file.as_deref())? else {
        return Ok(Value::Bool(false));
    };
    let Some(obj) = host.instantiate(b"finfo", Vec::new()) else {
        return Ok(Value::Bool(false));
    };
    if let Some(h) = crate::zend::object_handlers::object_of(&obj) {
        crate::zend::object_handlers::set_payload(&h, Rc::new(Finfo(RefCell::new(Some(magic)))));
    }
    Ok(obj)
}

/// `finfo::__construct`: finfo_open on `$this` under EH_THROW, so its
/// first warning is the exception; any failure throws.
fn construct(host: &mut dyn Host, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let f = "finfo::__construct";
    let (flags, file) = flags_and_file(host, args, 0, f)?;
    let h = crate::zend::object_handlers::object_of(this).ok_or_else(invalid)?;
    let fi = match crate::zend::object_handlers::payload_of::<Finfo>(&h) {
        Some(fi) => fi,
        None => {
            let fi = Rc::new(Finfo(RefCell::new(None)));
            crate::zend::object_handlers::set_payload(&h, fi.clone());
            fi
        }
    };
    let eh = crate::diag::replace_error_handling_throw();
    drop(fi.0.borrow_mut().take());
    let r = open_magic(host, f, flags, file.as_deref());
    drop(eh);
    match r {
        Ok(Some(magic)) => {
            *fi.0.borrow_mut() = Some(magic);
            Ok(Value::Null)
        }
        // The first warning, as the exception.
        Err(t) => Err(t),
        Ok(None) => Err(Thrown::error("Exception", "Constructor failed")),
    }
}

/// `finfo_close(finfo $finfo)`: a no-op.
fn fn_finfo_close(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    finfo_arg(host, args, 0, "finfo_close")?;
    Ok(Value::Bool(true))
}

fn set_flags(
    host: &mut dyn Host,
    fi: &Finfo,
    args: &[Value],
    i: usize,
    f: &str,
) -> Result<Value, Thrown> {
    let flags = zpp::int_param(host, args, i, f, "flags", 0)?;
    let mut m = fi.0.try_borrow_mut().map_err(|_| invalid())?;
    let magic = m.as_mut().ok_or_else(invalid)?;
    magic.set_flags(flags as i32);
    Ok(Value::Bool(true))
}

fn fn_finfo_set_flags(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "finfo_set_flags";
    let fi = finfo_arg(host, args, 0, f)?;
    set_flags(host, &fi, args, 1, f)
}

/// Run `op` on the finfo's magic with `options` set for the call
/// (php_fileinfo's "Set options for the current file/buffer").
fn with_options<R>(
    fi: &Finfo,
    options: i64,
    op: impl FnOnce(&mut Magic) -> Result<R, Thrown>,
) -> Result<R, Thrown> {
    let mut m = fi.0.try_borrow_mut().map_err(|_| invalid())?;
    let magic = m.as_mut().ok_or_else(invalid)?;
    let old = magic.flags();
    if options != 0 {
        magic.set_flags(options as i32);
    }
    let r = op(magic);
    if options != 0 {
        magic.set_flags(old);
    }
    r
}

/// "Failed identify data %d:%s".
fn identify_failed(host: &mut dyn Host, fname: &str, magic: &Magic) -> Result<(), Thrown> {
    let mut msg = format!("Failed identify data {}:", magic.errno()).into_bytes();
    msg.extend_from_slice(&magic.error().unwrap_or_else(|| b"(null)".to_vec()));
    crate::warn(host, fname, &msg)
}

/// php_fileinfo_from_path: "directory" for a directory, else the stream's
/// type. `None`: the open, stat or identification failed.
fn from_path(
    host: &mut dyn Host,
    fname: &str,
    magic: &mut Magic,
    path: &[u8],
    ctx: &Value,
) -> Result<Option<Vec<u8>>, Thrown> {
    let Some(res) = crate::ext::standard::file::open_stream(host, fname, path, b"rb", ctx, false)?
    else {
        return Ok(None);
    };
    let mut out = None;
    let mut r = Ok(());
    if let Some(st) = host.stream_fstat(&res) {
        // php tests the S_IFDIR bit alone.
        if st.mode & libc::S_IFDIR as u32 != 0 {
            out = Some(b"directory".to_vec());
        } else {
            out = magic_stream(host, fname, magic, &res)?;
            if out.is_none() {
                r = identify_failed(host, fname, magic);
            }
        }
    }
    host.stream_close(&res);
    r?;
    Ok(out)
}

fn magic_stream(
    host: &mut dyn Host,
    fname: &str,
    magic: &mut Magic,
    res: &Value,
) -> Result<Option<Vec<u8>>, Thrown> {
    let h = hooks::Handle::borrowed(res.clone());
    // SAFETY: the handle lives until after the call.
    let r = hooks::run(host, fname, || unsafe { magic.stream(h) });
    unsafe { hooks::Handle::free(h) };
    r
}

/// finfo_file / finfo::file, the arguments after the object starting at
/// `off`.
fn file(
    host: &mut dyn Host,
    call: &Call,
    fi: &Finfo,
    args: &[Value],
    off: usize,
) -> Result<Value, Thrown> {
    let f = call.fname;
    let path = zpp::path_param(host, args, off, f, "filename")?;
    let options = zpp::int_param(host, args, off + 1, f, "flags", 0)?;
    let ctx = context_param(host, call, args, off + 2)?;
    if fi.0.borrow().is_none() {
        return Err(invalid());
    }
    if path.is_empty() {
        let names = ["finfo", "filename", "flags", "context"];
        return Err(zpp::value_error(
            f,
            2,
            call.arg_name(2, &names),
            "must not be empty",
        ));
    }
    // php_stream_context_from_zval
    let ctx = match ctx {
        Some(Value::Resource(r)) if r.type_name != "stream-context" => {
            return Err(Thrown::type_error(format!(
                "{f}(): supplied resource is not a valid Stream-Context resource"
            )));
        }
        Some(v) => v,
        None => host.default_stream_context(),
    };
    let r = with_options(fi, options, |magic| from_path(host, f, magic, &path, &ctx))?;
    Ok(r.map_or(Value::Bool(false), Value::str))
}

fn fn_finfo_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call {
        fname: "finfo_file",
        method: false,
    };
    let fi = finfo_arg(host, args, 0, call.fname)?;
    file(host, &call, &fi, args, 1)
}

/// finfo_buffer / finfo::buffer; `argc` counts the object for the function.
fn buffer(
    host: &mut dyn Host,
    call: &Call,
    fi: &Finfo,
    args: &[Value],
    off: usize,
    argc: usize,
) -> Result<Value, Thrown> {
    let f = call.fname;
    let data = zpp::str_param(host, args, off, f, "string")?;
    let options = zpp::int_param(host, args, off + 1, f, "flags", 0)?;
    context_param(host, call, args, off + 2)?;
    if argc == 4 || (call.method && argc == 3) {
        crate::diag::deprecated(
            host,
            f,
            "The $context parameter has no effect for finfo_buffer()",
        )?;
    }
    if fi.0.borrow().is_none() {
        return Err(invalid());
    }
    let r = with_options(fi, options, |magic| {
        let r = hooks::run(host, f, || magic.buffer(&data))?;
        if r.is_none() {
            identify_failed(host, f, magic)?;
        }
        Ok(r)
    })?;
    Ok(r.map_or(Value::Bool(false), Value::str))
}

fn fn_finfo_buffer(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call {
        fname: "finfo_buffer",
        method: false,
    };
    let fi = finfo_arg(host, args, 0, call.fname)?;
    buffer(host, &call, &fi, args, 1, args.len())
}

/// `__finfo($method, $this, $argc, ...$args)`: a `finfo` method.
fn fn_method(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let method = zpp::arg_bytes(args, 0);
    let this = args.get(1).map(deref).unwrap_or(Value::Null);
    let argc = args
        .get(2)
        .map(|v| rhp_values::conv::to_int(&deref(v)))
        .unwrap_or(0) as usize;
    let a = &args[3.min(args.len())..];
    let a = &a[..argc.min(a.len())];
    if method == b"__construct" {
        return construct(host, &this, a);
    }
    let fi = crate::zend::object_handlers::payload::<Finfo>(&this)
        .unwrap_or_else(|| Rc::new(Finfo(RefCell::new(None))));
    match &method[..] {
        b"file" => file(
            host,
            &Call {
                fname: "finfo::file",
                method: true,
            },
            &fi,
            a,
            0,
        ),
        b"buffer" => buffer(
            host,
            &Call {
                fname: "finfo::buffer",
                method: true,
            },
            &fi,
            a,
            0,
            argc,
        ),
        b"set_flags" => {
            let f = "finfo::set_flags";
            if fi.0.borrow().is_none() {
                zpp::int_param(host, a, 0, f, "flags", 0)?;
                return Err(invalid());
            }
            set_flags(host, &fi, a, 0, f)
        }
        _ => Err(Thrown::error(
            "Error",
            format!(
                "Call to undefined method finfo::{}()",
                String::from_utf8_lossy(&method)
            ),
        )),
    }
}

/// `mime_content_type(resource|string $filename)`.
fn fn_mime_content_type(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mime_content_type";
    let arg = args.first().map(deref).unwrap_or(Value::Null);
    let (path, stream) = match &arg {
        Value::Str(s) => {
            if s.is_empty() {
                return Err(zpp::value_error(f, 1, "filename", "must not be empty"));
            }
            if s.as_bytes().contains(&0) {
                return Err(Thrown::type_error(format!(
                    "{f}(): Argument #1 ($filename) must not contain any null bytes"
                )));
            }
            (Some(s.as_bytes().to_vec()), None)
        }
        Value::Resource(_) => {
            if host.stream_kind(&arg).is_none() {
                return Err(Thrown::type_error(format!(
                    "{f}(): supplied resource is not a valid stream resource"
                )));
            }
            (None, Some(arg.clone()))
        }
        v => {
            return Err(Thrown::type_error(format!(
                "{f}(): Argument #1 ($filename) must be of type resource|string, {} given",
                zpp::zval_value_name(host, v)
            )))
        }
    };
    let Some(mut magic) = Magic::open(lm::MAGIC_MIME_TYPE) else {
        crate::warn(host, f, "Failed to load magic database")?;
        return Ok(Value::Bool(false));
    };
    if !hooks::run(host, f, || magic.load(None))? {
        identify_failed(host, f, &magic)?;
        return Ok(Value::Bool(false));
    }
    let r = match (path, stream) {
        (Some(path), _) => {
            let ctx = host.default_stream_context();
            from_path(host, f, &mut magic, &path, &ctx)?
        }
        (None, Some(res)) => {
            let pos = host.stream_tell(&res).unwrap_or(-1);
            host.stream_seek(&res, 0, libc::SEEK_SET);
            let r = magic_stream(host, f, &mut magic, &res)?;
            let warned = if r.is_none() {
                identify_failed(host, f, &magic)
            } else {
                Ok(())
            };
            host.stream_seek(&res, pos, libc::SEEK_SET);
            warned?;
            r
        }
        (None, None) => None,
    };
    Ok(r.map_or(Value::Bool(false), Value::str))
}
