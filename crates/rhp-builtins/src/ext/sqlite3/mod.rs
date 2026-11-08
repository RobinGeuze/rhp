//! The `sqlite3` extension on libsqlite3: a port of ext/sqlite3/sqlite3.c.
//! `SQLite3`, `SQLite3Stmt`, `SQLite3Result` and `SQLite3Exception` are
//! declared in PHP ([`PRELUDE_PHP`]); their methods call `__sqlite3` with
//! `$this`. An object's state (php's `php_sqlite3_db_object`,
//! `php_sqlite3_stmt`, `php_sqlite3_result`) is its payload. It shares
//! pdo_sqlite's declarations of the library ([`sys`]) and the way C
//! callbacks reach PHP ([`ffi`]).

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_int, c_void, CString};
use std::ptr;
use std::rc::{Rc, Weak};

use rhp_values::{conv, ArrayKey, ObjectHandle, PhpArray, Value};

use crate::ext::libxml::ffi;
use crate::ext::pdo::{deref, Cx};
use crate::ext::pdo_sqlite::keep_callback;
use crate::ext::pdo_sqlite::sys::{self, sqlite3, sqlite3_context, sqlite3_stmt, sqlite3_value};
use crate::main::streams::ops::{OpsStream, StreamOps};
use crate::zend::object_handlers::{self, ClassHandlers, ObjectHandlers};
use crate::{Host, StatInfo, Thrown, E_DEPRECATED, E_WARNING};

pub const PRELUDE_PHP: &str = include_str!("prelude.php");

const ASSOC: i64 = 1;
const NUM: i64 = 2;
const BOTH: i64 = 3;

pub fn constants() -> Vec<(&'static str, Value)> {
    let i = |n: i64| Value::Int(n);
    vec![
        ("SQLITE3_ASSOC", i(ASSOC)),
        ("SQLITE3_NUM", i(NUM)),
        ("SQLITE3_BOTH", i(BOTH)),
        ("SQLITE3_INTEGER", i(sys::SQLITE_INTEGER as i64)),
        ("SQLITE3_FLOAT", i(sys::SQLITE_FLOAT as i64)),
        ("SQLITE3_TEXT", i(sys::SQLITE_TEXT as i64)),
        ("SQLITE3_BLOB", i(sys::SQLITE_BLOB as i64)),
        ("SQLITE3_NULL", i(sys::SQLITE_NULL as i64)),
        ("SQLITE3_OPEN_READONLY", i(sys::SQLITE_OPEN_READONLY as i64)),
        (
            "SQLITE3_OPEN_READWRITE",
            i(sys::SQLITE_OPEN_READWRITE as i64),
        ),
        ("SQLITE3_OPEN_CREATE", i(sys::SQLITE_OPEN_CREATE as i64)),
        ("SQLITE3_DETERMINISTIC", i(2048)),
    ]
}

pub fn register(m: &mut crate::Registry) {
    crate::register_helpers(m, &[("__sqlite3", fn_sqlite3, 2, None)]);
}

pub(crate) static CLASSES: &[ClassHandlers] = &[
    ClassHandlers {
        class: "SQLite3",
        handlers: &DB_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "SQLite3Stmt",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "SQLite3Result",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
];

/// sqlite3_object_handlers
static DB_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_gc: Some(db_get_gc),
    ..ObjectHandlers::UNCLONEABLE
};

/// php_sqlite3_get_gc: the callbacks.
fn db_get_gc(h: &ObjectHandle, out: &mut Vec<Value>) {
    if let Some(d) = object_handlers::payload_of::<Db>(h) {
        if let Some(a) = d.authorizer.borrow().clone() {
            out.push(a);
        }
        for f in d.funcs.borrow().iter() {
            out.extend([&f.func, &f.step, &f.fini].into_iter().flatten().cloned());
        }
        for c in d.collations.borrow().iter() {
            out.push(c.cmp.clone());
        }
    }
}

// --- state -------------------------------------------------------------------

/// `php_sqlite3_func`
struct Func {
    name: CString,
    argc: c_int,
    func: Option<Value>,
    step: Option<Value>,
    fini: Option<Value>,
    db: *const Db,
}

/// `php_sqlite3_collation`
struct Collation {
    name: CString,
    cmp: Value,
    db: *const Db,
}

/// `php_sqlite3_db_object`
struct Db {
    db: Cell<*mut sqlite3>,
    initialised: Cell<bool>,
    exception: Cell<bool>,
    /// Boxed: sqlite keeps a pointer to each as its user data.
    #[allow(clippy::vec_box)]
    funcs: RefCell<Vec<Box<Func>>>,
    #[allow(clippy::vec_box)]
    collations: RefCell<Vec<Box<Collation>>>,
    authorizer: RefCell<Option<Value>>,
    in_callback: Cell<u32>,
    /// The statements `close()` finalizes (`free_list`).
    free_list: RefCell<Vec<Weak<Stmt>>>,
}

impl Default for Db {
    fn default() -> Db {
        Db {
            db: Cell::new(ptr::null_mut()),
            initialised: Cell::new(false),
            exception: Cell::new(false),
            funcs: RefCell::default(),
            collations: RefCell::default(),
            authorizer: RefCell::default(),
            in_callback: Cell::new(0),
            free_list: RefCell::default(),
        }
    }
}

impl Db {
    /// Whether running SQL may call PHP, so it needs the host.
    fn calls_php(&self) -> bool {
        !self.funcs.borrow().is_empty()
            || !self.collations.borrow().is_empty()
            || self.authorizer.borrow().is_some()
            || crate::main::fopen_wrappers::active()
    }

    /// Run `f` (sqlite code that may call back into PHP) with the host
    /// reachable when it may.
    fn run<R>(&self, cx: &mut Cx, f: impl FnOnce() -> R) -> R {
        if !self.calls_php() {
            return f();
        }
        let db = self.db.get();
        cx.ffi(move || unsafe { sys::sqlite3_interrupt(db) }, f)
    }

    fn errmsg(&self) -> String {
        errmsg(self.db.get())
    }

    /// zend_llist_clean(&free_list): finalize every listed statement.
    fn clean_free_list(&self) {
        let list = std::mem::take(&mut *self.free_list.borrow_mut());
        for s in list.iter().filter_map(Weak::upgrade) {
            s.finalize();
        }
    }

    /// zend_llist_del_element(free_list, stmt): finalize it if listed.
    fn del_stmt(&self, stmt: &Stmt) {
        let mut found = false;
        self.free_list.borrow_mut().retain(|w| {
            let hit = !found && ptr::eq(w.as_ptr(), stmt) && stmt.initialised.get();
            found |= hit;
            !hit && w.strong_count() > 0
        });
        if found {
            stmt.finalize();
        }
    }
}

impl Drop for Db {
    /// php_sqlite3_object_free_storage
    fn drop(&mut self) {
        let db = self.db.get();
        let open = self.initialised.get() && !db.is_null();
        for f in self.funcs.get_mut().drain(..) {
            if open {
                unsafe {
                    sys::sqlite3_create_function(
                        db,
                        f.name.as_ptr(),
                        f.argc,
                        sys::SQLITE_UTF8,
                        &*f as *const Func as *mut c_void,
                        None,
                        None,
                        None,
                    )
                };
            }
        }
        for c in self.collations.get_mut().drain(..) {
            if open {
                unsafe {
                    sys::sqlite3_create_collation(
                        db,
                        c.name.as_ptr(),
                        sys::SQLITE_UTF8,
                        ptr::null_mut(),
                        None,
                    )
                };
            }
        }
        if open {
            unsafe { sys::sqlite3_close_v2(db) };
            self.initialised.set(false);
        }
    }
}

/// `struct php_sqlite3_bound_param`
#[derive(Clone)]
struct Param {
    number: i64,
    name: Option<Vec<u8>>,
    typ: i64,
    value: Value,
}

/// The key a bound parameter is stored under: its name, else its number.
#[derive(Clone, PartialEq)]
enum Key {
    Num(i64),
    Name(Vec<u8>),
}

/// `php_sqlite3_stmt`
struct Stmt {
    stmt: Cell<*mut sqlite3_stmt>,
    db: RefCell<Option<Rc<Db>>>,
    /// The SQLite3 object, kept alive while the statement is.
    db_obj: RefCell<Option<Value>>,
    initialised: Cell<bool>,
    bound: RefCell<Option<Vec<(Key, Param)>>>,
}

impl Default for Stmt {
    fn default() -> Stmt {
        Stmt {
            stmt: Cell::new(ptr::null_mut()),
            db: RefCell::default(),
            db_obj: RefCell::default(),
            initialised: Cell::new(false),
            bound: RefCell::default(),
        }
    }
}

impl Stmt {
    fn db(&self) -> Option<Rc<Db>> {
        self.db.borrow().clone()
    }

