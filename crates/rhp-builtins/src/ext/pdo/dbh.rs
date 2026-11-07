//! ext/pdo/pdo_dbh.c: the `PDO` class.

use std::rc::Rc;

use rhp_values::{conv, ArrayKey, PhpArray, Value};

use super::driver::*;
use super::stmt::{self, describe_columns, handle_stmt_err, lookup_class, stmt_of, Stmt};
use super::{
    class_of, dbh_of, deref, drivers, handle_dbh_err, raise_impl_error, throw, Cx, Dbh, DriverEntry,
};
use crate::{Host, Thrown};

fn arg(a: &[Value], i: usize) -> Value {
    a.get(i).map(deref).unwrap_or(Value::Null)
}

/// `__pdo($method, $this, ...$args)`: PDO's methods (and Pdo\Sqlite's
/// inherited ones). For the static ones `$this` is the called class.
pub(super) fn fn_pdo(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let method = String::from_utf8_lossy(&conv::to_string_bytes(&deref(&args[0]))).into_owned();
    let this = deref(&args[1]);
    let a = &args[2..];
    let mut cx = Cx::new(host, format!("PDO::{method}"));
    let r = dbh_method(&mut cx, &method, &this, a);
    cx.finish(r)
}

/// PDO_CONSTRUCT_CHECK
pub(crate) fn construct_check(host: &dyn Host, this: &Value) -> Result<Rc<Dbh>, Thrown> {
    let d = dbh_of(host, this).ok_or_else(|| Thrown::type_error("not a PDO object"))?;
    if d.driver.borrow().is_none() {
        return Err(Thrown::error(
            "Error",
            format!("{} object is uninitialized", class_of(host, this)),
        ));
    }
    Ok(d)
}

fn dbh_method(cx: &mut Cx, method: &str, this: &Value, a: &[Value]) -> Result<Value, Thrown> {
    let b = Value::Bool;
    match method {
        "__construct" => {
            let class = cx.host.object_class_name(this).unwrap_or_default();
            construct_driver(cx, Some(this), &class, a).map(|_| Value::Null)
        }
        "connect" => {
            let class = conv::to_string_bytes(this);
            construct_driver(cx, None, &class, a)
        }
        "getAvailableDrivers" => Ok(super::driver_names()),
        "prepare" => prepare(cx, this, a),
        "query" => query(cx, this, a),
        _ => {
            let dbh = construct_check(&*cx.host, this)?;
            let drv = dbh.driver().expect("checked");
            match method {
                "beginTransaction" => {
                    if dbh.in_transaction() {
                        return Err(throw(cx.host, "There is already an active transaction"));
                    }
                    if drv.begin(cx, &dbh) {
                        dbh.in_txn.set(true);
                        return Ok(b(true));
                    }
                    handle_dbh_err(cx, &dbh)?;
                    Ok(b(false))
                }
                "commit" | "rollBack" => {
                    if !dbh.in_transaction() {
                        return Err(throw(cx.host, "There is no active transaction"));
                    }
                    let ok = if method == "commit" {
                        drv.commit(cx, &dbh)
                    } else {
                        drv.rollback(cx, &dbh)
                    };
                    if ok {
                        dbh.in_txn.set(false);
                        return Ok(b(true));
                    }
                    handle_dbh_err(cx, &dbh)?;
                    Ok(b(false))
                }
                "inTransaction" => Ok(b(dbh.in_transaction())),
                "setAttribute" => {
                    dbh.clear_err();
                    let attr = conv::to_int(&arg(a, 0));
                    Ok(b(attribute_set(cx, &dbh, attr, &arg(a, 1), 2)?))
                }
                "getAttribute" => get_attribute(cx, &dbh, conv::to_int(&arg(a, 0))),
                "exec" => {
                    let sql = conv::to_string_bytes(&arg(a, 0));
                    if sql.is_empty() {
                        return Err(cx.value_error(1, "statement", "must not be empty"));
                    }
                    dbh.clear_err();
                    let n = drv.exec(cx, &dbh, &sql);
                    if n == -1 {
                        handle_dbh_err(cx, &dbh)?;
                        return Ok(b(false));
                    }
                    Ok(Value::Int(n))
                }
                "lastInsertId" => {
                    dbh.clear_err();
                    let name = match arg(a, 0) {
                        Value::Null => None,
                        v => Some(conv::to_string_bytes(&v)),
                    };
                    match drv.last_id(&dbh, name.as_deref()) {
                        Some(id) => Ok(Value::str(id)),
                        None => {
                            handle_dbh_err(cx, &dbh)?;
                            Ok(b(false))
                        }
                    }
                }
                "errorCode" => {
                    if let Some((_, s)) = dbh.query_stmt.borrow().as_ref() {
                        return Ok(Value::str(s.error_code.borrow().as_str()));
                    }
                    let c = dbh.error_code.borrow().clone();
                    Ok(if c.is_empty() {
                        Value::Null
                    } else {
                        Value::str(c)
                    })
                }
                "errorInfo" => {
                    let mut out = PhpArray::new();
                    let q = dbh.query_stmt.borrow().as_ref().map(|(_, s)| s.clone());
                    let code = match &q {
                        Some(s) => s.error_code.borrow().clone(),
                        None => dbh.error_code.borrow().clone(),
                    };
                    out.push(Value::str(code.as_str()));
                    if code != ERR_NONE {
                        drv.fetch_err(&dbh, q.as_deref(), &mut out);
                    }
                    while out.len() < 3 {
                        out.push(Value::Null);
                    }
                    Ok(Value::Array(out))
                }
                "quote" => {
                    dbh.clear_err();
                    let s = conv::to_string_bytes(&arg(a, 0));
                    let ty = match a.get(1) {
                        Some(v) => conv::to_int(&deref(v)),
                        None => PARAM_STR,
                    };
                    match drv.quote(cx, &dbh, &s, ty)? {
                        Some(q) => Ok(Value::str(q)),
                        None => {
                            handle_dbh_err(cx, &dbh)?;
                            Ok(b(false))
                        }
                    }
                }
                _ => Err(Thrown::error(
                    "Error",
                    format!("Call to undefined method PDO::{method}()"),
                )),
            }
        }
    }
}

