//! ext/pdo_sqlite/sqlite_driver.c: the connection (`pdo_sqlite_db_handle`),
//! its user functions, aggregates, collations and authorizer.

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_int, c_void, CString};
use std::ptr;
use std::rc::Rc;

use rhp_values::{PhpArray, PhpRef, Value};

use super::sqlite_statement::SqliteStmt;
use super::sys::{self, sqlite3, sqlite3_context, sqlite3_value};
use crate::ext::libxml::ffi;
use crate::ext::pdo::driver::*;
use crate::ext::pdo::{self, Cx, Dbh, Stmt};
use crate::Thrown;

pub(crate) const ATTR_OPEN_FLAGS: i64 = ATTR_DRIVER_SPECIFIC;
pub(crate) const ATTR_READONLY_STATEMENT: i64 = ATTR_DRIVER_SPECIFIC + 1;
pub(crate) const ATTR_EXTENDED_RESULT_CODES: i64 = ATTR_DRIVER_SPECIFIC + 2;
pub(crate) const ATTR_BUSY_STATEMENT: i64 = ATTR_DRIVER_SPECIFIC + 3;
pub(crate) const ATTR_EXPLAIN_STATEMENT: i64 = ATTR_DRIVER_SPECIFIC + 4;
pub(crate) const ATTR_TRANSACTION_MODE: i64 = ATTR_DRIVER_SPECIFIC + 5;

const TRANSACTION_MODE_DEFERRED: i64 = 0;
const TRANSACTION_MODE_IMMEDIATE: i64 = 1;
const TRANSACTION_MODE_EXCLUSIVE: i64 = 2;

/// `struct pdo_sqlite_func`
pub(crate) struct Func {
    pub(crate) name: CString,
    pub(crate) argc: c_int,
    pub(crate) func: Option<Value>,
    pub(crate) step: Option<Value>,
    pub(crate) fini: Option<Value>,
}

/// `struct pdo_sqlite_collation`
pub(crate) struct Collation {
    pub(crate) name: CString,
    pub(crate) callback: Value,
}

/// `pdo_sqlite_db_handle`
pub(crate) struct Handle {
    pub(crate) db: Cell<*mut sqlite3>,
    errcode: Cell<c_int>,
    errmsg: RefCell<Vec<u8>>,
    /// Boxed: sqlite keeps a pointer to each as its user data, which must
    /// not move when the list grows.
    #[allow(clippy::vec_box)]
    pub(crate) funcs: RefCell<Vec<Box<Func>>>,
    #[allow(clippy::vec_box)]
    pub(crate) collations: RefCell<Vec<Box<Collation>>>,
    pub(crate) authorizer: RefCell<Option<Value>>,
    transaction_mode: Cell<i64>,
}

impl Handle {
    /// Whether running SQL may call PHP (user functions, collations, the
    /// authorizer, open_basedir's warnings), so it needs the host.
    pub(crate) fn calls_php(&self) -> bool {
        !self.funcs.borrow().is_empty()
            || !self.collations.borrow().is_empty()
            || self.authorizer.borrow().is_some()
            || crate::main::fopen_wrappers::active()
    }

    /// Run `f` (sqlite code that may call back into PHP) with the host
    /// reachable when it may.
    pub(crate) fn run<R>(&self, cx: &mut Cx, f: impl FnOnce() -> R) -> R {
        if !self.calls_php() {
            return f();
        }
        let db = self.db.get();
        cx.ffi(move || unsafe { sys::sqlite3_interrupt(db) }, f)
    }

    /// _pdo_sqlite_error: record the connection's error on `stmt` or the
    /// handle, as a SQLSTATE.
    pub(crate) fn error(&self, dbh: &Dbh, stmt: Option<&Stmt>) -> c_int {
        let db = self.db.get();
        let code = unsafe { sys::sqlite3_errcode(db) };
        self.errcode.set(code);
        let set = |s: &str| match stmt {
            Some(st) => st.set_error(s),
            None => dbh.set_error(s),
        };
        if code == sys::SQLITE_OK {
            set(ERR_NONE);
            return 0;
        }
        *self.errmsg.borrow_mut() = unsafe { sys::cstr(sys::sqlite3_errmsg(db)) }.to_vec();
        set(match code {
            sys::SQLITE_NOTFOUND => "42S02",
            sys::SQLITE_INTERRUPT => "01002",
            sys::SQLITE_NOLFS => "HYC00",
            sys::SQLITE_TOOBIG => "22001",
            sys::SQLITE_CONSTRAINT => "23000",
            _ => "HY000",
        });
        code
    }