    /// php_sqlite3_free_list_dtor
    fn finalize(&self) {
        if self.initialised.get() {
            unsafe { sys::sqlite3_finalize(self.stmt.get()) };
            self.initialised.set(false);
        }
    }

    fn handle(&self) -> *mut sqlite3 {
        unsafe { sys::sqlite3_db_handle(self.stmt.get()) }
    }
}

impl Drop for Stmt {
    /// php_sqlite3_stmt_object_free_storage
    fn drop(&mut self) {
        self.bound.get_mut().take();
        if self.initialised.get() {
            if let Some(db) = self.db.get_mut().clone() {
                db.free_list
                    .borrow_mut()
                    .retain(|w| w.strong_count() > 0 && !ptr::eq(w.as_ptr(), self));
            }
            self.finalize();
        }
    }
}

/// `php_sqlite3_result`
#[derive(Default)]
struct ResultSet {
    db: RefCell<Option<Rc<Db>>>,
    stmt: RefCell<Option<Rc<Stmt>>>,
    /// The SQLite3Stmt object (query() makes a hidden one), kept alive.
    stmt_obj: RefCell<Option<Value>>,
    is_prepared: Cell<bool>,
    column_count: Cell<i32>,
    column_names: RefCell<Option<Vec<ArrayKey>>>,
}

impl ResultSet {
    /// sqlite3result_clear_column_names_cache
    fn clear_cache(&self) {
        self.column_names.borrow_mut().take();
        self.column_count.set(-1);
    }
}

impl Drop for ResultSet {
    /// php_sqlite3_result_object_free_storage
    fn drop(&mut self) {
        if let Some(s) = self.stmt.get_mut().take() {
            if s.initialised.get() {
                unsafe { sys::sqlite3_reset(s.stmt.get()) };
            }
        }
    }
}

fn payload<T: Default + 'static>(this: &Value) -> Option<Rc<T>> {
    let h = object_handlers::object_of(this)?;
    if let Some(p) = object_handlers::payload_of::<T>(&h) {
        return Some(p);
    }
    let p = Rc::new(T::default());
    object_handlers::set_payload(&h, p.clone());
    Some(p)
}

fn errmsg(db: *mut sqlite3) -> String {
    String::from_utf8_lossy(unsafe { sys::cstr(sys::sqlite3_errmsg(db)) }).into_owned()
}

fn c_string(b: &[u8]) -> CString {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    CString::new(&b[..end]).unwrap_or_default()
}

// --- errors ------------------------------------------------------------------

/// SQLITE3_CHECK_INITIALIZED's error.
fn not_initialised(class: &str) -> Thrown {
    Thrown::error(
        "Error",
        format!("The {class} object has not been correctly initialised or is already closed"),
    )
}

/// zend_throw_exception(ce, message, code): no constructor runs.
fn exception(host: &mut dyn Host, class: &[u8], message: &str, code: i64) -> Thrown {
    let obj = match host.reflect_new_instance(class, Vec::new(), false) {
        Ok(o) => o,
        Err(t) => return t,
    };
    let r = (|| {
        host.reflect_set_prop(&obj, b"Exception", b"message", Value::str(message))?;
        host.reflect_set_prop(&obj, b"Exception", b"code", Value::Int(code))
    })();
    if let Err(t) = r {
        return t;
    }
    match obj {
        Value::Object(h) => Thrown::Object(h),
        _ => Thrown::error("Exception", message.to_string()),
    }
}

/// php_sqlite3_error: a SQLite3Exception in exception mode, else a warning.
fn error(cx: &mut Cx, db: Option<&Db>, code: c_int, msg: &str) -> Result<(), Thrown> {
    if db.is_some_and(|d| d.exception.get()) {
        let t = exception(cx.host, b"SQLite3Exception", msg, code as i64);
        // A second exception chains the pending one as its previous.
        return Err(match cx.pending.take() {
            Some(p) => cx.host.chain_thrown(t, p),
            None => t,
        });
    }
    cx.warn(msg)
}

/// php_sqlite3_error from inside a sqlite callback.
fn callback_error(db: Option<&Db>, msg: &str) {
    let exc = db.is_some_and(|d| d.exception.get());
    let fname = ffi::fname();
    ffi::call_host(|h| {
        if exc {
            return Err(exception(h, b"SQLite3Exception", msg, 0));
        }
        h.emit_diagnostic(E_WARNING, format!("{fname}(): {msg}").as_bytes())
    });
}

// --- dispatch ----------------------------------------------------------------

fn arg(a: &[Value], i: usize) -> Value {
    a.get(i).map(deref).unwrap_or(Value::Null)
}

fn bytes(a: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&arg(a, i))
}

/// `__sqlite3($method, $this, ...$args)`.
fn fn_sqlite3(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let method = String::from_utf8_lossy(&conv::to_string_bytes(&deref(&args[0]))).into_owned();
    let this = deref(&args[1]);
    let a = &args[2..];
    let fname = match method.split_once("::") {
        Some((class, m)) => format!("SQLite3{class}::{m}"),
        None => format!("SQLite3::{method}"),
    };
    let mut cx = Cx::new(host, fname);
    let r = match method.split_once("::") {
        Some(("Stmt", m)) => stmt_method(&mut cx, m, &this, a),
        Some((_, m)) => result_method(&mut cx, m, &this, a),
        None => db_method(&mut cx, &method, &this, a),
    };
    cx.finish(r)
}

fn db_of(this: &Value) -> Result<Rc<Db>, Thrown> {
    payload::<Db>(this).ok_or_else(|| not_initialised("SQLite3"))
}

/// SQLITE3_CHECK_INITIALIZED(db_obj, db_obj->initialised, SQLite3)
fn open_db(this: &Value) -> Result<Rc<Db>, Thrown> {
    let d = db_of(this)?;
    if !d.initialised.get() {
        return Err(not_initialised("SQLite3"));
    }
    Ok(d)
}

/// SQLITE3_CHECK_INITIALIZED(db_obj, db_obj->db, SQLite3)
fn has_handle(this: &Value) -> Result<Rc<Db>, Thrown> {
    let d = db_of(this)?;
    if d.db.get().is_null() {
        return Err(not_initialised("SQLite3"));
    }
    Ok(d)
}

fn no_nul(cx: &Cx, b: &[u8], n: usize, pname: &str) -> Result<(), Thrown> {
    if b.contains(&0) {
        return Err(cx.value_error(n, pname, "must not contain any null bytes"));
    }
    Ok(())
}

/// Z_PARAM_FUNC(_OR_NULL) of argument `i`.
fn check_callback(
    cx: &mut Cx,
    a: &[Value],
    i: usize,
    pname: &str,
    or_null: bool,
) -> Result<(), Thrown> {
    let cb = arg(a, i);
    if or_null && matches!(cb, Value::Null) {
        return Ok(());
    }
    if let Some(why) = cx.host.check_callable(&cb)? {
        return Err(Thrown::type_error(format!(
            "{}(): Argument #{} (${pname}) must be a valid callback{}, {why}",
            cx.fname,
            i + 1,
            if or_null { " or null" } else { "" }
        )));
    }
    Ok(())
}