/// create_driver_specific_pdo_object: the object to connect (a new one
/// for `connect()`), or an exception.
fn driver_specific_object(
    cx: &mut Cx,
    driver: &DriverEntry,
    called: &[u8],
    new_object: bool,
) -> Result<Option<Value>, Thrown> {
    let verb = if new_object { "connect" } else { "__construct" };
    let called_name = String::from_utf8_lossy(called).into_owned();
    let is_pdo = called.eq_ignore_ascii_case(b"PDO");
    let by_called = !is_pdo
        && drivers()
            .iter()
            .any(|d| cx.host.reflect_is_a(called, d.class.as_bytes()));
    let new = |cx: &mut Cx, cls: &[u8]| cx.host.reflect_new_instance(cls, Vec::new(), false);
    if by_called {
        if !cx.host.reflect_is_a(called, driver.class.as_bytes()) {
            let msg = format!(
                "{called_name}::{verb}() cannot be used for connecting to the \"{}\" driver, \
                 either call {}::{verb}() or PDO::{verb}() instead",
                driver.name, driver.class
            );
            return Err(throw(cx.host, &msg));
        }
        return Ok(if new_object {
            Some(new(cx, called)?)
        } else {
            None
        });
    }
    if !is_pdo && !new_object {
        return Ok(None);
    }
    if !is_pdo {
        let msg = format!(
            "{called_name}::{verb}() cannot be used for connecting to the \"{}\" driver, \
             either call {}::{verb}() or PDO::{verb}() instead",
            driver.name, driver.class
        );
        return Err(throw(cx.host, &msg));
    }
    Ok(if new_object {
        Some(new(cx, driver.class.as_bytes())?)
    } else {
        None
    })
}

