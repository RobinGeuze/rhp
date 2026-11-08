//! The `PDO` extension: a port of ext/pdo (pdo.c, pdo_dbh.c, pdo_stmt.c,
//! pdo_sqlstate.c). `PDO`, `PDOStatement`, `PDORow` and `PDOException` are
//! declared in PHP ([`PRELUDE_PHP`]); their methods call `__pdo` /
//! `__pdo_stmt` with `$this`, and an object's state (php's `pdo_dbh_t` /
//! `pdo_stmt_t`) is its payload, created on first use. Drivers implement
//! [`driver::DbhDriver`] and [`driver::StmtDriver`]; the one driver is
//! pdo_sqlite (`ext/pdo_sqlite`).

pub(crate) mod dbh;
pub(crate) mod driver;
pub(crate) mod sqlstate;
pub(crate) mod stmt;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rhp_values::{conv, ObjectHandle, PhpArray, Value};

use crate::ext::libxml::ffi;
use crate::zend::object_handlers::{self, ClassHandlers, ObjectHandlers};
use crate::{Host, Thrown};

use driver::*;
pub(crate) use rhp_values::ops::deref;
pub(crate) use stmt::Stmt;

pub const PRELUDE_PHP: &str = include_str!("prelude.php");

pub(crate) static CLASSES: &[ClassHandlers] = &[
    ClassHandlers {
        class: "PDO",
        handlers: &DBH_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "Pdo\\Sqlite",
        handlers: &DBH_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "PDOStatement",
        handlers: &stmt::STMT_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "PDORow",
        handlers: &stmt::ROW_HANDLERS,
        create_object: true,
    },
];

/// pdo_dbh_object_handlers
pub(crate) static DBH_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_gc: Some(dbh_get_gc),
    ..ObjectHandlers::OPAQUE
};

/// dbh_get_gc
fn dbh_get_gc(h: &ObjectHandle, out: &mut Vec<Value>) {
    if let Some(d) = object_handlers::payload_of::<Dbh>(h) {
        if let Some(v) = d.def_stmt_ctor_args.borrow().clone() {
            out.push(v);
        }
        if let Some(drv) = d.driver.borrow().clone() {
            drv.gc_values(out);
        }
    }
}

/// `db_handle_factory`.
pub(crate) type Factory =
    fn(&mut Cx, &Dbh, Option<&PhpArray>) -> Result<Option<Rc<dyn DbhDriver>>, Thrown>;

/// A registered driver (`pdo_driver_t`).
pub(crate) struct DriverEntry {
    pub(crate) name: &'static str,
    /// `db_handle_factory`: the connection, or `None` with an error
    /// raised or thrown.
    pub(crate) factory: Factory,
    /// The driver-specific subclass (`php_pdo_register_driver_specific_ce`).
    pub(crate) class: &'static str,
}

/// pdo_driver_hash, in registration order.
pub(crate) fn drivers() -> &'static [DriverEntry] {
    static DRIVERS: [DriverEntry; 1] = [DriverEntry {
        name: "sqlite",
        factory: crate::ext::pdo_sqlite::sqlite_driver::handle_factory,
        class: "Pdo\\Sqlite",
    }];
    &DRIVERS
}

/// `pdo_dbh_t`.
pub(crate) struct Dbh {
    /// The connection (`dbh->driver` with its `methods` and
    /// `driver_data`); `None` until a constructor succeeded.
    pub(crate) driver: RefCell<Option<Rc<dyn DbhDriver>>>,
    pub(crate) driver_name: Cell<&'static str>,
    pub(crate) data_source: RefCell<Vec<u8>>,
    pub(crate) error_code: RefCell<String>,
    pub(crate) error_mode: Cell<i64>,
    pub(crate) desired_case: Cell<i64>,
    pub(crate) native_case: Cell<i64>,
    pub(crate) oracle_nulls: Cell<i64>,
    pub(crate) default_fetch_type: Cell<i64>,
    pub(crate) stringify: Cell<bool>,
    pub(crate) auto_commit: Cell<bool>,
    pub(crate) in_txn: Cell<bool>,
    pub(crate) is_persistent: Cell<bool>,
    pub(crate) alloc_own_columns: Cell<bool>,
    /// Bit `1 << event` set: the driver's param_hook skips that event.
    pub(crate) skip_param_evt: Cell<u32>,
    pub(crate) def_stmt_ce: RefCell<Vec<u8>>,
    pub(crate) def_stmt_ctor_args: RefCell<Option<Value>>,
    /// The statement of a failed `query()`, whose error `errorCode()` and
    /// `errorInfo()` report.
    pub(crate) query_stmt: RefCell<Option<(Value, Rc<Stmt>)>>,
}

