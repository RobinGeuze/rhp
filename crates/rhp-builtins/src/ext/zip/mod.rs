//! The `zip` extension on libzip: a port of ext/zip/php_zip.c and
//! zip_stream.c. `ZipArchive` is declared in PHP ([`prelude`]); its methods
//! call `__zip`, its properties go through `__get`/`__set`/`__isset`/
//! `__unset` to `__zip_prop` (php's read_property, write_property and
//! has_property handlers), and its views through `get_properties_for`. An
//! object's state (php's `ze_zip_object`) is its payload; the open archive
//! (`php_zip_archive`) is shared with the entry streams opened from it. The
//! procedural `zip_*` functions use native resources.

mod archive;
mod procedural;
mod stream;
pub(crate) mod sys;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_double, c_int, c_longlong, c_void, CString};
use std::ptr;
use std::rc::Rc;
use std::sync::OnceLock;

use rhp_values::{ArrayKey, ObjectHandle, PhpArray, PhpStr, Value};

use crate::ext::libxml::ffi;
use crate::object::{ClassHandlers, CloneObj, ObjectHandlers, PropPurpose};
use crate::{Builtin, Host, Thrown};

pub use stream::wrappers;

/// `ZipArchive`'s handlers: no clone, the properties' view, and the close
/// of an archive whose object goes.
pub(crate) static CLASSES: &[ClassHandlers] = &[ClassHandlers {
    class: "ZipArchive",
    handlers: &HANDLERS,
    create_object: true,
}];

static HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(get_properties_for),
    clone_obj: CloneObj::None,
    free_obj_host: Some(object_free),
    ..ObjectHandlers::STD
};

/// php_zip_object_free_storage's close, which can run the archive's
/// callbacks: here, where there is a host. The payload's `Drop` closes an
/// archive the destructor pass did not reach, without them.
fn object_free(host: &mut dyn Host, obj: &ObjectHandle) -> Result<(), Thrown> {
    let Some(o) = crate::object::payload_of::<ZipObject>(obj) else {
        return Ok(());
    };
    let Some(a) = o.archive.borrow_mut().take() else {
        return Ok(());
    };
    release(host, None, a)
}

/// `php_zip_archive`: an open libzip archive, owned by its `ZipArchive`
/// and by the entry streams opened from it. The last owner closes it.
pub(crate) struct Archive {
    /// Null once closed.
    za: Cell<*mut sys::zip_t>,
    /// What `addFromString` handed libzip, read until the archive closes.
    buffers: RefCell<Vec<Box<[u8]>>>,
    /// Set while `close()`/`open()` closes it ("Already being closed").
    close: Cell<bool>,
    progress: RefCell<Option<Value>>,
    cancel: RefCell<Option<Value>>,
}

impl Archive {
    fn new(za: *mut sys::zip_t) -> Rc<Archive> {
        Rc::new(Archive {
            za: Cell::new(za),
            buffers: RefCell::new(Vec::new()),
            close: Cell::new(false),
            progress: RefCell::new(None),
            cancel: RefCell::new(None),
        })
    }

    pub(crate) fn za(&self) -> *mut sys::zip_t {
        self.za.get()
    }
}

impl Drop for Archive {
    /// php_zip_archive_release without a host: callbacks registered on the
    /// archive see no PHP and do nothing; a failed close is discarded
    /// without php's warning.
    fn drop(&mut self) {
        let za = self.za.replace(ptr::null_mut());
        if !za.is_null() {
            unsafe {
                if sys::zip_close(za) != 0 {
                    sys::zip_discard(za);
                }
            }
        }
    }
}

/// php_zip_archive_release with a host: the last reference closes the
/// archive while its callbacks can reach PHP, warning as php does when the
/// close fails. `fname` is the `ZipArchive` method running; with none the
/// warning names php's active function (`fclose`, `main`, …).
pub(crate) fn release(
    host: &mut dyn Host,
    fname: Option<&str>,
    a: Rc<Archive>,
) -> Result<(), Thrown> {
    if Rc::strong_count(&a) > 1 {
        return Ok(());
    }
    let za = a.za.get();
    if za.is_null() {
        return Ok(());
    }
    let mut rc = 0;
    let r = ffi::with_host(host, fname.unwrap_or("Unknown"), || {
        rc = unsafe { sys::zip_close(za) }
    });
    a.za.set(ptr::null_mut());
    if rc != 0 {
        let msg = String::from_utf8_lossy(unsafe { sys::cstr(sys::zip_strerror(za)) }).into_owned();
        unsafe { sys::zip_discard(za) };
        let msg = format!("Cannot destroy the zip context: {msg}");
        match fname {
            Some(f) => crate::warn(host, f, &msg)?,
            None => host.docref_diagnostic(crate::E_WARNING, &msg)?,
        }
    }
    drop(a);
    r
}