/// php_pdo_internal_construct_driver
fn construct_driver(
    cx: &mut Cx,
    current: Option<&Value>,
    called: &[u8],
    a: &[Value],
) -> Result<Value, Thrown> {
    let mut dsn = conv::to_string_bytes(&arg(a, 0));
    let username = match arg(a, 1) {
        Value::Null => None,
        v => Some(conv::to_string_bytes(&v)),
    };
    let options = match arg(a, 3) {
        Value::Array(o) => Some(o),
        _ => None,
    };
    let _ = username;
    let mut colon = dsn.iter().position(|&c| c == b':');
    if colon.is_none() {
        let alt = format!("pdo.dsn.{}", String::from_utf8_lossy(&dsn));
        match crate::main::ini::cfg_get(alt.as_bytes()) {
            Some(v) => {
                dsn = conv::to_string_bytes(&v);
                colon = dsn.iter().position(|&c| c == b':');
                if colon.is_none() {
                    return Err(throw(
                        cx.host,
                        &format!("invalid data source name (via INI: {alt})"),
                    ));
                }
            }
            None => {
                let msg = format!(
                    "{}(): Argument #1 ($dsn) must be a valid data source name",
                    cx.fname
                );
                return Err(throw(cx.host, &msg));
            }
        }
    }
    if dsn.starts_with(b"uri:") {
        cx.host.emit_diagnostic(
            crate::E_DEPRECATED,
            b"Looking up the DSN from a URI is deprecated due to possible security concerns with DSNs coming from remote URIs",
        )?;
        let uri = dsn[4..].to_vec();
        if uri.is_empty() {
            return Err(Thrown::error("ValueError", "Path must not be empty"));
        }
        // php_stream_open_wrapper with REPORT_ERRORS
        let line = match cx.host.stream_open(&uri, b"rb") {
            Some(s) => {
                let l = cx.host.stream_gets_max(&s, 511);
                cx.host.stream_close(&s);
                l
            }
            None => {
                let fname = cx.fname.clone();
                crate::open_failed(cx.host, &fname, &uri)?;
                None
            }
        };
        let Some(line) = line else {
            let msg = format!(
                "{}(): Argument #1 ($dsn) must be a valid data source URI",
                cx.fname
            );
            return Err(throw(cx.host, &msg));
        };
        dsn = line;
        colon = dsn.iter().position(|&c| c == b':');
        if colon.is_none() {
            let msg = format!(
                "{}(): Argument #1 ($dsn) must be a valid data source name (via URI)",
                cx.fname
            );
            return Err(throw(cx.host, &msg));
        }
    }
    let colon = colon.expect("checked");
    let Some(driver) = drivers()
        .iter()
        .find(|d| d.name.as_bytes() == &dsn[..colon])
    else {
        return Err(throw(cx.host, "could not find driver"));
    };
    let new_obj = driver_specific_object(cx, driver, called, current.is_none())?;
    let this = match (&new_obj, current) {
        (Some(o), _) => o.clone(),
        (None, Some(c)) => c.clone(),
        (None, None) => return Err(Thrown::error("Error", "no object")),
    };
    let dbh = dbh_of(&*cx.host, &this).ok_or_else(|| Thrown::type_error("not a PDO object"))?;

    let persistent = options
        .as_ref()
        .and_then(|o| o.get(&ArrayKey::Int(ATTR_PERSISTENT)))
        .map(deref)
        .is_some_and(|v| match &v {
            Value::Str(s) if !s.is_empty() && !conv::is_numeric(&v) => true,
            other => conv::to_int(other) != 0,
        });
    *dbh.data_source.borrow_mut() = dsn[colon + 1..].to_vec();
    let end = dbh.data_source.borrow().iter().position(|&c| c == 0);
    if let Some(e) = end {
        dbh.data_source.borrow_mut().truncate(e);
    }
    dbh.default_fetch_type.set(FETCH_BOTH);
    dbh.is_persistent.set(persistent);
    dbh.auto_commit
        .set(attr_lval(options.as_ref(), ATTR_AUTOCOMMIT, 1) != 0);
    dbh.error_mode
        .set(attr_lval(options.as_ref(), ATTR_ERRMODE, ERRMODE_EXCEPTION));

    // zend_replace_error_handling(EH_THROW, pdo_exception_ce): warnings
    // become PDOExceptions while the connection is set up.
    cx.eh_throw = true;
    let r = (|| -> Result<bool, Thrown> {
        let Some(conn) = (driver.factory)(cx, &dbh, options.as_ref())? else {
            return Ok(false);
        };
        *dbh.driver.borrow_mut() = Some(conn);
        dbh.driver_name.set(driver.name);
        if let Some(o) = &options {
            for (k, v) in o.iter() {
                if let ArrayKey::Int(attr) = k {
                    attribute_set(cx, &dbh, attr, &deref(v), 3)?;
                }
            }
        }
        Ok(true)
    })();
    cx.eh_throw = false;
    match r {
        Ok(true) => Ok(new_obj.unwrap_or(Value::Null)),
        Ok(false) => {
            if cx.exception() {
                Ok(Value::Null)
            } else {
                Err(throw(cx.host, "Constructor failed"))
            }
        }
        Err(t) => Err(t),
    }
}