impl Dbh {
    fn new() -> Dbh {
        Dbh {
            driver: RefCell::new(None),
            driver_name: Cell::new(""),
            data_source: RefCell::new(Vec::new()),
            error_code: RefCell::new(String::new()),
            error_mode: Cell::new(ERRMODE_SILENT),
            desired_case: Cell::new(CASE_NATURAL),
            native_case: Cell::new(CASE_NATURAL),
            oracle_nulls: Cell::new(0),
            default_fetch_type: Cell::new(FETCH_USE_DEFAULT),
            stringify: Cell::new(false),
            auto_commit: Cell::new(false),
            in_txn: Cell::new(false),
            is_persistent: Cell::new(false),
            alloc_own_columns: Cell::new(false),
            skip_param_evt: Cell::new(0),
            def_stmt_ce: RefCell::new(b"PDOStatement".to_vec()),
            def_stmt_ctor_args: RefCell::new(None),
            query_stmt: RefCell::new(None),
        }
    }

    pub(crate) fn driver(&self) -> Option<Rc<dyn DbhDriver>> {
        self.driver.borrow().clone()
    }

    pub(crate) fn set_error(&self, state: &str) {
        *self.error_code.borrow_mut() = state.to_string();
    }

    /// PDO_DBH_CLEAR_ERR
    pub(crate) fn clear_err(&self) {
        self.set_error(ERR_NONE);
        let q = self.query_stmt.borrow_mut().take();
        drop(q);
    }

    /// pdo_is_in_transaction
    pub(crate) fn in_transaction(&self) -> bool {
        match self.driver() {
            Some(d) => d.in_transaction(self),
            None => self.in_txn.get(),
        }
    }
}

impl Drop for Dbh {
    /// pdo_dbh_free_storage: roll back an open transaction, then close.
    fn drop(&mut self) {
        if let Some(d) = self.driver.get_mut().take() {
            if d.in_transaction(self) {
                d.rollback_on_free();
            }
            d.close();
        }
    }
}

/// A PDO method call: the host, php's active function name for messages,
/// and an exception a callback threw (php's `EG(exception)`), which the
/// method rethrows when it returns.
pub(crate) struct Cx<'a> {
    pub(crate) host: &'a mut dyn Host,
    pub(crate) fname: String,
    pub(crate) pending: Option<Thrown>,
    /// zend_replace_error_handling(EH_THROW, pdo_exception_ce): a warning
    /// becomes a PDOException.
    pub(crate) eh_throw: bool,
}