/// `ze_zip_object`: a `ZipArchive`'s state.
#[derive(Default)]
pub(crate) struct ZipObject {
    archive: RefCell<Option<Rc<Archive>>>,
    filename: RefCell<Option<Vec<u8>>>,
    last_id: Cell<i64>,
    /// The archive's error, saved by `close()` for the properties.
    err_zip: Cell<c_int>,
    err_sys: Cell<c_int>,
}

impl ZipObject {
    fn new() -> ZipObject {
        ZipObject {
            last_id: Cell::new(-1),
            ..Default::default()
        }
    }

    /// php_zip_object_za
    pub(crate) fn za(&self) -> *mut sys::zip_t {
        self.archive
            .borrow()
            .as_ref()
            .map_or(ptr::null_mut(), |a| a.za.get())
    }

    pub(crate) fn archive(&self) -> Option<Rc<Archive>> {
        self.archive.borrow().clone()
    }

    /// php_zip_status
    fn status(&self) -> i64 {
        let za = self.za();
        if za.is_null() {
            return self.err_zip.get() as i64;
        }
        unsafe {
            let e = sys::zip_get_error(za);
            let c = sys::zip_error_code_zip(e);
            sys::zip_error_fini(e);
            c as i64
        }
    }

    /// php_zip_status_sys
    fn status_sys(&self) -> i64 {
        let za = self.za();
        if za.is_null() {
            return self.err_sys.get() as i64;
        }
        unsafe {
            let e = sys::zip_get_error(za);
            let c = sys::zip_error_code_system(e);
            sys::zip_error_fini(e);
            c as i64
        }
    }

    /// php_zip_get_num_files
    fn num_files(&self) -> i64 {
        let za = self.za();
        if za.is_null() {
            return 0;
        }
        unsafe { sys::zip_get_num_entries(za, 0) }
    }

    /// php_zipobj_get_zip_comment
    fn comment(&self) -> Vec<u8> {
        let za = self.za();
        if za.is_null() {
            return Vec::new();
        }
        let mut len: c_int = 0;
        let p = unsafe { sys::zip_get_archive_comment(za, &mut len, 0) };
        if p.is_null() {
            return Vec::new();
        }
        unsafe { std::slice::from_raw_parts(p.cast::<u8>(), len.max(0) as usize) }.to_vec()
    }
}

/// The state of `this`, a `ZipArchive` (or subclass) object, created on
/// first use (php's `create_object`).
pub(crate) fn zip_object(host: &dyn Host, this: &Value) -> Option<Rc<ZipObject>> {
    if !crate::object::has_handlers(host, this, &HANDLERS) {
        return None;
    }
    let h = crate::object::object_of(this)?;
    if let Some(o) = crate::object::payload_of::<ZipObject>(&h) {
        return Some(o);
    }
    let o = Rc::new(ZipObject::new());
    crate::object::set_payload(&h, o.clone());
    Some(o)
}

/// The class name php's messages give an object (`object->ce->name`).
fn class_of(host: &dyn Host, this: &Value) -> String {
    host.object_class_name(this)
        .map(|n| String::from_utf8_lossy(&n).into_owned())
        .unwrap_or_else(|| "ZipArchive".into())
}

// --- properties --------------------------------------------------------------

/// php's `zip_prop_handlers`, in registration order.
const PROPS: [&str; 6] = [
    "lastId",
    "status",
    "statusSys",
    "numFiles",
    "filename",
    "comment",
];

/// php_zip_property_reader
fn read_prop(o: &ZipObject, name: &str) -> Value {
    match name {
        "lastId" => Value::Int(o.last_id.get()),
        "status" => Value::Int(o.status()),
        "statusSys" => Value::Int(o.status_sys()),
        "numFiles" => Value::Int(o.num_files()),
        "filename" => Value::str(o.filename.borrow().clone().unwrap_or_default()),
        _ => Value::str(o.comment()),
    }
}