/// pdo_get_long_param
pub(crate) fn get_long_param(cx: &mut Cx, v: &Value) -> Result<i64, Thrown> {
    match v {
        Value::Int(n) => Ok(*n),
        Value::Bool(b) => Ok(*b as i64),
        Value::Str(s) => match conv::numeric_string(s.as_bytes()) {
            Some(rhp_values::conv::Num::Int(n)) => Ok(n),
            _ => Err(attr_type_error(cx, "int", v)),
        },
        _ => Err(attr_type_error(cx, "int", v)),
    }
}

fn attr_type_error(cx: &Cx, ty: &str, v: &Value) -> Thrown {
    Thrown::type_error(format!(
        "Attribute value must be of type {ty} for selected attribute, {} given",
        cx.value_name(v)
    ))
}

/// pdo_get_bool_param
pub(crate) fn get_bool_param(cx: &mut Cx, v: &Value) -> Result<bool, Thrown> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::Int(n) => Ok(*n != 0),
        _ => Err(attr_type_error(cx, "bool", v)),
    }
}

/// zend_argument_value_error for the attribute's value argument. From the
/// constructor's options php passes 3, which names `$password`.
fn value_arg_error(cx: &Cx, n: usize, msg: &str) -> Thrown {
    let pname = if n == 3 { "password" } else { "value" };
    cx.value_error(n, pname, msg)
}

fn value_arg_type_error(cx: &Cx, n: usize, msg: &str) -> Thrown {
    let pname = if n == 3 { "password" } else { "value" };
    cx.type_error_arg(n, pname, msg)
}