impl<'a> Cx<'a> {
    pub(crate) fn new(host: &'a mut dyn Host, fname: impl Into<String>) -> Cx<'a> {
        Cx {
            host,
            fname: fname.into(),
            pending: None,
            eh_throw: false,
        }
    }

    /// Run C code whose callbacks may call PHP (sqlite3_step running a user
    /// function). `stop` interrupts it on `exit`.
    pub(crate) fn ffi<R>(&mut self, stop: impl Fn() + 'static, f: impl FnOnce() -> R) -> R {
        let fname = self.fname.clone();
        let (r, t) = ffi::with_host_keep(&mut *self.host, &fname, || {
            ffi::set_stop(stop);
            f()
        });
        if let Some(t) = t {
            if self.pending.is_none() {
                self.pending = Some(t);
            }
        }
        r
    }

    /// `EG(exception)`
    pub(crate) fn exception(&self) -> bool {
        self.pending.is_some()
    }

    /// The method's result with the pending exception, as php returns it.
    pub(crate) fn finish(self, r: Result<Value, Thrown>) -> Result<Value, Thrown> {
        match (r, self.pending) {
            (Ok(_), Some(p)) => Err(p),
            (Err(t), Some(p)) if t.is_catchable() && p.is_catchable() => {
                Err(self.host.chain_thrown(t, p))
            }
            (Err(t), Some(p)) => Err(if p.is_catchable() { t } else { p }),
            (r, None) => r,
        }
    }

    /// php_error_docref(NULL, level, msg) for the running method. While a
    /// callback's exception is pending a user error handler is not called.
    pub(crate) fn docref(&mut self, level: i64, msg: &str) -> Result<(), Thrown> {
        if self.pending.is_some() {
            let handler = self.host.set_error_handler(Value::Null, 0);
            self.host.restore_error_handler();
            if !matches!(handler, Value::Null) {
                return Ok(());
            }
        }
        let m = format!("{}(): {msg}", self.fname);
        if self.eh_throw && level == crate::E_WARNING {
            return Err(exception(self.host, &m, Value::Int(0), None));
        }
        self.host.emit_diagnostic(level, m.as_bytes())
    }

    pub(crate) fn warn(&mut self, msg: &str) -> Result<(), Thrown> {
        self.docref(crate::E_WARNING, msg)
    }

    pub(crate) fn value_error(&self, n: usize, pname: &str, msg: &str) -> Thrown {
        crate::zend::zpp::value_error(&self.fname, n, pname, msg)
    }

    pub(crate) fn type_error_arg(&self, n: usize, pname: &str, msg: &str) -> Thrown {
        Thrown::type_error(format!("{}(): Argument #{n} (${pname}) {msg}", self.fname))
    }

    pub(crate) fn value_name(&self, v: &Value) -> String {
        crate::zend::zpp::zval_value_name(&*self.host, v)
    }
}

/// A `PDOException` with `message` and `code` (php's object_init_ex and
/// property updates; no constructor runs).
pub(crate) fn exception(
    host: &mut dyn Host,
    message: &str,
    code: Value,
    error_info: Option<PhpArray>,
) -> Thrown {
    let obj = match host.reflect_new_instance(b"PDOException", Vec::new(), false) {
        Ok(o) => o,
        Err(t) => return t,
    };
    let r = (|| {
        host.reflect_set_prop(&obj, b"Exception", b"message", Value::str(message))?;
        host.reflect_set_prop(&obj, b"Exception", b"code", code)?;
        if let Some(info) = error_info {
            host.reflect_set_prop(&obj, b"PDOException", b"errorInfo", Value::Array(info))?;
        }
        Ok(())
    })();
    if let Err(t) = r {
        return t;
    }
    match obj {
        Value::Object(h) => Thrown::Object(h),
        _ => Thrown::error("Error", message.to_string()),
    }
}

/// zend_throw_exception_ex(php_pdo_get_exception(), 0, ...)
pub(crate) fn throw(host: &mut dyn Host, message: &str) -> Thrown {
    exception(host, message, Value::Int(0), None)
}

/// pdo_throw_exception: a connection error before the driver is set up.
pub(crate) fn throw_exception(host: &mut dyn Host, code: i64, msg: &str, state: &str) -> Thrown {
    let mut info = PhpArray::new();
    info.push(Value::str(state));
    info.push(Value::Int(code));
    info.push(Value::str(msg));
    exception(
        host,
        &format!("SQLSTATE[{state}] [{code}] {msg}"),
        Value::Int(code),
        Some(info),
    )
}

/// pdo_raise_impl_error
pub(crate) fn raise_impl_error(
    cx: &mut Cx,
    dbh: &Dbh,
    stmt: Option<&Stmt>,
    state: &str,
    supp: Option<&str>,
) -> Result<(), Thrown> {
    match stmt {
        Some(s) => s.set_error(state),
        None => dbh.set_error(state),
    }
    let msg = sqlstate::description(state).unwrap_or("<<Unknown error>>");
    let message = match supp {
        Some(s) => format!("SQLSTATE[{state}]: {msg}: {s}"),
        None => format!("SQLSTATE[{state}]: {msg}"),
    };
    if dbh.error_mode.get() != ERRMODE_EXCEPTION {
        return cx.warn(&message);
    }
    let mut info = PhpArray::new();
    info.push(Value::str(state));
    info.push(Value::Int(0));
    Err(exception(cx.host, &message, Value::str(state), Some(info)))
}

/// pdo_handle_error
pub(crate) fn handle_error(cx: &mut Cx, dbh: &Dbh, stmt: Option<&Stmt>) -> Result<(), Thrown> {
    if dbh.error_mode.get() == ERRMODE_SILENT {
        return Ok(());
    }
    let state = match stmt {
        Some(s) => s.error_code.borrow().clone(),
        None => dbh.error_code.borrow().clone(),
    };
    let msg = sqlstate::description(&state).unwrap_or("<<Unknown error>>");
    let mut info = PhpArray::new();
    info.push(Value::str(state.as_str()));
    let mut native = 0;
    let mut supp = None;
    if let Some(d) = dbh.driver() {
        d.fetch_err(dbh, stmt, &mut info);
        if let Some(Value::Int(n)) = info.get(&rhp_values::ArrayKey::Int(1)) {
            native = *n;
        }
        if let Some(v) = info.get(&rhp_values::ArrayKey::Int(2)) {
            supp = Some(String::from_utf8_lossy(&conv::to_string_bytes(v)).into_owned());
        }
    }
    let message = match (native, &supp) {
        (n, Some(s)) if n != 0 => format!("SQLSTATE[{state}]: {msg}: {n} {s}"),
        (_, Some(s)) => format!("SQLSTATE[{state}]: {msg}: {s}"),
        _ => format!("SQLSTATE[{state}]: {msg}"),
    };
    if dbh.error_mode.get() == ERRMODE_WARNING {
        return cx.warn(&message);
    }
    if cx.exception() {
        return Ok(());
    }
    Err(exception(
        cx.host,
        &message,
        Value::str(state.as_str()),
        Some(info),
    ))
}

/// PDO_HANDLE_DBH_ERR
pub(crate) fn handle_dbh_err(cx: &mut Cx, dbh: &Dbh) -> Result<(), Thrown> {
    if *dbh.error_code.borrow() != ERR_NONE {
        return handle_error(cx, dbh, None);
    }
    Ok(())
}

/// The state of `this`, a PDO (or subclass) object, created on first use.
pub(crate) fn dbh_of(host: &dyn Host, this: &Value) -> Option<Rc<Dbh>> {
    if !object_handlers::has_handlers(host, this, &DBH_HANDLERS) {
        return None;
    }
    let h = object_handlers::object_of(this)?;
    if let Some(d) = object_handlers::payload_of::<Dbh>(&h) {
        return Some(d);
    }
    let d = Rc::new(Dbh::new());
    object_handlers::set_payload(&h, d.clone());
    Some(d)
}

/// The class name php's messages give an object.
pub(crate) fn class_of(host: &dyn Host, this: &Value) -> String {
    host.object_class_name(this)
        .map(|n| String::from_utf8_lossy(&n).into_owned())
        .unwrap_or_default()
}

/// `pdo_drivers()` and `PDO::getAvailableDrivers()`.
pub(crate) fn driver_names() -> Value {
    let mut a = PhpArray::new();
    for d in drivers() {
        a.push(Value::str(d.name));
    }
    Value::Array(a)
}

fn fn_pdo_drivers(_host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(driver_names())
}

/// PHP_MINFO_FUNCTION(pdo)
pub fn info_rows() -> Vec<(String, String)> {
    let names: Vec<&str> = drivers().iter().map(|d| d.name).collect();
    vec![
        ("PDO support".into(), "enabled".into()),
        ("PDO drivers".into(), names.join(", ")),
    ]
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("pdo_drivers", fn_pdo_drivers)]);
    crate::register_helpers(
        m,
        &[
            ("__pdo", dbh::fn_pdo, 2, None),
            ("__pdo_stmt", stmt::fn_pdo_stmt, 2, None),
            ("__pdo_row", stmt::fn_pdo_row, 3, None),
        ],
    );
}

/// The E_DEPRECATED of the `PDO::SQLITE_*` constants pdo_sqlite declares
/// on PDO (REGISTER_PDO_CLASS_CONST_LONG_DEPRECATED_ALIAS_85).
pub(crate) fn class_constant_deprecation(class: &[u8], name: &[u8]) -> Option<String> {
    if !class.eq_ignore_ascii_case(b"PDO") {
        return None;
    }
    let base = name.strip_prefix(b"SQLITE_")?;
    let base = String::from_utf8_lossy(base);
    Some(format!(
        "Constant PDO::SQLITE_{base} is deprecated since 8.5, use Pdo\\Sqlite::{base} instead"
    ))
}