fn db_method(cx: &mut Cx, method: &str, this: &Value, a: &[Value]) -> Result<Value, Thrown> {
    let b = Value::Bool;
    match method {
        "__construct" | "open" => open(cx, this, a).map(|_| Value::Null),
        "close" => {
            let d = db_of(this)?;
            if d.initialised.get() {
                if d.in_callback.get() > 0 {
                    return Err(Thrown::error(
                        "Error",
                        "Cannot close SQLite3 database while inside a callback",
                    ));
                }
                d.clean_free_list();
                let db = d.db.get();
                if !db.is_null() {
                    let rc = unsafe { sys::sqlite3_close(db) };
                    if rc != sys::SQLITE_OK {
                        let m = format!("Unable to close database: {}", d.errmsg());
                        error(cx, Some(&d), rc, &m)?;
                        return Ok(b(false));
                    }
                }
                d.initialised.set(false);
            }
            Ok(b(true))
        }
        "exec" => {
            let d = open_db(this)?;
            let sql = c_string(&bytes(a, 0));
            let mut err: *mut c_char = ptr::null_mut();
            let rc = d.run(cx, || unsafe {
                sys::sqlite3_exec(
                    d.db.get(),
                    sql.as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    &mut err,
                )
            });
            if rc != sys::SQLITE_OK {
                let m = String::from_utf8_lossy(unsafe { sys::cstr(err) }).into_owned();
                unsafe { sys::sqlite3_free(err.cast()) };
                error(cx, Some(&d), rc, &m)?;
                return Ok(b(false));
            }
            Ok(b(true))
        }
        "version" => {
            let mut out = PhpArray::new();
            out.insert(
                ArrayKey::from_bytes(b"versionString"),
                Value::str(sys::libversion()),
            );
            out.insert(
                ArrayKey::from_bytes(b"versionNumber"),
                Value::Int(unsafe { sys::sqlite3_libversion_number() } as i64),
            );
            Ok(Value::Array(out))
        }
        "lastInsertRowID" => {
            let d = open_db(this)?;
            Ok(Value::Int(unsafe {
                sys::sqlite3_last_insert_rowid(d.db.get())
            }))
        }
        "lastErrorCode" | "lastExtendedErrorCode" => {
            let d = has_handle(this)?;
            if !d.initialised.get() {
                return Ok(Value::Int(0));
            }
            let db = d.db.get();
            Ok(Value::Int(if method == "lastErrorCode" {
                unsafe { sys::sqlite3_errcode(db) as i64 }
            } else {
                unsafe { sys::sqlite3_extended_errcode(db) as i64 }
            }))
        }
        "enableExtendedResultCodes" => {
            let d = has_handle(this)?;
            if d.initialised.get() {
                let on = conv::to_bool(&arg(a, 0)) as c_int;
                if unsafe { sys::sqlite3_extended_result_codes(d.db.get(), on) } == sys::SQLITE_OK {
                    return Ok(b(true));
                }
            }
            Ok(b(false))
        }
        "lastErrorMsg" => {
            let d = has_handle(this)?;
            if !d.initialised.get() {
                return Ok(Value::str(""));
            }
            Ok(Value::str(unsafe {
                sys::cstr(sys::sqlite3_errmsg(d.db.get()))
            }))
        }
        "busyTimeout" => {
            let d = open_db(this)?;
            let ms = conv::to_int(&arg(a, 0)).clamp(i32::MIN as i64, i32::MAX as i64);
            unsafe { sys::sqlite3_busy_timeout(d.db.get(), ms as c_int) };
            Ok(b(true))
        }
        "loadExtension" => load_extension(cx, this, a),
        "changes" => {
            let d = open_db(this)?;
            Ok(Value::Int(
                unsafe { sys::sqlite3_changes(d.db.get()) } as i64
            ))
        }
        "escapeString" => {
            // sqlite3_mprintf("%q"): the C string, its quotes doubled.
            let s = bytes(a, 0);
            let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
            let mut out = Vec::with_capacity(end);
            for &c in &s[..end] {
                out.push(c);
                if c == b'\'' {
                    out.push(c);
                }
            }
            Ok(Value::str(out))
        }
        "prepare" => {
            let d = open_db(this)?;
            let sql = bytes(a, 0);
            if sql.is_empty() {
                return Ok(b(false));
            }
            let obj = cx
                .host
                .reflect_new_instance(b"SQLite3Stmt", Vec::new(), false)?;
            let s = payload::<Stmt>(&obj).ok_or_else(|| not_initialised("SQLite3Stmt"))?;
            if !prepare_into(cx, &d, this, &s, &sql)? {
                return Ok(b(false));
            }
            Ok(obj)
        }
        "query" => query(cx, this, a),
        "querySingle" => query_single(cx, this, a),
        "createFunction" => {
            check_callback(cx, a, 1, "callback", false)?;
            let d = open_db(this)?;
            let name = bytes(a, 0);
            if name.is_empty() {
                return Ok(b(false));
            }
            let f = Box::new(Func {
                name: c_string(&name),
                argc: conv::to_int(&arg(a, 2)) as c_int,
                func: Some(keep_callback(&*cx.host, arg(a, 1))),
                step: None,
                fini: None,
                db: Rc::as_ptr(&d),
            });
            let flags = conv::to_int(&arg(a, 3)) as c_int;
            let rc = unsafe {
                sys::sqlite3_create_function(
                    d.db.get(),
                    f.name.as_ptr(),
                    f.argc,
                    flags | sys::SQLITE_UTF8,
                    &*f as *const Func as *mut c_void,
                    Some(func_callback),
                    None,
                    None,
                )
            };
            if rc == sys::SQLITE_OK {
                d.funcs.borrow_mut().push(f);
                return Ok(b(true));
            }
            Ok(b(false))
        }
        "createAggregate" => {
            check_callback(cx, a, 1, "stepCallback", false)?;
            check_callback(cx, a, 2, "finalCallback", false)?;
            let d = open_db(this)?;
            let name = bytes(a, 0);
            if name.is_empty() {
                return Ok(b(false));
            }
            let f = Box::new(Func {
                name: c_string(&name),
                argc: conv::to_int(&arg(a, 3)) as c_int,
                func: None,
                step: Some(keep_callback(&*cx.host, arg(a, 1))),
                fini: Some(keep_callback(&*cx.host, arg(a, 2))),
                db: Rc::as_ptr(&d),
            });
            let rc = unsafe {
                sys::sqlite3_create_function(
                    d.db.get(),
                    f.name.as_ptr(),
                    f.argc,
                    sys::SQLITE_UTF8,
                    &*f as *const Func as *mut c_void,
                    None,
                    Some(step_callback),
                    Some(final_callback),
                )
            };
            if rc == sys::SQLITE_OK {
                d.funcs.borrow_mut().push(f);
                return Ok(b(true));
            }
            Ok(b(false))
        }
        "createCollation" => {
            check_callback(cx, a, 1, "callback", false)?;
            let d = open_db(this)?;
            let name = bytes(a, 0);
            if name.is_empty() {
                return Ok(b(false));
            }
            let c = Box::new(Collation {
                name: c_string(&name),
                cmp: keep_callback(&*cx.host, arg(a, 1)),
                db: Rc::as_ptr(&d),
            });
            let rc = unsafe {
                sys::sqlite3_create_collation(
                    d.db.get(),
                    c.name.as_ptr(),
                    sys::SQLITE_UTF8,
                    &*c as *const Collation as *mut c_void,
                    Some(collation_callback),
                )
            };
            if rc == sys::SQLITE_OK {
                d.collations.borrow_mut().push(c);
                return Ok(b(true));
            }
            Ok(b(false))
        }
        "openBlob" => open_blob(cx, this, a),
        "enableExceptions" => {
            let d = db_of(this)?;
            let was = d.exception.get();
            let on = conv::to_bool(&arg(a, 0));
            if !on {
                let m = format!("{}(): Use of warnings for SQLite3 is deprecated", cx.fname);
                cx.host.emit_diagnostic(E_DEPRECATED, m.as_bytes())?;
            }
            d.exception.set(on);
            Ok(b(was))
        }
        "setAuthorizer" => {
            check_callback(cx, a, 0, "callback", true)?;
            let d = open_db(this)?;
            *d.authorizer.borrow_mut() = match arg(a, 0) {
                Value::Null => None,
                v => Some(keep_callback(&*cx.host, v)),
            };
            Ok(b(true))
        }
        "backup" => backup(cx, this, a),
        _ => Err(Thrown::error(
            "Error",
            format!("Call to undefined method {}()", cx.fname),
        )),
    }
}

/// SQLite3::open (and the constructor)
fn open(cx: &mut Cx, this: &Value, a: &[Value]) -> Result<(), Thrown> {
    let d = db_of(this)?;
    let filename = bytes(a, 0);
    no_nul(cx, &filename, 1, "filename")?;
    let flags = conv::to_int(&arg(a, 1)) as c_int;
    if d.initialised.get() {
        return Err(Thrown::error("Exception", "Already initialised DB Object"));
    }
    let path = if !filename.is_empty() && filename != b":memory:" {
        let full = crate::zend::virtual_cwd::expand_filepath(&filename);
        if full.is_empty() {
            return Err(Thrown::error("Exception", "Unable to expand filepath"));
        }
        // php_check_open_basedir: its warning, then the exception.
        if let Some(m) = crate::main::fopen_wrappers::refusal(&full) {
            cx.warn(&m)?;
            return Err(Thrown::error(
                "Exception",
                format!(
                    "open_basedir prohibits opening {}",
                    String::from_utf8_lossy(&full)
                ),
            ));
        }
        full
    } else {
        filename
    };
    let cpath = c_string(&path);
    let mut db: *mut sqlite3 = ptr::null_mut();
    let rc = unsafe { sys::sqlite3_open_v2(cpath.as_ptr(), &mut db, flags, ptr::null()) };
    if rc != sys::SQLITE_OK {
        let msg = if db.is_null() {
            String::from_utf8_lossy(unsafe { sys::cstr(sys::sqlite3_errstr(rc)) }).into_owned()
        } else {
            errmsg(db)
        };
        unsafe { sys::sqlite3_close(db) };
        return Err(Thrown::error(
            "Exception",
            format!("Unable to open database: {msg}"),
        ));
    }
    d.db.set(db);
    d.initialised.set(true);
    *d.authorizer.borrow_mut() = None;
    unsafe {
        sys::sqlite3_set_authorizer(db, Some(authorizer), Rc::as_ptr(&d) as *mut c_void);
    }
    let defensive = cx
        .host
        .ini_get(b"sqlite3.defensive")
        .is_some_and(|v| conv::to_bool(&Value::str(v)));
    if defensive {
        unsafe {
            sys::sqlite3_db_config(
                db,
                sys::SQLITE_DBCONFIG_DEFENSIVE,
                1 as c_int,
                ptr::null_mut::<c_int>(),
            )
        };
    }
    Ok(())
}