/// pdo_dbh_attribute_set
pub(crate) fn attribute_set(
    cx: &mut Cx,
    dbh: &Dbh,
    attr: i64,
    v: &Value,
    argn: usize,
) -> Result<bool, Thrown> {
    match attr {
        ATTR_ERRMODE => {
            let l = get_long_param(cx, v)?;
            if !(ERRMODE_SILENT..=ERRMODE_EXCEPTION).contains(&l) {
                return Err(value_arg_error(
                    cx,
                    argn,
                    "Error mode must be one of the PDO::ERRMODE_* constants",
                ));
            }
            dbh.error_mode.set(l);
            return Ok(true);
        }
        ATTR_CASE => {
            let l = get_long_param(cx, v)?;
            if !(CASE_NATURAL..=CASE_LOWER).contains(&l) {
                return Err(value_arg_error(
                    cx,
                    argn,
                    "Case folding mode must be one of the PDO::CASE_* constants",
                ));
            }
            dbh.desired_case.set(l);
            return Ok(true);
        }
        ATTR_ORACLE_NULLS => {
            let l = get_long_param(cx, v)?;
            dbh.oracle_nulls.set(l);
            return Ok(true);
        }
        ATTR_DEFAULT_FETCH_MODE => {
            let l = if let Value::Array(arr) = v {
                if let Some(Value::Int(m)) = arr.get(&ArrayKey::Int(0)) {
                    if *m == FETCH_INTO || *m == FETCH_CLASS {
                        return Err(value_arg_error(
                            cx,
                            argn,
                            "PDO::FETCH_INTO and PDO::FETCH_CLASS cannot be set as the default fetch mode",
                        ));
                    }
                }
                conv::to_int(v)
            } else {
                get_long_param(cx, v)?
            };
            if l == FETCH_USE_DEFAULT {
                return Err(value_arg_error(
                    cx,
                    argn,
                    "Fetch mode must be a bitmask of PDO::FETCH_* constants",
                ));
            }
            dbh.default_fetch_type.set(l);
            return Ok(true);
        }
        ATTR_STRINGIFY_FETCHES => {
            let b = get_bool_param(cx, v)?;
            dbh.stringify.set(b);
            if let Some(d) = dbh.driver() {
                d.set_attribute(cx, dbh, attr, v)?;
            }
            return Ok(true);
        }
        ATTR_STATEMENT_CLASS => {
            if dbh.is_persistent.get() {
                raise_impl_error(
                    cx,
                    dbh,
                    None,
                    "HY000",
                    Some("PDO::ATTR_STATEMENT_CLASS cannot be used with persistent PDO instances"),
                )?;
                handle_dbh_err(cx, dbh)?;
                return Ok(false);
            }
            let Value::Array(arr) = v else {
                let msg = format!(
                    "PDO::ATTR_STATEMENT_CLASS value must be of type array, {} given",
                    cx.value_name(v)
                );
                return Err(value_arg_type_error(cx, argn, &msg));
            };
            let (cls, ctor) = statement_class(cx, arr, |cx, m, is_type| {
                if is_type {
                    value_arg_type_error(cx, argn, m)
                } else {
                    value_arg_error(cx, argn, m)
                }
            })?;
            *dbh.def_stmt_ce.borrow_mut() = cls;
            *dbh.def_stmt_ctor_args.borrow_mut() = ctor;
            return Ok(true);
        }
        _ => {}
    }
    let drv = dbh.driver().expect("connected");
    dbh.clear_err();
    if drv.set_attribute(cx, dbh, attr, v)? {
        return Ok(true);
    }
    handle_dbh_err(cx, dbh)?;
    Ok(false)
}

/// The checks of an ATTR_STATEMENT_CLASS value: the class name and its
/// constructor arguments.
fn statement_class(
    cx: &mut Cx,
    arr: &PhpArray,
    err: impl Fn(&Cx, &str, bool) -> Thrown,
) -> Result<(Vec<u8>, Option<Value>), Thrown> {
    let Some(item) = arr.get(&ArrayKey::Int(0)).map(deref) else {
        return Err(err(
            cx,
            "PDO::ATTR_STATEMENT_CLASS value must be an array with the format array(classname, constructor_args)",
            false,
        ));
    };
    let cls = match &item {
        Value::Str(s) => lookup_class(cx, s.as_bytes())?,
        _ => None,
    };
    let Some(cls) = cls else {
        return Err(err(
            cx,
            "PDO::ATTR_STATEMENT_CLASS class must be a valid class",
            true,
        ));
    };
    if !cx.host.reflect_is_a(&cls, b"PDOStatement") {
        return Err(err(
            cx,
            "PDO::ATTR_STATEMENT_CLASS class must be derived from PDOStatement",
            true,
        ));
    }
    if ctor_visibility(cx, &cls)? == Some(1) {
        return Err(err(
            cx,
            "User-supplied statement class cannot have a public constructor",
            true,
        ));
    }
    let ctor = match arr.get(&ArrayKey::Int(1)).map(deref) {
        None => None,
        Some(Value::Array(c)) => Some(Value::Array(c)),
        Some(other) => {
            let msg = format!(
                "PDO::ATTR_STATEMENT_CLASS constructor_args must be of type ?array, {} given",
                cx.value_name(&other)
            );
            return Err(err(cx, &msg, true));
        }
    };
    Ok((cls, ctor))
}