/// `__zip_prop($op, $this, $name)`: php's read_property (0),
/// write_property (1), has_property (2) and unset (3) for the six
/// properties; null for any other name, which the prelude hands to the
/// standard behaviour.
fn fn_zip_prop(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = rhp_values::conv::to_int(&deref(&args[0]));
    let this = deref(&args[1]);
    let name = rhp_values::conv::to_string_bytes(&deref(&args[2]));
    // Another name: the prelude's __set creates it, and the engine raises
    // the dynamic-property deprecation by the object's class flags.
    let Some(name) = PROPS.iter().find(|p| p.as_bytes() == name.as_slice()) else {
        return Ok(Value::Null);
    };
    let Some(o) = zip_object(host, &this) else {
        return Ok(Value::Null);
    };
    Ok(match op {
        0 => read_prop(&o, name),
        1 => {
            return Err(Thrown::error(
                "Error",
                format!(
                    "Cannot write read-only property {}::${name}",
                    class_of(host, &this)
                ),
            ))
        }
        // `isset`: the value is never null.
        _ => Value::Bool(true),
    })
}

/// php_zip_get_properties: the object's own properties with the six
/// read-only ones before them, for every view.
fn get_properties_for(
    host: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    let o = match crate::object::payload_of::<ZipObject>(h) {
        Some(o) => o,
        None => {
            let o = Rc::new(ZipObject::new());
            crate::object::set_payload(h, o.clone());
            o
        }
    };
    let mut a = PhpArray::new();
    for p in PROPS {
        a.insert(ArrayKey::Str(PhpStr::from(p.as_bytes())), read_prop(&o, p));
    }
    // get_object_vars sees the caller's accessible properties, unmangled.
    let own = match purpose {
        PropPurpose::GetObjectVars => match host.object_vars(&Value::Object(h.clone())) {
            Some(Value::Array(v)) => v,
            _ => host.object_props_mangled(h),
        },
        _ => host.object_props_mangled(h),
    };
    for (k, v) in own.iter() {
        a.insert(k.clone(), v.clone());
    }
    Ok(Some(a))
}

// --- callbacks ---------------------------------------------------------------

/// php_zip_progress_callback
unsafe extern "C" fn progress_callback(_za: *mut sys::zip_t, state: c_double, ud: *mut c_void) {
    ffi::guard((), || {
        let a = unsafe { &*(ud as *const Archive) };
        // zend_call_known_fcc does nothing while an exception is pending.
        if ffi::aborted() {
            return;
        }
        let Some(cb) = a.progress.borrow().clone() else {
            return;
        };
        ffi::call_host(|h| h.call_callable(&cb, &[Value::Float(state)]));
    })
}

unsafe extern "C" fn progress_free(ud: *mut c_void) {
    ffi::guard((), || {
        let a = unsafe { &*(ud as *const Archive) };
        let cb = a.progress.borrow_mut().take();
        drop(cb);
    })
}

/// php_zip_cancel_callback
unsafe extern "C" fn cancel_callback(_za: *mut sys::zip_t, ud: *mut c_void) -> c_int {
    ffi::guard(-1, || {
        let a = unsafe { &*(ud as *const Archive) };
        // Freeing the object (no host) closes the archive without PHP.
        if !ffi::has_host() {
            return 0;
        }
        if ffi::aborted() {
            return -1;
        }
        let Some(cb) = a.cancel.borrow().clone() else {
            return 0;
        };
        ffi::call_host(|h| {
            let r = deref(&h.call_callable(&cb, &[])?);
            match crate::ext::zpp::try_get_long(h, &r)? {
                Some(n) => Ok(n as c_int),
                None => Err(Thrown::type_error(format!(
                    "Return value of callback provided to ZipArchive::registerCancelCallback() \
                     must be of type int, {} returned",
                    crate::ext::zpp::given_type(h, &r)
                ))),
            }
        })
        .unwrap_or(-1)
    })
}

unsafe extern "C" fn cancel_free(ud: *mut c_void) {
    ffi::guard((), || {
        let a = unsafe { &*(ud as *const Archive) };
        let cb = a.cancel.borrow_mut().take();
        drop(cb);
    })
}

// --- constants and the prelude -----------------------------------------------