/// SQLite3::loadExtension
fn load_extension(cx: &mut Cx, this: &Value, a: &[Value]) -> Result<Value, Thrown> {
    let d = open_db(this)?;
    let ext = bytes(a, 0);
    let dir = cx
        .host
        .ini_get(b"sqlite3.extension_dir")
        .filter(|v| !v.is_empty());
    let Some(dir) = dir else {
        error(cx, Some(&d), 0, "SQLite Extensions are disabled")?;
        return Ok(Value::Bool(false));
    };
    if ext.is_empty() {
        return Err(cx.value_error(1, "name", "must not be empty"));
    }
    let mut lib = dir.clone();
    if dir.last() != Some(&b'/') {
        lib.push(b'/');
    }
    lib.extend_from_slice(&ext);
    use std::os::unix::ffi::OsStrExt;
    let Ok(full) = std::fs::canonicalize(std::ffi::OsStr::from_bytes(&lib)) else {
        let m = format!(
            "Unable to load extension at '{}'",
            String::from_utf8_lossy(&lib)
        );
        error(cx, Some(&d), 0, &m)?;
        return Ok(Value::Bool(false));
    };
    let full = full.as_os_str().as_bytes().to_vec();
    if !full.starts_with(&dir) {
        error(
            cx,
            Some(&d),
            0,
            "Unable to open extensions outside the defined directory",
        )?;
        return Ok(Value::Bool(false));
    }
    let path = c_string(&full);
    let db = d.db.get();
    let mut err: *mut c_char = ptr::null_mut();
    unsafe {
        sys::sqlite3_enable_load_extension(db, 1);
        let rc = sys::sqlite3_load_extension(db, path.as_ptr(), ptr::null(), &mut err);
        if rc != sys::SQLITE_OK {
            let m = String::from_utf8_lossy(sys::cstr(err)).into_owned();
            sys::sqlite3_free(err.cast());
            sys::sqlite3_enable_load_extension(db, 0);
            error(cx, Some(&d), 0, &m)?;
            return Ok(Value::Bool(false));
        }
        sys::sqlite3_enable_load_extension(db, 0);
    }
    Ok(Value::Bool(true))
}

/// Prepare `sql` into `s` for the connection `this` (SQLite3::prepare,
/// SQLite3Stmt::__construct). False after the error was raised.
fn prepare_into(
    cx: &mut Cx,
    d: &Rc<Db>,
    this: &Value,
    s: &Rc<Stmt>,
    sql: &[u8],
) -> Result<bool, Thrown> {
    *s.db.borrow_mut() = Some(d.clone());
    *s.db_obj.borrow_mut() = Some(this.clone());
    let mut st: *mut sqlite3_stmt = ptr::null_mut();
    let rc = d.run(cx, || unsafe {
        sys::sqlite3_prepare_v2(
            d.db.get(),
            sql.as_ptr().cast(),
            sql.len() as c_int,
            &mut st,
            ptr::null_mut(),
        )
    });
    if rc != sys::SQLITE_OK {
        let m = format!("Unable to prepare statement: {}", d.errmsg());
        error(cx, Some(d), rc, &m)?;
        return Ok(false);
    }
    s.stmt.set(st);
    s.initialised.set(true);
    d.free_list.borrow_mut().push(Rc::downgrade(s));
    Ok(true)
}

/// A new SQLite3Result object.
fn new_result(
    cx: &mut Cx,
    d: &Rc<Db>,
    s: &Rc<Stmt>,
    stmt_obj: Value,
    is_prepared: bool,
) -> Result<Value, Thrown> {
    let obj = cx
        .host
        .reflect_new_instance(b"SQLite3Result", Vec::new(), false)?;
    let r = payload::<ResultSet>(&obj).ok_or_else(|| not_initialised("SQLite3Result"))?;
    *r.db.borrow_mut() = Some(d.clone());
    *r.stmt.borrow_mut() = Some(s.clone());
    r.is_prepared.set(is_prepared);
    *r.stmt_obj.borrow_mut() = Some(stmt_obj);
    r.column_count.set(-1);
    Ok(obj)
}

/// SQLite3::exec's path for query() and querySingle() whose result is
/// unused.
fn exec_unused(cx: &mut Cx, d: &Db, sql: &[u8]) -> Result<Value, Thrown> {
    let sql = c_string(sql);
    let mut err: *mut c_char = ptr::null_mut();
    let rc = d.run(cx, || unsafe {
        sys::sqlite3_exec(
            d.db.get(),
            sql.as_ptr(),
            ptr::null(),
            ptr::null_mut(),
            &mut err,
        )
    });
    if rc != sys::SQLITE_OK {
        let m = String::from_utf8_lossy(unsafe { sys::cstr(err) }).into_owned();
        unsafe { sys::sqlite3_free(err.cast()) };
        error(cx, Some(d), rc, &m)?;
    }
    Ok(Value::Bool(false))
}

/// SQLite3::query
fn query(cx: &mut Cx, this: &Value, a: &[Value]) -> Result<Value, Thrown> {
    let d = open_db(this)?;
    let sql = bytes(a, 0);
    if sql.is_empty() {
        return Ok(Value::Bool(false));
    }
    if cx.host.result_unused() {
        return exec_unused(cx, &d, &sql);
    }
    // php allocates a SQLite3Stmt object here too (object handles).
    let stmt_obj = cx
        .host
        .reflect_new_instance(b"SQLite3Stmt", Vec::new(), false)?;
    let s = payload::<Stmt>(&stmt_obj).ok_or_else(|| not_initialised("SQLite3Stmt"))?;
    *s.db.borrow_mut() = Some(d.clone());
    *s.db_obj.borrow_mut() = Some(this.clone());
    let mut st: *mut sqlite3_stmt = ptr::null_mut();
    let rc = d.run(cx, || unsafe {
        sys::sqlite3_prepare_v2(
            d.db.get(),
            sql.as_ptr().cast(),
            sql.len() as c_int,
            &mut st,
            ptr::null_mut(),
        )
    });
    if rc != sys::SQLITE_OK {
        let m = format!("Unable to prepare statement: {}", d.errmsg());
        error(cx, Some(&d), rc, &m)?;
        return Ok(Value::Bool(false));
    }
    s.stmt.set(st);
    s.initialised.set(true);
    let result = new_result(cx, &d, &s, stmt_obj, false)?;
    let rc = d.run(cx, || unsafe { sys::sqlite3_step(st) });
    match rc {
        sys::SQLITE_ROW | sys::SQLITE_DONE => {
            d.free_list.borrow_mut().push(Rc::downgrade(&s));
            unsafe { sys::sqlite3_reset(st) };
            Ok(result)
        }
        _ => {
            let r = if !cx.exception() {
                let m = format!("Unable to execute statement: {}", d.errmsg());
                error(
                    cx,
                    Some(&d),
                    unsafe { sys::sqlite3_errcode(d.db.get()) },
                    &m,
                )
            } else {
                Ok(())
            };
            s.finalize();
            drop(result);
            r.map(|_| Value::Bool(false))
        }
    }
}

/// sqlite_value_to_zval
fn column_value(cx: &mut Cx, db: Option<&Db>, st: *mut sqlite3_stmt, i: c_int) -> Value {
    unsafe {
        match sys::sqlite3_column_type(st, i) {
            sys::SQLITE_INTEGER => Value::Int(sys::sqlite3_column_int64(st, i)),
            sys::SQLITE_FLOAT => Value::Float(sys::sqlite3_column_double(st, i)),
            sys::SQLITE_NULL => Value::Null,
            sys::SQLITE_TEXT => {
                let p = sys::sqlite3_column_text(st, i);
                if p.is_null() {
                    let _ = error(
                        cx,
                        db,
                        sys::SQLITE_NOMEM,
                        "Failed to retrieve column value due to out of memory",
                    );
                    return Value::Null;
                }
                Value::str(sys::cstr(p.cast()))
            }
            _ => {
                let p = sys::sqlite3_column_blob(st, i);
                if p.is_null() {
                    return Value::str("");
                }
                let n = sys::sqlite3_column_bytes(st, i).max(0) as usize;
                Value::str(std::slice::from_raw_parts(p as *const u8, n))
            }
        }
    }
}