/// The visibility flag of class `cls`'s constructor (1 public, 2
/// protected, 4 private), `None` without one.
fn ctor_visibility(cx: &mut Cx, cls: &[u8]) -> Result<Option<i64>, Thrown> {
    let Some(cid) = cx.host.resolve_class(cls) else {
        return Ok(None);
    };
    if !cx.host.class_has_method(cid, b"__construct") {
        return Ok(None);
    }
    let mut pair = PhpArray::new();
    pair.push(Value::str(cls));
    pair.push(Value::str("__construct"));
    let info = cx.host.reflect_op(b"fn", &[Value::Array(pair)])?;
    Ok(match info {
        Value::Array(a) => a.get(&ArrayKey::from_bytes(b"vis")).map(conv::to_int),
        _ => None,
    })
}

/// PDO::getAttribute
fn get_attribute(cx: &mut Cx, dbh: &Dbh, attr: i64) -> Result<Value, Thrown> {
    dbh.clear_err();
    match attr {
        ATTR_PERSISTENT => return Ok(Value::Bool(dbh.is_persistent.get())),
        ATTR_CASE => return Ok(Value::Int(dbh.desired_case.get())),
        ATTR_ORACLE_NULLS => return Ok(Value::Int(dbh.oracle_nulls.get())),
        ATTR_ERRMODE => return Ok(Value::Int(dbh.error_mode.get())),
        ATTR_DRIVER_NAME => return Ok(Value::str(dbh.driver_name.get())),
        ATTR_STATEMENT_CLASS => {
            let mut a = PhpArray::new();
            a.push(Value::str(dbh.def_stmt_ce.borrow().clone()));
            if let Some(c) = dbh.def_stmt_ctor_args.borrow().clone() {
                a.push(c);
            }
            return Ok(Value::Array(a));
        }
        ATTR_DEFAULT_FETCH_MODE => return Ok(Value::Int(dbh.default_fetch_type.get())),
        ATTR_STRINGIFY_FETCHES => return Ok(Value::Bool(dbh.stringify.get())),
        _ => {}
    }
    let drv = dbh.driver().expect("connected");
    match drv.get_attribute(dbh, attr) {
        (-1, _) => {
            handle_dbh_err(cx, dbh)?;
            Ok(Value::Bool(false))
        }
        (0, _) => {
            raise_impl_error(
                cx,
                dbh,
                None,
                "IM001",
                Some("driver does not support that attribute"),
            )?;
            Ok(Value::Bool(false))
        }
        (_, v) => Ok(v),
    }
}

/// pdo_stmt_instantiate and the statement's state.
fn stmt_instantiate(
    cx: &mut Cx,
    dbh: &Rc<Dbh>,
    this: &Value,
    cls: &[u8],
    ctor_args: &Option<Value>,
    sql: &[u8],
) -> Result<(Value, Rc<Stmt>), Thrown> {
    if ctor_args.is_some() && ctor_visibility(cx, cls)?.is_none() {
        return Err(Thrown::error(
            "Error",
            "User-supplied statement does not accept constructor arguments",
        ));
    }
    let obj = cx.host.reflect_new_instance(cls, Vec::new(), false)?;
    let st = stmt_of(&*cx.host, &obj).ok_or_else(|| {
        Thrown::error("Error", "Cannot instantiate user-supplied statement class")
    })?;
    *st.query_string.borrow_mut() = sql.to_vec();
    st.default_fetch_type.set(dbh.default_fetch_type.get());
    *st.dbh.borrow_mut() = Some(dbh.clone());
    *st.database_object.borrow_mut() = Some(this.clone());
    Ok((obj, st))
}

/// pdo_stmt_construct
fn stmt_construct(
    cx: &mut Cx,
    obj: &Value,
    st: &Stmt,
    cls: &[u8],
    ctor_args: &Option<Value>,
) -> Result<(), Thrown> {
    let q = Value::str(st.query_string.borrow().clone());
    cx.host
        .reflect_set_prop(obj, b"PDOStatement", b"queryString", q)?;
    if ctor_visibility(cx, cls)?.is_some() {
        let args = ctor_args.clone().unwrap_or(Value::Array(PhpArray::new()));
        cx.host.reflect_invoke(obj, cls, b"__construct", args)?;
    }
    Ok(())
}