/// One of ZipArchive's constants, with its PHP attribute ("" for none).
struct Const {
    name: String,
    value: CVal,
    attr: String,
}

enum CVal {
    Int(i64),
    Str(Vec<u8>),
}

unsafe extern "C" fn collect_constant(
    ctx: *mut c_void,
    name: *const c_char,
    ival: c_longlong,
    sval: *const c_char,
    attr: *const c_char,
) {
    ffi::guard((), || {
        let out = unsafe { &mut *(ctx as *mut Vec<Const>) };
        let s = |p| String::from_utf8_lossy(unsafe { sys::cstr(p) }).into_owned();
        out.push(Const {
            name: s(name),
            value: if sval.is_null() {
                CVal::Int(ival)
            } else {
                CVal::Str(unsafe { sys::cstr(sval) }.to_vec())
            },
            attr: s(attr),
        });
    })
}

/// ZipArchive's constants as the linked zip.h defines them (consts.h).
fn constants() -> &'static [Const] {
    static CONSTS: OnceLock<Vec<Const>> = OnceLock::new();
    CONSTS.get_or_init(|| {
        let mut out: Vec<Const> = Vec::new();
        unsafe {
            sys::rhp_zip_constants(collect_constant, (&mut out as *mut Vec<Const>).cast());
        }
        out
    })
}

/// A PHP literal for a constant's value.
fn php_literal(v: &CVal) -> String {
    match v {
        CVal::Int(n) => n.to_string(),
        CVal::Str(s) => {
            let mut out = String::from("'");
            for &b in s {
                match b {
                    b'\'' => out.push_str("\\'"),
                    b'\\' => out.push_str("\\\\"),
                    _ => out.push(b as char),
                }
            }
            out.push('\'');
            out
        }
    }
}

/// The `ZipArchive` class: its constants from the linked libzip, then the
/// methods (prelude.php).
pub fn prelude() -> &'static str {
    static PRELUDE: OnceLock<String> = OnceLock::new();
    PRELUDE.get_or_init(|| {
        let mut consts = String::new();
        for c in constants() {
            if !c.attr.is_empty() {
                consts.push_str("    ");
                consts.push_str(&c.attr);
                consts.push('\n');
            }
            consts.push_str(&format!(
                "    public const {} = {};\n",
                c.name,
                php_literal(&c.value)
            ));
        }
        include_str!("prelude.php").replace("    // %CONSTANTS%\n", &consts)
    })
}

/// The E_DEPRECATED a fetch of a `#[\Deprecated]` ZipArchive constant
/// raises (FL_RECOMPRESS, from the stub's attribute).
pub(crate) fn class_constant_deprecation(class: &[u8], name: &[u8]) -> Option<String> {
    if !class.eq_ignore_ascii_case(b"ZipArchive") {
        return None;
    }
    let c = constants().iter().find(|c| c.name.as_bytes() == name)?;
    let args = c.attr.strip_prefix("#[\\Deprecated(")?.strip_suffix(")]")?;
    let arg = |key: &str| {
        let rest = &args[args.find(&format!("{key}: '"))? + key.len() + 3..];
        Some(rest[..rest.find('\'')?].to_string())
    };
    let mut msg = format!("Constant ZipArchive::{} is deprecated", c.name);
    if let Some(since) = arg("since") {
        msg.push_str(&format!(" since {since}"));
    }
    if let Some(m) = arg("message") {
        msg.push_str(&format!(", {m}"));
    }
    Some(msg)
}

/// The value of ZipArchive constant `name` in the linked libzip.
pub(crate) fn constant(name: &str) -> Option<i64> {
    constants()
        .iter()
        .find(|c| c.name == name)
        .and_then(|c| match c.value {
            CVal::Int(n) => Some(n),
            CVal::Str(_) => None,
        })
}

/// `ZipArchive::LIBZIP_VERSION`: `zip_libzip_version()`, as php reads it.
pub fn libzip_version() -> String {
    String::from_utf8_lossy(unsafe { sys::cstr(sys::zip_libzip_version()) }).into_owned()
}