    fn exec_sql(&self, cx: &mut Cx, sql: &CString) -> c_int {
        let db = self.db.get();
        let p = sql.as_ptr();
        self.run(cx, || unsafe {
            sys::sqlite3_exec(db, p, ptr::null(), ptr::null_mut(), ptr::null_mut())
        })
    }

    /// pdo_sqlite_cleanup_callbacks
    fn cleanup_callbacks(&self) {
        *self.authorizer.borrow_mut() = None;
        let db = self.db.get();
        let funcs = std::mem::take(&mut *self.funcs.borrow_mut());
        for f in &funcs {
            if !db.is_null() {
                unsafe {
                    sys::sqlite3_create_function(
                        db,
                        f.name.as_ptr(),
                        f.argc,
                        sys::SQLITE_UTF8,
                        &**f as *const Func as *mut c_void,
                        None,
                        None,
                        None,
                    );
                }
            }
        }
        let colls = std::mem::take(&mut *self.collations.borrow_mut());
        for c in &colls {
            if !db.is_null() {
                unsafe {
                    sys::sqlite3_create_collation(
                        db,
                        c.name.as_ptr(),
                        sys::SQLITE_UTF8,
                        &**c as *const Collation as *mut c_void,
                        None,
                    );
                }
            }
        }
    }

    /// sqlite_handle_closer
    fn closer(&self) {
        self.cleanup_callbacks();
        let db = self.db.replace(ptr::null_mut());
        if !db.is_null() {
            unsafe { sys::sqlite3_close_v2(db) };
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.closer();
    }
}

/// A CString of `b` up to its first NUL, as C reads php's `char *`.
pub(crate) fn c_string(b: &[u8]) -> CString {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    CString::new(&b[..end]).expect("no NUL")
}

impl DbhDriver for Handle {
    /// sqlite_handle_preparer
    fn prepare(
        &self,
        cx: &mut Cx,
        dbh: &Dbh,
        sql: &[u8],
        stmt: &Stmt,
        options: Option<&PhpArray>,
    ) -> bool {
        stmt.supports_placeholders.set(3);
        if attr_lval(options, ATTR_CURSOR, CURSOR_FWDONLY) != CURSOR_FWDONLY {
            self.errcode.set(sys::SQLITE_ERROR);
            self.error(dbh, None);
            return false;
        }
        let db = self.db.get();
        let mut s: *mut sys::sqlite3_stmt = ptr::null_mut();
        let mut tail: *const c_char = ptr::null();
        let rc = self.run(cx, || unsafe {
            sys::sqlite3_prepare_v2(
                db,
                sql.as_ptr().cast(),
                sql.len() as c_int,
                &mut s,
                &mut tail,
            )
        });
        let me = handle_rc(dbh);
        *stmt.driver.borrow_mut() = Some(Rc::new(SqliteStmt::new(me, s)));
        if rc == sys::SQLITE_OK {
            return true;
        }
        self.error(dbh, None);
        false
    }

    /// sqlite_handle_doer
    fn exec(&self, cx: &mut Cx, dbh: &Dbh, sql: &[u8]) -> i64 {
        let q = c_string(sql);
        if self.exec_sql(cx, &q) != sys::SQLITE_OK {
            self.error(dbh, None);
            -1
        } else {
            unsafe { sys::sqlite3_changes(self.db.get()) as i64 }
        }
    }

    /// sqlite_handle_quoter
    fn quote(&self, cx: &mut Cx, dbh: &Dbh, s: &[u8], _ty: i64) -> Result<Option<Vec<u8>>, Thrown> {
        if s.len() > (i32::MAX as usize - 3) / 2 {
            return Ok(None);
        }
        if s.contains(&0) {
            let msg = "SQLite PDO::quote does not support null bytes";
            match dbh.error_mode.get() {
                ERRMODE_EXCEPTION => return Err(pdo::throw(cx.host, msg)),
                ERRMODE_WARNING => cx.warn(msg)?,
                _ => {}
            }
            return Ok(None);
        }
        // sqlite3_snprintf("'%q'")
        let mut out = Vec::with_capacity(s.len() + 2);
        out.push(b'\'');
        for &c in s {
            if c == b'\'' {
                out.push(b'\'');
            }
            out.push(c);
        }
        out.push(b'\'');
        Ok(Some(out))
    }