/// PDO::prepare
fn prepare(cx: &mut Cx, this: &Value, a: &[Value]) -> Result<Value, Thrown> {
    let dbh = construct_check(&*cx.host, this)?;
    let sql = conv::to_string_bytes(&arg(a, 0));
    let options = match arg(a, 1) {
        Value::Array(o) => Some(o),
        _ => None,
    };
    if sql.is_empty() {
        return Err(cx.value_error(1, "query", "must not be empty"));
    }
    dbh.clear_err();
    let (cls, ctor) = match options
        .as_ref()
        .and_then(|o| o.get(&ArrayKey::Int(ATTR_STATEMENT_CLASS)))
        .map(deref)
    {
        Some(Value::Array(v)) => statement_class(cx, &v, |_, m, is_type| {
            if is_type {
                Thrown::type_error(m.to_string())
            } else {
                Thrown::error("ValueError", m.to_string())
            }
        })?,
        Some(other) => {
            return Err(Thrown::type_error(format!(
                "PDO::ATTR_STATEMENT_CLASS value must be of type array, {} given",
                cx.value_name(&other)
            )))
        }
        None => (
            dbh.def_stmt_ce.borrow().clone(),
            dbh.def_stmt_ctor_args.borrow().clone(),
        ),
    };
    let (obj, st) = stmt_instantiate(cx, &dbh, this, &cls, &ctor, &sql)?;
    let drv = dbh.driver().expect("checked");
    if drv.prepare(cx, &dbh, &sql, &st, options.as_ref()) {
        stmt_construct(cx, &obj, &st, &cls, &ctor)?;
        return Ok(obj);
    }
    handle_dbh_err(cx, &dbh)?;
    Ok(Value::Bool(false))
}

/// PDO::query
fn query(cx: &mut Cx, this: &Value, a: &[Value]) -> Result<Value, Thrown> {
    let dbh = construct_check(&*cx.host, this)?;
    let nargs = conv::to_int(&arg(a, 0)) as usize;
    let sql = conv::to_string_bytes(&arg(a, 1));
    let mode = match arg(a, 2) {
        Value::Null => None,
        v => Some(conv::to_int(&v)),
    };
    let rest: Vec<Value> = a
        .iter()
        .skip(3)
        .take(nargs.saturating_sub(2))
        .cloned()
        .collect();
    if sql.is_empty() {
        return Err(cx.value_error(1, "query", "must not be empty"));
    }
    dbh.clear_err();
    let cls = dbh.def_stmt_ce.borrow().clone();
    let ctor = dbh.def_stmt_ctor_args.borrow().clone();
    let (obj, st) = stmt_instantiate(cx, &dbh, this, &cls, &ctor, &sql)?;
    *st.active_query_string.borrow_mut() = Some(sql.clone());
    let drv = dbh.driver().expect("checked");
    if drv.prepare(cx, &dbh, &sql, &st, None) {
        st.clear_err();
        let ok_mode = match mode {
            None => true,
            Some(m) => {
                stmt::setup_fetch_mode(cx, &st, m, 2, &rest)?;
                true
            }
        };
        if ok_mode {
            st.clear_err();
            let sdrv = st.driver.borrow().clone().expect("prepared");
            if sdrv.execute(cx, &st) {
                let mut ret = true;
                if !st.executed.get() {
                    if dbh.alloc_own_columns.get() {
                        ret = describe_columns(cx, &st);
                    }
                    st.executed.set(true);
                }
                if ret {
                    stmt_construct(cx, &obj, &st, &cls, &ctor)?;
                    return Ok(obj);
                }
            }
        }
        // something broke
        *st.database_object.borrow_mut() = None;
        *dbh.query_stmt.borrow_mut() = Some((obj, st.clone()));
        handle_stmt_err(cx, &dbh, &st)?;
    } else {
        handle_dbh_err(cx, &dbh)?;
    }
    Ok(Value::Bool(false))
}