/// PHP_MINFO_FUNCTION(zip): the rows of phpinfo's zip table.
pub fn info_rows() -> Vec<(String, String)> {
    let supported = |m: Option<i64>, enc: bool| {
        let yes = match m {
            Some(m) if enc => unsafe { sys::zip_encryption_method_supported(m as u16, 1) != 0 },
            Some(m) => unsafe { sys::zip_compression_method_supported(m as i32, 1) != 0 },
            None => false,
        };
        if yes { "Yes" } else { "No" }.to_string()
    };
    let mut rows = vec![
        ("Zip".into(), "enabled".into()),
        ("Zip version".into(), crate::version::EXT_ZIP.into()),
    ];
    let header = env!("RHP_LIBZIP_VERSION");
    let runtime = libzip_version();
    if header != runtime {
        rows.push(("Libzip headers version".into(), header.into()));
        rows.push(("Libzip library version".into(), runtime));
    } else {
        rows.push(("Libzip version".into(), header.into()));
    }
    rows.push((
        "BZIP2 compression".into(),
        supported(constant("CM_BZIP2"), false),
    ));
    rows.push(("XZ compression".into(), supported(constant("CM_XZ"), false)));
    rows.push((
        "ZSTD compression".into(),
        supported(constant("CM_ZSTD"), false),
    ));
    rows.push((
        "AES-128 encryption".into(),
        supported(constant("EM_AES_128"), true),
    ));
    rows.push((
        "AES-192 encryption".into(),
        supported(constant("EM_AES_192"), true),
    ));
    rows.push((
        "AES-256 encryption".into(),
        supported(constant("EM_AES_256"), true),
    ));
    rows
}

// --- shared helpers ----------------------------------------------------------

pub(crate) fn deref(v: &Value) -> Value {
    crate::ext::zpp::deref(v)
}

/// A string argument as C sees php's `char *`: up to the first NUL.
pub(crate) fn c_string(b: &[u8]) -> CString {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    CString::new(&b[..end]).expect("no NUL")
}

/// zpp's `P`/`p`: a path must not contain NUL bytes.
pub(crate) fn check_path(fname: &str, n: usize, pname: &str, b: &[u8]) -> Result<(), Thrown> {
    if b.contains(&0) {
        return Err(crate::ext::zpp::value_error(
            fname,
            n,
            pname,
            "must not contain any null bytes",
        ));
    }
    Ok(())
}

/// zend_argument_must_not_be_empty_error
pub(crate) fn must_not_be_empty(fname: &str, n: usize, pname: &str) -> Thrown {
    crate::ext::zpp::value_error(fname, n, pname, "must not be empty")
}

/// MAXPATHLEN
pub(crate) const MAXPATHLEN: usize = libc::PATH_MAX as usize;

/// php's `expand_filepath`: the absolute path libzip is handed (libzip
/// opens files itself, against the process's directory). `None` for an
/// empty or too long path.
pub(crate) fn expand_filepath(path: &[u8]) -> Option<Vec<u8>> {
    if path.is_empty() {
        return None;
    }
    let p = crate::ext::dom::expand_filepath(path);
    (p.len() < MAXPATHLEN - 1).then_some(p)
}

/// `zip_stat_t` as php's RETURN_SB array.
pub(crate) fn stat_array(sb: &sys::zip_stat_t) -> Value {
    let mut a = PhpArray::new();
    let mut put = |k: &str, v: Value| a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
    put("name", Value::str(unsafe { sys::cstr(sb.name) }.to_vec()));
    put("index", Value::Int(sb.index as i64));
    put("crc", Value::Int(sb.crc as i64));
    put("size", Value::Int(sb.size as i64));
    put("mtime", Value::Int(sb.mtime));
    put("comp_size", Value::Int(sb.comp_size as i64));
    put("comp_method", Value::Int(sb.comp_method as i64));
    put("encryption_method", Value::Int(sb.encryption_method as i64));
    Value::Array(a)
}

/// php_zip_file_error: an entry's error code and message.
pub(crate) fn file_error(zf: *mut sys::zip_file_t) -> (c_int, String) {
    unsafe {
        let e = sys::zip_file_get_error(zf);
        let msg = String::from_utf8_lossy(sys::cstr(sys::zip_error_strerror(e))).into_owned();
        (sys::zip_error_code_zip(e), msg)
    }
}

// --- registration ------------------------------------------------------------

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    for (name, func, min) in [
        ("__zip", archive::fn_zip as crate::NativeFn, 2),
        ("__zip_prop", fn_zip_prop, 3),
    ] {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: None,
            },
        );
    }
    procedural::register(m);
}