    fn begin(&self, cx: &mut Cx, dbh: &Dbh) -> bool {
        let sql = match self.transaction_mode.get() {
            TRANSACTION_MODE_DEFERRED => "BEGIN DEFERRED TRANSACTION",
            TRANSACTION_MODE_IMMEDIATE => "BEGIN IMMEDIATE TRANSACTION",
            TRANSACTION_MODE_EXCLUSIVE => "BEGIN EXCLUSIVE TRANSACTION",
            _ => "BEGIN",
        };
        self.simple(cx, dbh, sql)
    }

    fn commit(&self, cx: &mut Cx, dbh: &Dbh) -> bool {
        self.simple(cx, dbh, "COMMIT")
    }

    fn rollback(&self, cx: &mut Cx, dbh: &Dbh) -> bool {
        self.simple(cx, dbh, "ROLLBACK")
    }

    fn rollback_on_free(&self) {
        let db = self.db.get();
        if !db.is_null() {
            unsafe {
                sys::sqlite3_exec(
                    db,
                    c"ROLLBACK".as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            };
        }
    }

    /// pdo_sqlite_set_attr
    fn set_attribute(&self, cx: &mut Cx, _dbh: &Dbh, attr: i64, v: &Value) -> Result<bool, Thrown> {
        let db = self.db.get();
        match attr {
            ATTR_TIMEOUT => {
                let l = pdo::dbh::get_long_param(cx, v)?;
                unsafe { sys::sqlite3_busy_timeout(db, l.wrapping_mul(1000) as c_int) };
                Ok(true)
            }
            ATTR_EXTENDED_RESULT_CODES => {
                let l = pdo::dbh::get_long_param(cx, v)?;
                unsafe { sys::sqlite3_extended_result_codes(db, l as c_int) };
                Ok(true)
            }
            ATTR_TRANSACTION_MODE => {
                let l = pdo::dbh::get_long_param(cx, v)?;
                if (TRANSACTION_MODE_DEFERRED..=TRANSACTION_MODE_EXCLUSIVE).contains(&l) {
                    self.transaction_mode.set(l);
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            _ => Ok(false),
        }
    }

    fn last_id(&self, _dbh: &Dbh, _name: Option<&[u8]>) -> Option<Vec<u8>> {
        let id = unsafe { sys::sqlite3_last_insert_rowid(self.db.get()) };
        Some(id.to_string().into_bytes())
    }

    /// pdo_sqlite_fetch_error_func
    fn fetch_err(&self, _dbh: &Dbh, _stmt: Option<&Stmt>, info: &mut PhpArray) {
        if self.errcode.get() != 0 {
            info.push(Value::Int(self.errcode.get() as i64));
            info.push(Value::str(self.errmsg.borrow().clone()));
        }
    }

    /// pdo_sqlite_get_attribute
    fn get_attribute(&self, _dbh: &Dbh, attr: i64) -> (i32, Value) {
        match attr {
            ATTR_CLIENT_VERSION | ATTR_SERVER_VERSION => (1, Value::str(sys::libversion())),
            ATTR_TRANSACTION_MODE => (1, Value::Int(self.transaction_mode.get())),
            _ => (0, Value::Null),
        }
    }

    /// pdo_sqlite_in_transaction
    fn in_transaction(&self, _dbh: &Dbh) -> bool {
        let db = self.db.get();
        !db.is_null() && unsafe { sys::sqlite3_get_autocommit(db) } == 0
    }

    fn close(&self) {
        self.closer();
    }

    /// pdo_sqlite_get_gc
    fn gc_values(&self, out: &mut Vec<Value>) {
        if let Some(a) = self.authorizer.borrow().clone() {
            out.push(a);
        }
        for f in self.funcs.borrow().iter() {
            for v in [&f.func, &f.step, &f.fini].into_iter().flatten() {
                out.push(v.clone());
            }
        }
        for c in self.collations.borrow().iter() {
            out.push(c.callback.clone());
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Handle {
    fn simple(&self, cx: &mut Cx, dbh: &Dbh, sql: &str) -> bool {
        let q = c_string(sql.as_bytes());
        if self.exec_sql(cx, &q) != sys::SQLITE_OK {
            self.error(dbh, None);
            return false;
        }
        true
    }
}

/// The connection of `dbh` as its driver type.
pub(crate) fn handle_rc(dbh: &Dbh) -> Rc<Handle> {
    let d = dbh.driver().expect("connected");
    let raw = Rc::into_raw(d) as *const Handle;
    // SAFETY: pdo_sqlite's DbhDriver is always a Handle (as_any below
    // checks before any caller gets here).
    unsafe { Rc::from_raw(raw) }
}

/// The connection of `dbh`, when its driver is pdo_sqlite.
pub(crate) fn handle_of(dbh: &Dbh) -> Option<Rc<Handle>> {
    let d = dbh.driver()?;
    d.as_any().downcast_ref::<Handle>()?;
    Some(handle_rc(dbh))
}

/// make_filename_safe: the path sqlite opens, `None` when open_basedir
/// refuses it.
fn make_filename_safe(cx: &mut Cx, name: &[u8]) -> Result<Option<Vec<u8>>, Thrown> {
    if name.len() >= 5 && name[..5].eq_ignore_ascii_case(b"file:") {
        if crate::main::fopen_wrappers::active() {
            return Ok(None);
        }
        return Ok(Some(name.to_vec()));
    }
    if !name.is_empty() && name != b":memory:" {
        let full = crate::zend::virtual_cwd::expand_filepath(name);
        if full.is_empty() {
            return Ok(None);
        }
        if let Some(m) = crate::main::fopen_wrappers::refusal(&full) {
            // Under the constructor's EH_THROW the warning is an exception
            // the next one gets as its previous.
            if let Err(t) = cx.warn(&m) {
                if !cx.eh_throw {
                    return Err(t);
                }
                cx.pending.get_or_insert(t);
            }
            return Ok(None);
        }
        return Ok(Some(full));
    }
    Ok(Some(name.to_vec()))
}

/// pdo_sqlite_handle_factory
pub(crate) fn handle_factory(
    cx: &mut Cx,
    dbh: &Dbh,
    options: Option<&PhpArray>,
) -> Result<Option<Rc<dyn DbhDriver>>, Thrown> {
    let h = Rc::new(Handle {
        db: Cell::new(ptr::null_mut()),
        errcode: Cell::new(0),
        errmsg: RefCell::new(Vec::new()),
        funcs: RefCell::new(Vec::new()),
        collations: RefCell::new(Vec::new()),
        authorizer: RefCell::new(None),
        transaction_mode: Cell::new(TRANSACTION_MODE_DEFERRED),
    });
    // skip all but this one param event
    dbh.skip_param_evt.set(0x7F ^ (1 << PARAM_EVT_EXEC_PRE));
    let source = dbh.data_source.borrow().clone();
    let Some(filename) = make_filename_safe(cx, &source)? else {
        let msg = format!(
            "open_basedir prohibits opening {}",
            String::from_utf8_lossy(&source)
        );
        return Err(pdo::throw(cx.host, &msg));
    };
    let mut flags = attr_lval(
        options,
        ATTR_OPEN_FLAGS,
        (sys::SQLITE_OPEN_READWRITE | sys::SQLITE_OPEN_CREATE) as i64,
    ) as c_int;
    if !crate::main::fopen_wrappers::active() {
        flags |= sys::SQLITE_OPEN_URI;
    }
    let path = c_string(&filename);
    let mut db: *mut sqlite3 = ptr::null_mut();
    let rc = crate::zend::virtual_cwd::library_created(&filename, 0o644, || unsafe {
        sys::sqlite3_open_v2(path.as_ptr(), &mut db, flags, ptr::null())
    });
    h.db.set(db);
    if rc != sys::SQLITE_OK {
        // pdo_sqlite_error before dbh->methods is set: pdo_throw_exception.
        let code = unsafe { sys::sqlite3_errcode(db) };
        let msg =
            String::from_utf8_lossy(unsafe { sys::cstr(sys::sqlite3_errmsg(db)) }).into_owned();
        let state = match code {
            sys::SQLITE_NOTFOUND => "42S02",
            sys::SQLITE_INTERRUPT => "01002",
            sys::SQLITE_NOLFS => "HYC00",
            sys::SQLITE_TOOBIG => "22001",
            sys::SQLITE_CONSTRAINT => "23000",
            _ => "HY000",
        };
        dbh.set_error(state);
        return Err(pdo::throw_exception(cx.host, code as i64, &msg, state));
    }
    unsafe {
        sys::sqlite3_set_authorizer(db, Some(authorizer), Rc::as_ptr(&h) as *mut c_void);
    }
    let timeout = attr_lval(options, ATTR_TIMEOUT, 60);
    unsafe { sys::sqlite3_busy_timeout(db, timeout.wrapping_mul(1000) as c_int) };
    dbh.alloc_own_columns.set(true);
    Ok(Some(h))
}

// --- callbacks -------------------------------------------------------------------

/// `aggregate_context`: the step callbacks' context value and row count,
/// in memory sqlite zeroes and owns.
#[repr(C)]
struct AggContext {
    val: *const std::cell::RefCell<Value>,
    row: i64,
}

/// A sqlite value as PHP sees it in a callback.
unsafe fn value_of(v: *mut sqlite3_value) -> Value {
    unsafe {
        match sys::sqlite3_value_type(v) {
            sys::SQLITE_INTEGER => Value::Int(sys::sqlite3_value_int(v) as i64),
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

/// do_callback
unsafe fn do_callback(
    cb: &Value,
    argc: c_int,
    argv: *mut *mut sqlite3_value,
    ctx: *mut sqlite3_context,
    is_agg: bool,
) {
    let mut args = Vec::with_capacity(argc.max(0) as usize + 2);
    let mut agg: *mut AggContext = ptr::null_mut();
    if is_agg {
        agg = unsafe {
            sys::sqlite3_aggregate_context(ctx, std::mem::size_of::<AggContext>() as c_int)
        }
        .cast();
        if agg.is_null() {
            return;
        }
        let a = unsafe { &mut *agg };
        if a.val.is_null() {
            let r: PhpRef = Rc::new(RefCell::new(Value::Null));
            a.val = Rc::into_raw(r);
        }
        let r: PhpRef = unsafe {
            Rc::increment_strong_count(a.val);
            Rc::from_raw(a.val)
        };
        args.push(Value::Ref(r));
        a.row += 1;
        args.push(Value::Int(a.row));
    }
    for i in 0..argc.max(0) as usize {
        args.push(unsafe { value_of(*argv.add(i)) });
    }
    // zend_call_known_fcc does nothing while an exception is pending.
    let ret = if ffi::aborted() {
        None
    } else {
        ffi::call_host(|h| h.call_callable(cb, &args))
    };
    drop(args);
    if !is_agg || argv.is_null() {
        match ret {
            Some(v) => {
                let v = rhp_values::ops::deref(&v);
                match &v {
                    Value::Int(n) => unsafe { sys::sqlite3_result_int(ctx, *n as c_int) },
                    Value::Null => unsafe { sys::sqlite3_result_null(ctx) },
                    Value::Float(f) => unsafe { sys::sqlite3_result_double(ctx, *f) },
                    other => {
                        let s = ffi::call_host(|h| crate::zend::zpp::zval_get_string(h, other));
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
                }
            }
            None => unsafe {
                sys::sqlite3_result_error(ctx, c"failed to invoke callback".as_ptr(), 0)
            },
        }
        if !agg.is_null() {
            let a = unsafe { &mut *agg };
            if !a.val.is_null() {
                drop(unsafe { Rc::from_raw(a.val) });
                a.val = ptr::null();
            }
        }
    } else if !agg.is_null() {
        let a = unsafe { &mut *agg };
        if let Some(v) = ret {
            let r = unsafe { &*a.val };
            *r.borrow_mut() = rhp_values::ops::deref(&v);
        }
    }
}

unsafe fn func_of<'a>(ctx: *mut sqlite3_context) -> &'a Func {
    unsafe { &*(sys::sqlite3_user_data(ctx) as *const Func) }
}

/// php_sqlite3_func_callback
pub(crate) unsafe extern "C" fn func_callback(
    ctx: *mut sqlite3_context,
    argc: c_int,
    argv: *mut *mut sqlite3_value,
) {
    ffi::guard((), || unsafe {
        let f = func_of(ctx);
        if let Some(cb) = &f.func {
            do_callback(cb, argc, argv, ctx, false);
        }
    })
}

/// php_sqlite3_func_step_callback
pub(crate) unsafe extern "C" fn step_callback(
    ctx: *mut sqlite3_context,
    argc: c_int,
    argv: *mut *mut sqlite3_value,
) {
    ffi::guard((), || unsafe {
        let f = func_of(ctx);
        if let Some(cb) = &f.step {
            do_callback(cb, argc, argv, ctx, true);
        }
    })
}

/// php_sqlite3_func_final_callback
pub(crate) unsafe extern "C" fn final_callback(ctx: *mut sqlite3_context) {
    ffi::guard((), || unsafe {
        let f = func_of(ctx);
        if let Some(cb) = &f.fini {
            do_callback(cb, 0, ptr::null_mut(), ctx, true);
        }
    })
}

/// php_sqlite3_collation_callback
pub(crate) unsafe extern "C" fn collation_callback(
    ctx: *mut c_void,
    l1: c_int,
    s1: *const c_void,
    l2: c_int,
    s2: *const c_void,
) -> c_int {
    ffi::guard(0, || {
        let c = unsafe { &*(ctx as *const Collation) };
        let a = unsafe { std::slice::from_raw_parts(s1 as *const u8, l1.max(0) as usize) };
        let b = unsafe { std::slice::from_raw_parts(s2 as *const u8, l2.max(0) as usize) };
        if ffi::aborted() {
            return 0;
        }
        let args = [Value::str(a), Value::str(b)];
        let Some(r) = ffi::call_host(|h| h.call_callable(&c.callback, &args)) else {
            return 0;
        };
        match rhp_values::ops::deref(&r) {
            Value::Int(n) => n.signum() as c_int,
            other => {
                let fname = ffi::fname();
                ffi::call_host(|h| -> Result<(), Thrown> {
                    Err(Thrown::type_error(format!(
                        "{fname}(): Return value of the collation callback must be of type int, {} returned",
                        crate::zend::zpp::zval_value_name(h, &other)
                    )))
                });
                -1
            }
        }
    })
}

/// The authorizer every connection gets: open_basedir for ATTACH, then
/// the user's callback.
unsafe extern "C" fn authorizer(
    arg: *mut c_void,
    access: c_int,
    a1: *const c_char,
    a2: *const c_char,
    a3: *const c_char,
    a4: *const c_char,
) -> c_int {
    ffi::guard(sys::SQLITE_DENY, || {
        let h = unsafe { &*(arg as *const Handle) };
        if crate::main::fopen_wrappers::active() && access == sys::SQLITE_ATTACH {
            if a1.is_null() {
                return sys::SQLITE_DENY;
            }
            let name = unsafe { sys::cstr(a1) }.to_vec();
            let ok = ffi::call_host(|host| {
                let mut cx = Cx::new(host, ffi::fname());
                make_filename_safe(&mut cx, &name)
            })
            .flatten();
            if ok.is_none() {
                return sys::SQLITE_DENY;
            }
        }
        let Some(cb) = h.authorizer.borrow().clone() else {
            return sys::SQLITE_OK;
        };
        if ffi::aborted() {
            return sys::SQLITE_DENY;
        }
        let s = |p: *const c_char| {
            if p.is_null() {
                Value::Null
            } else {
                Value::str(unsafe { sys::cstr(p) })
            }
        };
        let args = [Value::Int(access as i64), s(a1), s(a2), s(a3), s(a4)];
        let fname = ffi::fname();
        ffi::call_host(|host| {
            let r = rhp_values::ops::deref(&host.call_callable(&cb, &args)?);
            match r {
                Value::Int(n) => {
                    let n = n as c_int;
                    if n != sys::SQLITE_OK && n != sys::SQLITE_IGNORE && n != sys::SQLITE_DENY {
                        return Err(Thrown::error("ValueError", format!(
                            "{fname}(): Return value of the authorizer callback must be one of Pdo\\Sqlite::OK, Pdo\\Sqlite::DENY, or Pdo\\Sqlite::IGNORE"
                        )));
                    }
                    Ok(n)
                }
                other => Err(Thrown::type_error(format!(
                    "{fname}(): Return value of the authorizer callback must be of type int, {} returned",
                    crate::zend::zpp::zval_value_name(host, &other)
                ))),
            }
        })
        .unwrap_or(sys::SQLITE_DENY)
    })
}