/// SQLite3::querySingle
fn query_single(cx: &mut Cx, this: &Value, a: &[Value]) -> Result<Value, Thrown> {
    let d = open_db(this)?;
    let sql = bytes(a, 0);
    let entire_row = conv::to_bool(&arg(a, 1));
    if sql.is_empty() {
        return Ok(Value::Bool(false));
    }
    if cx.host.result_unused() {
        return exec_unused(cx, &d, &sql);
    }
    let mut st: *mut sqlite3_stmt = ptr::null_mut();
    let rc = d.run(cx, || unsafe {
        sys::sqlite3_prepare_v2(
            d.db.get(),
            sql.as_ptr().cast(),
            sql.len() as c_int,
            &mut st,
            ptr::null_mut(),
        )
    });
    if rc != sys::SQLITE_OK {
        let m = format!("Unable to prepare statement: {}", d.errmsg());
        error(cx, Some(&d), rc, &m)?;
        return Ok(Value::Bool(false));
    }
    let rc = d.run(cx, || unsafe { sys::sqlite3_step(st) });
    let r = match rc {
        sys::SQLITE_ROW => {
            if !entire_row {
                Ok(column_value(cx, Some(&d), st, 0))
            } else {
                let n = unsafe { sys::sqlite3_data_count(st) };
                let mut out = PhpArray::new();
                for i in 0..n {
                    let v = column_value(cx, Some(&d), st, i);
                    let name = unsafe { sys::cstr(sys::sqlite3_column_name(st, i)) };
                    out.insert(ArrayKey::Str(name.into()), v);
                }
                Ok(Value::Array(out))
            }
        }
        sys::SQLITE_DONE => Ok(if entire_row {
            Value::Array(PhpArray::new())
        } else {
            Value::Null
        }),
        _ => {
            if !cx.exception() {
                let m = format!("Unable to execute statement: {}", d.errmsg());
                let code = unsafe { sys::sqlite3_errcode(d.db.get()) };
                if let Err(t) = error(cx, Some(&d), code, &m) {
                    unsafe { sys::sqlite3_finalize(st) };
                    return Err(t);
                }
            }
            Ok(Value::Bool(false))
        }
    };
    unsafe { sys::sqlite3_finalize(st) };
    r
}

/// SQLite3::backup
fn backup(cx: &mut Cx, this: &Value, a: &[Value]) -> Result<Value, Thrown> {
    let src_name = bytes(a, 1);
    no_nul(cx, &src_name, 2, "sourceDatabase")?;
    let dst_name = bytes(a, 2);
    no_nul(cx, &dst_name, 3, "destinationDatabase")?;
    let src = open_db(this)?;
    let dst = open_db(&arg(a, 0))?;
    let (sn, dn) = (c_string(&src_name), c_string(&dst_name));
    let mut rc = unsafe {
        let b = sys::sqlite3_backup_init(dst.db.get(), dn.as_ptr(), src.db.get(), sn.as_ptr());
        if b.is_null() {
            sys::sqlite3_errcode(src.db.get())
        } else {
            while sys::sqlite3_backup_step(b, -1) == sys::SQLITE_OK {}
            sys::sqlite3_backup_finish(b)
        }
    };
    if rc != sys::SQLITE_OK {
        let m = match rc {
            sys::SQLITE_BUSY => "Backup failed: source database is busy".to_string(),
            sys::SQLITE_LOCKED => "Backup failed: source database is locked".to_string(),
            _ => format!("Backup failed: {}", src.errmsg()),
        };
        error(cx, Some(&src), rc, &m)?;
        rc = sys::SQLITE_ERROR;
    }
    Ok(Value::Bool(rc == sys::SQLITE_OK))
}

/// SQLite3::openBlob
fn open_blob(cx: &mut Cx, this: &Value, a: &[Value]) -> Result<Value, Thrown> {
    let dbname = if a.len() > 3 {
        bytes(a, 3)
    } else {
        b"main".to_vec()
    };
    no_nul(cx, &dbname, 4, "database")?;
    let d = open_db(this)?;
    let flags = if a.len() > 4 {
        conv::to_int(&arg(a, 4))
    } else {
        sys::SQLITE_OPEN_READONLY as i64
    };
    let rw = (flags & sys::SQLITE_OPEN_READWRITE as i64 != 0) as c_int;
    let (table, column, dbname) = (
        c_string(&bytes(a, 0)),
        c_string(&bytes(a, 1)),
        c_string(&dbname),
    );
    let rowid = conv::to_int(&arg(a, 2));
    let mut blob: *mut sys::sqlite3_blob = ptr::null_mut();
    let rc = unsafe {
        sys::sqlite3_blob_open(
            d.db.get(),
            dbname.as_ptr(),
            table.as_ptr(),
            column.as_ptr(),
            rowid,
            rw,
            &mut blob,
        )
    };
    if rc != sys::SQLITE_OK {
        let m = format!("Unable to open blob: {}", d.errmsg());
        let code = unsafe { sys::sqlite3_errcode(d.db.get()) };
        error(cx, Some(&d), code, &m)?;
        return Ok(Value::Bool(false));
    }
    let size = unsafe { sys::sqlite3_blob_bytes(blob) }.max(0) as usize;
    let ops = BlobStream {
        blob,
        position: 0,
        size,
        flags,
        _conn: d,
    };
    let mode: &[u8] = if rw != 0 { b"r+b" } else { b"rb" };
    Ok(cx
        .host
        .stream_from_ops(OpsStream::new(Box::new(ops), mode, Vec::new()))
        .unwrap_or(Value::Bool(false)))
}

/// `php_stream_sqlite3_data` with php_stream_sqlite3_ops.
struct BlobStream {
    blob: *mut sys::sqlite3_blob,
    position: usize,
    size: usize,
    flags: i64,
    /// The connection stays open while the blob does.
    _conn: Rc<Db>,
}

impl StreamOps for BlobStream {
    fn label(&self) -> &'static str {
        "SQLite3"
    }

    fn read(&mut self, buf: &mut [u8], eof: &mut bool, _diags: &mut Vec<(i64, String)>) -> isize {
        let mut count = buf.len();
        if self.position + count >= self.size {
            count = self.size - self.position;
            *eof = true;
        }
        if count > 0 {
            let rc = unsafe {
                sys::sqlite3_blob_read(
                    self.blob,
                    buf.as_mut_ptr().cast(),
                    count as c_int,
                    self.position as c_int,
                )
            };
            if rc != sys::SQLITE_OK {
                return -1;
            }
            self.position += count;
        }
        count as isize
    }

    fn write(&mut self, data: &[u8]) -> isize {
        use crate::main::streams::plain_wrapper::push_diag;
        if self.flags & sys::SQLITE_OPEN_READONLY as i64 != 0 {
            push_diag(
                E_WARNING,
                true,
                "Can't write to blob stream: is open as read only",
            );
            return -1;
        }
        if self.position + data.len() > self.size {
            push_diag(
                E_WARNING,
                true,
                "It is not possible to increase the size of a BLOB",
            );
            return -1;
        }
        let rc = unsafe {
            sys::sqlite3_blob_write(
                self.blob,
                data.as_ptr().cast(),
                data.len() as c_int,
                self.position as c_int,
            )
        };
        if rc != sys::SQLITE_OK {
            return -1;
        }
        if self.position + data.len() >= self.size {
            self.position = self.size;
        } else {
            self.position += data.len();
        }
        data.len() as isize
    }

    fn can_seek(&self) -> bool {
        true
    }

    fn seek(&mut self, offset: i64, whence: i32) -> (i32, i64) {
        let size = self.size as i64;
        let pos = self.position as i64;
        let target = match whence {
            libc::SEEK_CUR => pos + offset,
            libc::SEEK_SET => offset,
            libc::SEEK_END => size + offset,
            _ => return (-1, pos),
        };
        if target < 0 {
            self.position = 0;
            (-1, -1)
        } else if target > size {
            self.position = self.size;
            (-1, -1)
        } else {
            self.position = target as usize;
            (0, target)
        }
    }

    fn stat(&mut self) -> Option<StatInfo> {
        Some(StatInfo {
            size: self.size as u64,
            ..Default::default()
        })
    }

    fn close(&mut self, _host: &mut dyn Host) -> Result<(), Thrown> {
        if !self.blob.is_null() {
            unsafe { sys::sqlite3_blob_close(self.blob) };
            self.blob = ptr::null_mut();
        }
        Ok(())
    }
}

impl Drop for BlobStream {
    fn drop(&mut self) {
        if !self.blob.is_null() {
            unsafe { sys::sqlite3_blob_close(self.blob) };
        }
    }
}

// --- SQLite3Stmt ---------------------------------------------------------------

/// SQLITE3_CHECK_INITIALIZED(stmt_obj->db_obj, stmt_obj->initialised,
/// SQLite3), then SQLITE3_CHECK_INITIALIZED_STMT.
fn live_stmt(this: &Value) -> Result<(Rc<Stmt>, Rc<Db>), Thrown> {
    let s = payload::<Stmt>(this).ok_or_else(|| not_initialised("SQLite3"))?;
    let Some(d) = s.db() else {
        return Err(not_initialised("SQLite3"));
    };
    if !s.initialised.get() {
        return Err(not_initialised("SQLite3"));
    }
    if s.stmt.get().is_null() {
        return Err(not_initialised("SQLite3Stmt"));
    }
    Ok((s, d))
}

fn stmt_method(cx: &mut Cx, method: &str, this: &Value, a: &[Value]) -> Result<Value, Thrown> {
    let b = Value::Bool;
    match method {
        "__construct" => {
            let s = payload::<Stmt>(this).ok_or_else(|| not_initialised("SQLite3Stmt"))?;
            let db_obj = arg(a, 0);
            let d = open_db(&db_obj)?;
            let sql = bytes(a, 1);
            if sql.is_empty() {
                return Ok(Value::Null);
            }
            prepare_into(cx, &d, &db_obj, &s, &sql)?;
            Ok(Value::Null)
        }
        "paramCount" => {
            let (s, _) = live_stmt(this)?;
            Ok(Value::Int(
                unsafe { sys::sqlite3_bind_parameter_count(s.stmt.get()) } as i64,
            ))
        }
        "close" => {
            let s = payload::<Stmt>(this).ok_or_else(|| not_initialised("SQLite3"))?;
            let Some(d) = s.db() else {
                return Err(not_initialised("SQLite3"));
            };
            if !s.initialised.get() {
                return Err(not_initialised("SQLite3"));
            }
            d.del_stmt(&s);
            Ok(b(true))
        }
        "reset" => {
            let (s, d) = live_stmt(this)?;
            if unsafe { sys::sqlite3_reset(s.stmt.get()) } != sys::SQLITE_OK {
                let h = s.handle();
                let m = format!("Unable to reset statement: {}", errmsg(h));
                error(cx, Some(&d), unsafe { sys::sqlite3_errcode(h) }, &m)?;
                return Ok(b(false));
            }
            Ok(b(true))
        }
        "clear" => {
            let (s, d) = live_stmt(this)?;
            if unsafe { sys::sqlite3_clear_bindings(s.stmt.get()) } != sys::SQLITE_OK {
                let h = s.handle();
                let m = format!("Unable to clear statement: {}", errmsg(h));
                error(cx, Some(&d), unsafe { sys::sqlite3_errcode(h) }, &m)?;
                return Ok(b(false));
            }
            let old = s.bound.borrow_mut().take();
            drop(old);
            Ok(b(true))
        }
        "readOnly" => {
            let (s, _) = live_stmt(this)?;
            Ok(b(unsafe { sys::sqlite3_stmt_readonly(s.stmt.get()) } != 0))
        }
        "busy" => {
            let (s, _) = live_stmt(this)?;
            Ok(b(unsafe { sys::sqlite3_stmt_busy(s.stmt.get()) } != 0))
        }
        "explain" => {
            let (s, _) = live_stmt(this)?;
            Ok(Value::Int(
                unsafe { sys::sqlite3_stmt_isexplain(s.stmt.get()) } as i64,
            ))
        }
        "setExplain" => {
            let mode = conv::to_int(&arg(a, 0));
            if !(0..=2).contains(&mode) {
                return Err(cx.value_error(
                    1,
                    "mode",
                    "must be one of the SQLite3Stmt::EXPLAIN_MODE_* constants",
                ));
            }
            let (s, _) = live_stmt(this)?;
            Ok(b(unsafe {
                sys::sqlite3_stmt_explain(s.stmt.get(), mode as c_int)
            } == sys::SQLITE_OK))
        }
        "getSQL" => {
            let (s, d) = live_stmt(this)?;
            if !bind_params(cx, &s, &d)? || cx.exception() {
                return Ok(b(false));
            }
            if conv::to_bool(&arg(a, 0)) {
                let p = unsafe { sys::sqlite3_expanded_sql(s.stmt.get()) };
                let v = Value::str(unsafe { sys::cstr(p) });
                unsafe { sys::sqlite3_free(p.cast()) };
                Ok(v)
            } else {
                Ok(Value::str(unsafe {
                    sys::cstr(sys::sqlite3_sql(s.stmt.get()))
                }))
            }
        }
        "bindParam" | "bindValue" => bind(cx, method == "bindParam", this, a),
        "execute" => execute(cx, this),
        _ => Err(Thrown::error(
            "Error",
            format!("Call to undefined method {}()", cx.fname),
        )),
    }
}

/// sqlite3stmt_bind: ($argc, $param, $value, $type)
fn bind(_cx: &mut Cx, by_ref: bool, this: &Value, a: &[Value]) -> Result<Value, Thrown> {
    let argc = conv::to_int(&arg(a, 0));
    let (s, _) = live_stmt(this)?;
    let value = if by_ref {
        match arg(a, 2) {
            Value::Array(arr) => arr.get(&ArrayKey::Int(0)).cloned().unwrap_or(Value::Null),
            v => v,
        }
    } else {
        a.get(2).cloned().unwrap_or(Value::Null)
    };
    let mut p = Param {
        number: -1,
        name: None,
        typ: conv::to_int(&arg(a, 3)),
        value,
    };
    match arg(a, 1) {
        Value::Int(n) => p.number = n,
        v => p.name = Some(conv::to_string_bytes(&v)),
    }
    // PHP_SQLITE3_SET_TYPE; bindParam's zval is a reference, which
    // takes the default (text).
    if argc < 3 && !by_ref {
        p.typ = match deref(&p.value) {
            Value::Int(_) | Value::Bool(_) => sys::SQLITE_INTEGER,
            Value::Float(_) => sys::SQLITE_FLOAT,
            Value::Null => sys::SQLITE_NULL,
            _ => sys::SQLITE_TEXT,
        } as i64;
    }
    // register_bound_parameter_to_sqlite
    if let Some(name) = &mut p.name {
        if name.first() != Some(&b':') && name.first() != Some(&b'@') {
            name.insert(0, b':');
        }
        let c = c_string(name);
        p.number = unsafe { sys::sqlite3_bind_parameter_index(s.stmt.get(), c.as_ptr()) } as i64;
    }
    if p.number < 1 {
        return Ok(Value::Bool(false));
    }
    let mut bound = s.bound.borrow_mut();
    let list = bound.get_or_insert_with(Vec::new);
    let num = p.number;
    let old = list
        .iter()
        .position(|(k, _)| *k == Key::Num(num))
        .map(|i| list.remove(i));
    let key = match &p.name {
        Some(n) => Key::Name(n.clone()),
        None => Key::Num(num),
    };
    let old2 = match list.iter_mut().find(|(k, _)| *k == key) {
        Some(slot) => Some(std::mem::replace(&mut slot.1, p)),
        None => {
            list.push((key, p));
            None
        }
    };
    drop(bound);
    drop((old, old2));
    Ok(Value::Bool(true))
}

/// php_sqlite3_bind_params: false on FAILURE.
fn bind_params(cx: &mut Cx, s: &Stmt, d: &Db) -> Result<bool, Thrown> {
    let params: Vec<Param> = match &*s.bound.borrow() {
        Some(l) => l.iter().map(|(_, p)| p.clone()).collect(),
        None => return Ok(true),
    };
    let st = s.stmt.get();
    for p in params {
        let n = p.number as c_int;
        let v = deref(&p.value);
        let fail = |cx: &mut Cx, rc: c_int| -> Result<(), Thrown> {
            if rc != sys::SQLITE_OK {
                let m = format!("Unable to bind parameter number {}", p.number);
                error(cx, Some(d), rc, &m)?;
            }
            Ok(())
        };
        if matches!(v, Value::Null) {
            fail(cx, unsafe { sys::sqlite3_bind_null(st, n) })?;
            continue;
        }
        match p.typ as c_int {
            sys::SQLITE_INTEGER => {
                // convert_to_long(parameter): the bound variable changes.
                let i = conv::to_int(&v);
                set_param(&p.value, Value::Int(i));
                fail(cx, unsafe { sys::sqlite3_bind_int64(st, n, i) })?;
            }
            sys::SQLITE_FLOAT => {
                let f = conv::to_float(&v);
                set_param(&p.value, Value::Float(f));
                fail(cx, unsafe { sys::sqlite3_bind_double(st, n, f) })?;
            }
            sys::SQLITE_BLOB => {
                let data = if let Value::Resource(_) = v {
                    if !cx.host.stream_is_open(&v) {
                        let m = format!("Unable to read stream for parameter {}", p.number);
                        error(cx, Some(d), 0, &m)?;
                        return Ok(false);
                    }
                    let mut all = Vec::new();
                    while let Some(chunk) = cx.host.stream_read(&v, 8192) {
                        if chunk.is_empty() {
                            break;
                        }
                        all.extend_from_slice(&chunk);
                    }
                    all
                } else {
                    crate::zend::zpp::zval_get_string(cx.host, &v)?
                };
                fail(cx, unsafe {
                    sys::sqlite3_bind_blob(
                        st,
                        n,
                        data.as_ptr().cast(),
                        data.len() as c_int,
                        sys::SQLITE_TRANSIENT,
                    )
                })?;
            }
            sys::SQLITE_TEXT => {
                let data = match crate::zend::zpp::zval_get_string(cx.host, &v) {
                    Ok(d) => d,
                    Err(t) => {
                        cx.pending.get_or_insert(t);
                        return Ok(false);
                    }
                };
                fail(cx, unsafe {
                    sys::sqlite3_bind_text(
                        st,
                        n,
                        data.as_ptr().cast(),
                        data.len() as c_int,
                        sys::SQLITE_TRANSIENT,
                    )
                })?;
            }
            sys::SQLITE_NULL => fail(cx, unsafe { sys::sqlite3_bind_null(st, n) })?,
            _ => {
                let m = format!(
                    "Unknown parameter type: {} for parameter {}",
                    p.typ, p.number
                );
                error(cx, Some(d), 0, &m)?;
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Write a converted value back where a bound parameter lives (its
/// reference, for bindParam).
fn set_param(slot: &Value, v: Value) {
    if let Value::Ref(r) = slot {
        *r.borrow_mut() = v;
    }
}

/// SQLite3Stmt::execute
fn execute(cx: &mut Cx, this: &Value) -> Result<Value, Thrown> {
    let s = payload::<Stmt>(this).ok_or_else(|| not_initialised("SQLite3"))?;
    let Some(d) = s.db() else {
        return Err(not_initialised("SQLite3"));
    };
    if !s.initialised.get() {
        return Err(not_initialised("SQLite3"));
    }
    let st = s.stmt.get();
    unsafe { sys::sqlite3_reset(st) };
    if !bind_params(cx, &s, &d)? || cx.exception() {
        return Ok(Value::Bool(false));
    }
    let rc = d.run(cx, || unsafe { sys::sqlite3_step(st) });
    match rc {
        sys::SQLITE_ROW | sys::SQLITE_DONE => {
            unsafe { sys::sqlite3_reset(st) };
            new_result(cx, &d, &s, this.clone(), true)
        }
        _ => {
            if rc == sys::SQLITE_ERROR {
                unsafe { sys::sqlite3_reset(st) };
            }
            if !cx.exception() {
                let h = s.handle();
                let m = format!("Unable to execute statement: {}", errmsg(h));
                error(cx, Some(&d), unsafe { sys::sqlite3_errcode(h) }, &m)?;
            }
            Ok(Value::Bool(false))
        }
    }
}

// --- SQLite3Result -------------------------------------------------------------

/// SQLITE3_CHECK_INITIALIZED(result_obj->db_obj,
/// result_obj->stmt_obj->initialised, SQLite3Result)
/// A result with its statement and database.
type LiveResult = (Rc<ResultSet>, Rc<Stmt>, Rc<Db>);

fn live_result(this: &Value) -> Result<LiveResult, Thrown> {
    let r = payload::<ResultSet>(this).ok_or_else(|| not_initialised("SQLite3Result"))?;
    let s = r.stmt.borrow().clone();
    let d = r.db.borrow().clone();
    match (s, d) {
        (Some(s), Some(d)) if s.initialised.get() => Ok((r, s, d)),
        _ => Err(not_initialised("SQLite3Result")),
    }
}

/// sqlite3result_fill_column_names_cache
fn fill_names(r: &ResultSet, st: *mut sqlite3_stmt, n: i32) {
    let names = (0..n)
        .map(|i| ArrayKey::from_bytes(unsafe { sys::cstr(sys::sqlite3_column_name(st, i)) }))
        .collect();
    *r.column_names.borrow_mut() = Some(names);
}

/// php_sqlite3_fetch_one
fn fetch_one(
    cx: &mut Cx,
    r: &ResultSet,
    d: &Db,
    st: *mut sqlite3_stmt,
    n: i32,
    mode: i64,
) -> Value {
    let mut out = PhpArray::with_capacity(n.max(0) as usize);
    let names = r.column_names.borrow();
    for i in 0..n {
        let v = column_value(cx, Some(d), st, i);
        if mode & NUM != 0 {
            out.insert(ArrayKey::Int(i as i64), v.clone());
        }
        if mode & ASSOC != 0 {
            if let Some(k) = names.as_ref().and_then(|ns| ns.get(i as usize)) {
                out.insert(k.clone(), v);
            }
        }
    }
    Value::Array(out)
}

fn step_error(cx: &mut Cx, d: &Db, s: &Stmt) -> Result<(), Thrown> {
    let h = s.handle();
    let m = format!("Unable to execute statement: {}", errmsg(h));
    error(cx, Some(d), unsafe { sys::sqlite3_errcode(h) }, &m)
}

fn result_method(cx: &mut Cx, method: &str, this: &Value, a: &[Value]) -> Result<Value, Thrown> {
    let b = Value::Bool;
    match method {
        "__construct" => Err(Thrown::error(
            "Exception",
            "SQLite3Result cannot be directly instantiated",
        )),
        "numColumns" => {
            let (_, s, _) = live_result(this)?;
            Ok(Value::Int(
                unsafe { sys::sqlite3_column_count(s.stmt.get()) } as i64,
            ))
        }
        "columnName" => {
            let col = conv::to_int(&arg(a, 0));
            let (_, s, _) = live_result(this)?;
            let p = unsafe { sys::sqlite3_column_name(s.stmt.get(), col as c_int) };
            if p.is_null() || col != col as c_int as i64 {
                return Ok(b(false));
            }
            Ok(Value::str(unsafe { sys::cstr(p) }))
        }
        "columnType" => {
            let col = conv::to_int(&arg(a, 0));
            let (_, s, _) = live_result(this)?;
            let st = s.stmt.get();
            if unsafe { sys::sqlite3_data_count(st) } == 0 {
                return Ok(b(false));
            }
            Ok(Value::Int(
                unsafe { sys::sqlite3_column_type(st, col as c_int) } as i64,
            ))
        }
        "fetchArray" => {
            let mode = conv::to_int(&arg(a, 0));
            let (r, s, d) = live_result(this)?;
            let st = s.stmt.get();
            let rc = d.run(cx, || unsafe { sys::sqlite3_step(st) });
            match rc {
                sys::SQLITE_ROW => {
                    if r.column_count.get() == -1 {
                        r.column_count.set(unsafe { sys::sqlite3_column_count(st) });
                    }
                    let n = r.column_count.get();
                    if mode & ASSOC != 0 && r.column_names.borrow().is_none() {
                        fill_names(&r, st, n);
                    }
                    Ok(fetch_one(cx, &r, &d, st, n, mode))
                }
                sys::SQLITE_DONE => Ok(b(false)),
                _ => {
                    step_error(cx, &d, &s)?;
                    Ok(b(false))
                }
            }
        }
        "fetchAll" => {
            let mode = conv::to_int(&arg(a, 0));
            let (r, s, d) = live_result(this)?;
            let st = s.stmt.get();
            let n = unsafe { sys::sqlite3_column_count(st) };
            if mode & ASSOC != 0 && r.column_names.borrow().is_none() {
                fill_names(&r, st, n);
            }
            r.column_count.set(n);
            let mut out = PhpArray::new();
            loop {
                let rc = d.run(cx, || unsafe { sys::sqlite3_step(st) });
                match rc {
                    sys::SQLITE_ROW => {
                        out.push(fetch_one(cx, &r, &d, st, n, mode));
                    }
                    sys::SQLITE_DONE => break,
                    _ => {
                        if !cx.exception() {
                            step_error(cx, &d, &s)?;
                        }
                        return Ok(b(false));
                    }
                }
            }
            Ok(Value::Array(out))
        }
        "reset" => {
            let (r, s, _) = live_result(this)?;
            r.clear_cache();
            Ok(b(
                unsafe { sys::sqlite3_reset(s.stmt.get()) } == sys::SQLITE_OK
            ))
        }
        "finalize" => {
            let (r, s, d) = live_result(this)?;
            r.clear_cache();
            if !r.is_prepared.get() {
                d.del_stmt(&s);
            } else {
                unsafe { sys::sqlite3_reset(s.stmt.get()) };
            }
            Ok(b(true))
        }
        _ => Err(Thrown::error(
            "Error",
            format!("Call to undefined method {}()", cx.fname),
        )),
    }
}

// --- callbacks -----------------------------------------------------------------

/// `php_sqlite3_agg_context`, in memory sqlite zeroes and owns.
#[repr(C)]
struct AggContext {
    /// A leaked `Box<Value>`, or null.
    val: *mut Value,
    row: i64,
}

unsafe fn agg_context<'a>(ctx: *mut sqlite3_context) -> Option<&'a mut AggContext> {
    let p: *mut AggContext =
        unsafe { sys::sqlite3_aggregate_context(ctx, std::mem::size_of::<AggContext>() as c_int) }
            .cast();
    unsafe { p.as_mut() }
}

/// A sqlite value as a callback argument.
unsafe fn value_of(v: *mut sqlite3_value) -> Value {
    unsafe {
        match sys::sqlite3_value_type(v) {
            sys::SQLITE_INTEGER => Value::Int(sys::sqlite3_value_int64(v)),
            sys::SQLITE_FLOAT => Value::Float(sys::sqlite3_value_double(v)),
            sys::SQLITE_NULL => Value::Null,
            _ => {
                let p = sys::sqlite3_value_text(v);
                let n = sys::sqlite3_value_bytes(v);
                if p.is_null() {
                    Value::str("")
                } else {
                    Value::str(std::slice::from_raw_parts(p, n.max(0) as usize))
                }
            }
        }
    }
}

/// RAII for `(*in_callback)++` / `--`.
struct InCallback(*const Db);

impl InCallback {
    fn enter(db: *const Db) -> InCallback {
        if let Some(d) = unsafe { db.as_ref() } {
            d.in_callback.set(d.in_callback.get() + 1);
        }
        InCallback(db)
    }
}

impl Drop for InCallback {
    fn drop(&mut self) {
        if let Some(d) = unsafe { self.0.as_ref() } {
            d.in_callback.set(d.in_callback.get().saturating_sub(1));
        }
    }
}

/// sqlite3_do_callback
unsafe fn do_callback(
    cb: &Value,
    db: *const Db,
    argc: c_int,
    argv: *mut *mut sqlite3_value,
    ctx: *mut sqlite3_context,
    is_agg: bool,
) {
    let mut args = Vec::with_capacity(argc.max(0) as usize + 2);
    let mut agg: Option<&mut AggContext> = None;
    if is_agg {
        let Some(a) = (unsafe { agg_context(ctx) }) else {
            return;
        };
        if a.val.is_null() {
            a.val = Box::into_raw(Box::new(Value::Null));
        }
        args.push(unsafe { (*a.val).clone() });
        args.push(Value::Int(a.row));
        agg = Some(a);
    }
    for i in 0..argc.max(0) as usize {
        args.push(unsafe { value_of(*argv.add(i)) });
    }
    let ret = {
        let _guard = InCallback::enter(db);
        if ffi::aborted() {
            None
        } else {
            ffi::call_host(|h| h.call_callable(cb, &args))
        }
    };
    drop(args);
    if !is_agg || argv.is_null() {
        match ret {
            Some(v) => match rhp_values::ops::deref(&v) {
                Value::Int(n) => unsafe { sys::sqlite3_result_int64(ctx, n) },
                Value::Null => unsafe { sys::sqlite3_result_null(ctx) },
                Value::Float(f) => unsafe { sys::sqlite3_result_double(ctx, f) },
                other => {
                    let s = ffi::call_host(|h| crate::zend::zpp::zval_get_string(h, &other));
                    if let Some(s) = s {
                        unsafe {
                            sys::sqlite3_result_text(
                                ctx,
                                s.as_ptr().cast(),
                                s.len() as c_int,
                                sys::SQLITE_TRANSIENT,
                            )
                        };
                    }
                }
            },
            None => unsafe {
                sys::sqlite3_result_error(ctx, c"failed to invoke callback".as_ptr(), 0)
            },
        }
        if let Some(a) = agg {
            if !a.val.is_null() {
                drop(unsafe { Box::from_raw(a.val) });
                a.val = ptr::null_mut();
            }
        }
    } else if let Some(a) = agg {
        // The step's return value is the next context.
        let v = ret
            .map(|v| rhp_values::ops::deref(&v))
            .unwrap_or(Value::Null);
        unsafe { *a.val = v };
    }
}

unsafe fn func_of<'a>(ctx: *mut sqlite3_context) -> &'a Func {
    unsafe { &*(sys::sqlite3_user_data(ctx) as *const Func) }
}

/// php_sqlite3_callback_func
unsafe extern "C" fn func_callback(
    ctx: *mut sqlite3_context,
    argc: c_int,
    argv: *mut *mut sqlite3_value,
) {
    ffi::guard((), || unsafe {
        let f = func_of(ctx);
        if let Some(cb) = &f.func {
            do_callback(cb, f.db, argc, argv, ctx, false);
        }
    })
}

/// php_sqlite3_callback_step
unsafe extern "C" fn step_callback(
    ctx: *mut sqlite3_context,
    argc: c_int,
    argv: *mut *mut sqlite3_value,
) {
    ffi::guard((), || unsafe {
        let f = func_of(ctx);
        if let Some(a) = agg_context(ctx) {
            a.row += 1;
        }
        if let Some(cb) = &f.step {
            do_callback(cb, f.db, argc, argv, ctx, true);
        }
    })
}

/// php_sqlite3_callback_final
unsafe extern "C" fn final_callback(ctx: *mut sqlite3_context) {
    ffi::guard((), || unsafe {
        let f = func_of(ctx);
        if let Some(a) = agg_context(ctx) {
            a.row = 0;
        }
        if let Some(cb) = &f.fini {
            do_callback(cb, f.db, 0, ptr::null_mut(), ctx, true);
        }
    })
}

/// php_sqlite3_callback_compare
unsafe extern "C" fn collation_callback(
    coll: *mut c_void,
    l1: c_int,
    s1: *const c_void,
    l2: c_int,
    s2: *const c_void,
) -> c_int {
    ffi::guard(0, || {
        let c = unsafe { &*(coll as *const Collation) };
        if ffi::aborted() {
            return 0;
        }
        let a = unsafe { std::slice::from_raw_parts(s1 as *const u8, l1.max(0) as usize) };
        let b = unsafe { std::slice::from_raw_parts(s2 as *const u8, l2.max(0) as usize) };
        let args = [Value::str(a), Value::str(b)];
        let r = {
            let _guard = InCallback::enter(c.db);
            ffi::call_host(|h| h.call_callable(&c.cmp, &args))
        };
        if ffi::aborted() {
            return 0;
        }
        match r.map(|v| rhp_values::ops::deref(&v)) {
            Some(Value::Int(n)) => n as c_int,
            _ => {
                callback_error(
                    None,
                    "An error occurred while invoking the compare callback (invalid return type).  Collation behaviour is undefined.",
                );
                0
            }
        }
    })
}

/// php_sqlite3_authorizer: open_basedir for ATTACH, then the user's
/// callback.
unsafe extern "C" fn authorizer(
    arg: *mut c_void,
    action: c_int,
    a1: *const c_char,
    a2: *const c_char,
    a3: *const c_char,
    a4: *const c_char,
) -> c_int {
    ffi::guard(sys::SQLITE_DENY, || {
        if crate::main::fopen_wrappers::active() && action == sys::SQLITE_ATTACH {
            if a1.is_null() {
                return sys::SQLITE_DENY;
            }
            let name = unsafe { sys::cstr(a1) };
            if name != b":memory:" && !name.is_empty() {
                if name.starts_with(b"file:") {
                    return sys::SQLITE_DENY;
                }
                let fname = ffi::fname();
                let ok =
                    ffi::call_host(|h| crate::main::fopen_wrappers::check(h, &fname, name, true));
                if ok != Some(true) {
                    return sys::SQLITE_DENY;
                }
            }
        }
        let d = unsafe { &*(arg as *const Db) };
        let Some(cb) = d.authorizer.borrow().clone() else {
            return sys::SQLITE_OK;
        };
        let s = |p: *const c_char| {
            if p.is_null() {
                Value::Null
            } else {
                Value::str(unsafe { sys::cstr(p) })
            }
        };
        let args = [Value::Int(action as i64), s(a1), s(a2), s(a3), s(a4)];
        let r = {
            let _guard = InCallback::enter(d);
            if ffi::aborted() {
                None
            } else {
                ffi::call_host(|h| h.call_callable(&cb, &args))
            }
        };
        match r.map(|v| rhp_values::ops::deref(&v)) {
            None => {
                callback_error(
                    Some(d),
                    "An error occurred while invoking the authorizer callback",
                );
                sys::SQLITE_DENY
            }
            Some(Value::Int(n)) => {
                let n = n as c_int;
                if n != sys::SQLITE_OK && n != sys::SQLITE_IGNORE && n != sys::SQLITE_DENY {
                    callback_error(
                        Some(d),
                        &format!("The authorizer callback returned an invalid value: {n}"),
                    );
                    return sys::SQLITE_DENY;
                }
                n
            }
            Some(_) => {
                callback_error(
                    Some(d),
                    "The authorizer callback returned an invalid type: expected int",
                );
                sys::SQLITE_DENY
            }
        }
    })
}
