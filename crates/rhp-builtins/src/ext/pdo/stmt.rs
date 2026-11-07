//! ext/pdo/pdo_stmt.c: `PDOStatement` and `PDORow`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use indexmap::IndexMap;
use rhp_values::{conv, ArrayKey, ObjectHandle, PhpArray, PhpStr, Value};

use super::driver::*;
use super::{class_of, deref, handle_error, raise_impl_error, Cx, Dbh};
use crate::zend::internal_iterator::IteratorFuncs;
use crate::zend::object_handlers::{self, ObjectHandlers};
use crate::{Host, Thrown};

/// pdo_dbstmt_object_handlers
pub(crate) static STMT_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_iterator: Some(stmt_iter_get),
    get_gc: Some(stmt_get_gc),
    write_property: Some(stmt_prop_write),
    unset_property: Some(stmt_prop_delete),
    ..ObjectHandlers::OPAQUE
};

/// pdo_row_object_handlers
pub(crate) static ROW_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(row_get_properties_for),
    dimension: Some(&ROW_DIMS),
    ..ObjectHandlers::OPAQUE
};

static ROW_DIMS: object_handlers::DimensionHandlers = object_handlers::DimensionHandlers {
    read: row_dim_read,
    write: row_dim_write,
    has: row_dim_exists,
    unset: row_dim_delete,
};

/// A column (`struct pdo_column_data`).
#[derive(Clone)]
pub(crate) struct Column {
    pub(crate) name: Vec<u8>,
    pub(crate) maxlen: i64,
    pub(crate) precision: i64,
}

/// A bound parameter or column (`struct pdo_bound_param_data`).
#[derive(Clone)]
pub(crate) struct BoundParam {
    pub(crate) paramno: i64,
    pub(crate) name: Option<Vec<u8>>,
    pub(crate) max_value_len: i64,
    pub(crate) param_type: i64,
    pub(crate) is_param: bool,
    /// The value, or for bindParam/bindColumn the variable's reference.
    pub(crate) parameter: Value,
    pub(crate) driver_params: Option<Value>,
}

impl BoundParam {
    /// The value (through the reference).
    pub(crate) fn value(&self) -> Value {
        deref(&self.parameter)
    }

    /// Replace the value (the variable's, for a reference).
    pub(crate) fn set_value(&mut self, v: Value) {
        match &self.parameter {
            Value::Ref(r) => *r.borrow_mut() = v,
            _ => self.parameter = v,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) enum PKey {
    Name(Vec<u8>),
    Num(i64),
}

pub(crate) type Params = IndexMap<PKey, BoundParam>;

/// The default fetch mode's arguments (`stmt->fetch`).
#[derive(Default, Clone)]
pub(crate) struct Fetch {
    pub(crate) column: i64,
    pub(crate) cls: Option<Vec<u8>>,
    pub(crate) ctor_args: Option<PhpArray>,
    pub(crate) into: Option<Value>,
    pub(crate) func: Option<Value>,
}

/// `pdo_stmt_t`.
pub(crate) struct Stmt {
    pub(crate) dbh: RefCell<Option<Rc<Dbh>>>,
    /// The PDO object (`database_object_handle`), kept alive by its
    /// statements.
    pub(crate) database_object: RefCell<Option<Value>>,
    pub(crate) driver: RefCell<Option<Rc<dyn StmtDriver>>>,
    pub(crate) query_string: RefCell<Vec<u8>>,
    pub(crate) active_query_string: RefCell<Option<Vec<u8>>>,
    pub(crate) supports_placeholders: Cell<u8>,
    pub(crate) columns: RefCell<Option<Vec<Column>>>,
    pub(crate) column_count: Cell<i32>,
    pub(crate) row_count: Cell<i64>,
    pub(crate) executed: Cell<bool>,
    pub(crate) error_code: RefCell<String>,
    pub(crate) bound_params: RefCell<Option<Params>>,
    pub(crate) bound_columns: RefCell<Option<Params>>,
    pub(crate) default_fetch_type: Cell<i64>,
    pub(crate) fetch: RefCell<Fetch>,
    pub(crate) in_fetch: Cell<bool>,
    pub(crate) in_param_event: Cell<bool>,
    /// The PDORow of FETCH_LAZY, while it lives.
    lazy_object: RefCell<Option<std::rc::Weak<RefCell<rhp_values::ObjectData>>>>,
}

impl Stmt {
    fn new() -> Stmt {
        Stmt {
            dbh: RefCell::new(None),
            database_object: RefCell::new(None),
            driver: RefCell::new(None),
            query_string: RefCell::new(Vec::new()),
            active_query_string: RefCell::new(None),
            supports_placeholders: Cell::new(0),
            columns: RefCell::new(None),
            column_count: Cell::new(0),
            row_count: Cell::new(0),
            executed: Cell::new(false),
            error_code: RefCell::new(String::new()),
            bound_params: RefCell::new(None),
            bound_columns: RefCell::new(None),
            default_fetch_type: Cell::new(FETCH_USE_DEFAULT),
            fetch: RefCell::new(Fetch::default()),
            in_fetch: Cell::new(false),
            in_param_event: Cell::new(false),
            lazy_object: RefCell::new(None),
        }
    }

    pub(crate) fn set_error(&self, state: &str) {
        *self.error_code.borrow_mut() = state.to_string();
    }

    /// PDO_STMT_CLEAR_ERR
    pub(crate) fn clear_err(&self) {
        self.set_error(ERR_NONE);
    }

    fn has_error(&self) -> bool {
        *self.error_code.borrow() != ERR_NONE
    }

    pub(crate) fn dbh(&self) -> Rc<Dbh> {
        self.dbh.borrow().clone().expect("statement of a PDO")
    }

    fn drv(&self) -> Rc<dyn StmtDriver> {
        self.driver.borrow().clone().expect("prepared statement")
    }

    /// php_pdo_stmt_set_column_count
    pub(crate) fn set_column_count(&self, n: i32) {
        if self.columns.borrow().is_none() {
            self.column_count.set(n);
            return;
        }
        if n == self.column_count.get() {
            return;
        }
        *self.columns.borrow_mut() = None;
        self.column_count.set(n);
    }

    fn column_name(&self, i: i32) -> Vec<u8> {
        self.columns
            .borrow()
            .as_ref()
            .and_then(|c| c.get(i as usize))
            .map(|c| c.name.clone())
            .unwrap_or_default()
    }
}

/// dbstmt_prop_write: `queryString` is written once.
fn stmt_prop_write(host: &mut dyn Host, h: &ObjectHandle, name: &[u8]) -> Result<(), Thrown> {
    if name == b"queryString"
        && host
            .object_prop(&Value::Object(h.clone()), b"queryString")
            .is_some()
    {
        return Err(Thrown::error("Error", "Property queryString is read only"));
    }
    Ok(())
}

/// dbstmt_prop_delete
fn stmt_prop_delete(_: &mut dyn Host, _: &ObjectHandle, name: &[u8]) -> Result<(), Thrown> {
    if name == b"queryString" {
        return Err(Thrown::error("Error", "Property queryString is read only"));
    }
    Ok(())
}

/// The state of `this`, a PDOStatement (or subclass) object.
pub(crate) fn stmt_of(host: &dyn Host, this: &Value) -> Option<Rc<Stmt>> {
    if !object_handlers::has_handlers(host, this, &STMT_HANDLERS) {
        return None;
    }
    let h = object_handlers::object_of(this)?;
    if let Some(s) = object_handlers::payload_of::<Stmt>(&h) {
        return Some(s);
    }
    let s = Rc::new(Stmt::new());
    object_handlers::set_payload(&h, s.clone());
    Some(s)
}

/// dbstmt_get_gc
fn stmt_get_gc(h: &ObjectHandle, out: &mut Vec<Value>) {
    let Some(s) = object_handlers::payload_of::<Stmt>(h) else {
        return;
    };
    if let Some(d) = s.database_object.borrow().clone() {
        out.push(d);
    }
    let f = s.fetch.borrow();
    if let Some(v) = &f.into {
        out.push(v.clone());
    }
    if let Some(a) = &f.ctor_args {
        out.push(Value::Array(a.clone()));
    }
    if let Some(v) = &f.func {
        out.push(v.clone());
    }
    for map in [&s.bound_params, &s.bound_columns] {
        if let Some(m) = map.borrow().as_ref() {
            for p in m.values() {
                out.push(p.parameter.clone());
                if let Some(d) = &p.driver_params {
                    out.push(d.clone());
                }
            }
        }
    }
}

/// pdo_stmt_describe_columns
pub(crate) fn describe_columns(cx: &mut Cx, stmt: &Stmt) -> bool {
    let n = stmt.column_count.get();
    let dbh = stmt.dbh();
    let drv = stmt.drv();
    let mut cols = Vec::with_capacity(n.max(0) as usize);
    for col in 0..n {
        let Some(mut name) = drv.describe(cx, stmt, col) else {
            *stmt.columns.borrow_mut() = Some(cols);
            return false;
        };
        if dbh.native_case.get() != dbh.desired_case.get() && dbh.desired_case.get() != CASE_NATURAL
        {
            match dbh.desired_case.get() {
                CASE_LOWER => name.make_ascii_lowercase(),
                CASE_UPPER => name.make_ascii_uppercase(),
                _ => {}
            }
        }
        if let Some(bc) = stmt.bound_columns.borrow_mut().as_mut() {
            if let Some(p) = bc.get_mut(&PKey::Name(name.clone())) {
                p.paramno = col as i64;
            }
        }
        cols.push(Column {
            name,
            maxlen: -1,
            precision: 0,
        });
    }
    *stmt.columns.borrow_mut() = Some(cols);
    true
}

/// dispatch_param_event
fn dispatch_param_event(cx: &mut Cx, stmt: &Stmt, event: u32) -> Result<bool, Thrown> {
    let dbh = stmt.dbh();
    if dbh.skip_param_evt.get() & (1 << event) != 0 {
        return Ok(true);
    }
    let drv = stmt.drv();
    stmt.in_param_event.set(true);
    let mut ret = Ok(true);
    'outer: for map in [&stmt.bound_params, &stmt.bound_columns] {
        let taken = map.borrow_mut().take();
        let Some(mut m) = taken else {
            continue;
        };
        for p in m.values_mut() {
            match drv.param_hook(cx, stmt, p, event) {
                Ok(true) => {}
                other => {
                    *map.borrow_mut() = Some(m);
                    ret = other;
                    break 'outer;
                }
            }
        }
        *map.borrow_mut() = Some(m);
    }
    stmt.in_param_event.set(false);
    ret
}

fn disallow_reentrant(stmt: &Stmt) -> Result<(), Thrown> {
    if stmt.in_param_event.get() {
        return Err(Thrown::error(
            "Error",
            "Cannot modify a PDOStatement while parameter hooks are running",
        ));
    }
    Ok(())
}

/// try_convert_to_string
fn to_string_value(cx: &mut Cx, v: &Value) -> Result<Value, Thrown> {
    Ok(match v {
        Value::Str(_) => v.clone(),
        other => Value::str(crate::zend::zpp::zval_get_string(cx.host, other)?),
    })
}

/// really_register_bound_param
fn register_bound_param(
    cx: &mut Cx,
    mut param: BoundParam,
    stmt: &Stmt,
    is_param: bool,
) -> Result<bool, Thrown> {
    let dbh = stmt.dbh();
    let v = param.value();
    let ty = param_type(param.param_type);
    if ty == PARAM_STR && param.max_value_len <= 0 && !matches!(v, Value::Null) {
        let s = to_string_value(cx, &v)?;
        param.set_value(s);
    } else if ty == PARAM_INT && matches!(v, Value::Bool(_)) {
        param.set_value(Value::Int(conv::to_int(&v)));
    } else if ty == PARAM_BOOL && matches!(v, Value::Int(_)) {
        param.set_value(Value::Bool(conv::to_bool(&v)));
    }
    param.is_param = is_param;

    if !is_param && param.name.is_some() && stmt.columns.borrow().is_some() {
        let name = param.name.clone().unwrap_or_default();
        let found = stmt
            .columns
            .borrow()
            .as_ref()
            .and_then(|c| c.iter().position(|c| c.name == name));
        if let Some(i) = found {
            param.paramno = i as i64;
        }
        if param.paramno == -1 {
            let msg = format!(
                "Did not find column name '{}' in the defined columns; it will not be bound",
                String::from_utf8_lossy(&name)
            );
            raise_impl_error(cx, &dbh, Some(stmt), "HY000", Some(&msg))?;
            return Ok(false);
        }
    }

    if let Some(n) = &param.name {
        if is_param && n.first() != Some(&b':') {
            let mut t = vec![b':'];
            t.extend_from_slice(n);
            param.name = Some(t);
        }
    }

    // rewrite_name_to_position: drivers with native named placeholders
    // keep no bound_param_map.

    let drv = stmt.drv();
    if !drv.param_hook(cx, stmt, &mut param, PARAM_EVT_NORMALIZE)? {
        if stmt.has_error() {
            handle_error(cx, &dbh, Some(stmt))?;
        }
        return Ok(false);
    }

    let map = if is_param {
        &stmt.bound_params
    } else {
        &stmt.bound_columns
    };
    let key = match &param.name {
        Some(n) => PKey::Name(n.clone()),
        None => PKey::Num(param.paramno),
    };
    {
        let mut m = map.borrow_mut();
        let m = m.get_or_insert_with(IndexMap::new);
        if param.paramno >= 0 {
            m.shift_remove(&PKey::Num(param.paramno));
        }
        m.insert(key.clone(), param);
    }
    let mut p = map
        .borrow_mut()
        .as_mut()
        .and_then(|m| m.get(&key).cloned())
        .expect("just inserted");
    let ok = drv.param_hook(cx, stmt, &mut p, PARAM_EVT_ALLOC)?;
    if !ok {
        if stmt.has_error() {
            handle_error(cx, &dbh, Some(stmt))?;
        }
        if let Some(m) = map.borrow_mut().as_mut() {
            m.shift_remove(&key);
        }
        return Ok(false);
    }
    if let Some(m) = map.borrow_mut().as_mut() {
        if let Some(slot) = m.get_mut(&key) {
            *slot = p;
        }
    }
    Ok(true)
}

/// pdo_stmt_invalidate_result
fn invalidate_result(cx: &mut Cx, stmt: &Stmt) {
    stmt.drv().close_cursor(stmt);
    let _ = cx;
    stmt.executed.set(false);
}

/// PDOStatement::execute
fn execute(cx: &mut Cx, stmt: &Stmt, input: Option<PhpArray>) -> Result<Value, Thrown> {
    disallow_reentrant(stmt)?;
    if stmt.executed.get() {
        invalidate_result(cx, stmt);
    }
    stmt.clear_err();
    let dbh = stmt.dbh();
    if let Some(input) = input {
        *stmt.bound_params.borrow_mut() = None;
        for (k, v) in input.iter() {
            let (name, paramno) = match k {
                ArrayKey::Str(s) => (Some(s.as_bytes().to_vec()), -1),
                ArrayKey::Int(n) => (None, n),
            };
            let p = BoundParam {
                paramno,
                name,
                max_value_len: 0,
                param_type: PARAM_STR,
                is_param: true,
                parameter: deref(v),
                driver_params: None,
            };
            if !register_bound_param(cx, p, stmt, true)? {
                return Ok(Value::Bool(false));
            }
        }
    }
    if stmt.supports_placeholders.get() != PLACEHOLDER_NONE
        && !dispatch_param_event(cx, stmt, PARAM_EVT_EXEC_PRE)?
    {
        handle_stmt_err(cx, &dbh, stmt)?;
        return Ok(Value::Bool(false));
    }
    let drv = stmt.drv();
    if drv.execute(cx, stmt) {
        let mut ret = true;
        if !stmt.executed.get() {
            if dbh.alloc_own_columns.get() && stmt.columns.borrow().is_none() {
                ret = describe_columns(cx, stmt);
            }
            stmt.executed.set(true);
        }
        if ret && !dispatch_param_event(cx, stmt, PARAM_EVT_EXEC_POST)? {
            handle_stmt_err(cx, &dbh, stmt)?;
            return Ok(Value::Bool(false));
        }
        return Ok(Value::Bool(ret));
    }
    handle_stmt_err(cx, &dbh, stmt)?;
    Ok(Value::Bool(false))
}

/// PDO_HANDLE_STMT_ERR
pub(crate) fn handle_stmt_err(cx: &mut Cx, dbh: &Dbh, stmt: &Stmt) -> Result<(), Thrown> {
    if stmt.has_error() {
        return handle_error(cx, dbh, Some(stmt));
    }
    Ok(())
}

/// fetch_value
fn fetch_value(
    cx: &mut Cx,
    stmt: &Stmt,
    colno: i64,
    type_override: Option<i64>,
) -> Result<Value, Thrown> {
    if colno < 0 {
        return Err(Thrown::error(
            "ValueError",
            "Column index must be greater than or equal to 0",
        ));
    }
    if colno >= stmt.column_count.get() as i64 {
        return Err(Thrown::error("ValueError", "Invalid column index"));
    }
    let dbh = stmt.dbh();
    let mut v = stmt.drv().get_col(cx, stmt, colno as i32);
    if dbh.oracle_nulls.get() == NULL_EMPTY_STRING && matches!(&v, Value::Str(s) if s.is_empty()) {
        v = Value::Null;
    }
    let type_override = if dbh.stringify.get() {
        Some(PARAM_STR)
    } else {
        type_override
    };
    if let Some(t) = type_override {
        if !matches!(v, Value::Null) {
            v = match t {
                PARAM_INT => Value::Int(conv::to_int(&v)),
                PARAM_BOOL => Value::Bool(conv::to_bool(&v)),
                PARAM_STR => match &v {
                    Value::Bool(false) => Value::str("0"),
                    Value::Str(_) => v,
                    Value::Resource(_) => {
                        let res = v.clone();
                        let mut all = Vec::new();
                        while let Some(chunk) = cx.host.stream_read(&res, 8192) {
                            if chunk.is_empty() {
                                break;
                            }
                            all.extend_from_slice(&chunk);
                        }
                        Value::str(all)
                    }
                    other => Value::str(conv::to_string_bytes(other)),
                },
                PARAM_NULL => Value::Null,
                PARAM_LOB => match &v {
                    Value::Str(s) => memory_stream(cx, s.as_bytes()),
                    _ => v,
                },
                _ => v,
            };
        }
    }
    if matches!(v, Value::Null) && dbh.oracle_nulls.get() == NULL_TO_STRING {
        v = Value::str("");
    }
    Ok(v)
}

/// php_stream_memory_open(TEMP_STREAM_READONLY, s)
fn memory_stream(cx: &mut Cx, s: &[u8]) -> Value {
    match cx.host.stream_open(b"php://memory", b"w+b") {
        Some(res) => {
            cx.host.stream_write(&res, s);
            cx.host.stream_rewind(&res);
            res
        }
        None => Value::str(s),
    }
}

/// do_fetch_common
fn do_fetch_common(cx: &mut Cx, stmt: &Stmt) -> Result<bool, Thrown> {
    if !stmt.executed.get() {
        return Ok(false);
    }
    if !dispatch_param_event(cx, stmt, PARAM_EVT_FETCH_PRE)? {
        return Ok(false);
    }
    if !stmt.drv().fetch(cx, stmt) {
        return Ok(false);
    }
    if stmt.columns.borrow().is_none() && !describe_columns(cx, stmt) {
        return Ok(false);
    }
    if !dispatch_param_event(cx, stmt, PARAM_EVT_FETCH_POST)? {
        return Ok(false);
    }
    let cols: Vec<(i64, i64, rhp_values::PhpRef)> = match stmt.bound_columns.borrow().as_ref() {
        Some(m) => m
            .values()
            .filter(|p| p.paramno >= 0)
            .filter_map(|p| match &p.parameter {
                Value::Ref(r) => Some((p.paramno, p.param_type, r.clone())),
                _ => None,
            })
            .collect(),
        None => Vec::new(),
    };
    for (no, ty, r) in cols {
        let v = fetch_value(cx, stmt, no, Some(ty))?;
        let old = std::mem::replace(&mut *r.borrow_mut(), Value::Null);
        drop(old);
        cx.host.assign_ref(&r, v)?;
    }
    Ok(true)
}

/// pdo_do_key_pair_fetch
fn key_pair_fetch(cx: &mut Cx, stmt: &Stmt, out: &mut PhpArray) -> Result<bool, Thrown> {
    if !do_fetch_common(cx, stmt)? {
        return Ok(false);
    }
    if stmt.column_count.get() != 2 {
        let dbh = stmt.dbh();
        raise_impl_error(
            cx,
            &dbh,
            Some(stmt),
            "HY000",
            Some("PDO::FETCH_KEY_PAIR fetch mode requires the result set to contain exactly 2 columns."),
        )?;
        return Ok(false);
    }
    let key = fetch_value(cx, stmt, 0, None)?;
    let val = fetch_value(cx, stmt, 1, None)?;
    match key {
        Value::Int(n) => {
            out.insert(ArrayKey::Int(n), val);
        }
        other => {
            let k = crate::zend::zpp::zval_get_string(cx.host, &other)?;
            out.insert(ArrayKey::from_bytes(&k), val);
        }
    }
    Ok(true)
}

/// The class and constructor arguments of FETCH_CLASS.
fn call_ctor(cx: &mut Cx, obj: &Value, cls: &[u8], args: Option<&PhpArray>) -> Result<(), Thrown> {
    let args = Value::Array(args.cloned().unwrap_or_default());
    cx.host.reflect_invoke(obj, cls, b"__construct", args)?;
    Ok(())
}

fn has_ctor(host: &mut dyn Host, cls: &[u8]) -> bool {
    host.resolve_class(cls)
        .is_some_and(|c| host.class_has_method(c, b"__construct"))
}

/// The canonical name of class `name` (autoloading it), if it exists.
pub(crate) fn lookup_class(cx: &mut Cx, name: &[u8]) -> Result<Option<Vec<u8>>, Thrown> {
    Ok(match cx.host.resolve_class_autoload(name)? {
        Some(c) => cx.host.class_name(c),
        None => None,
    })
}

/// do_fetch: the row in mode `how`, or `None` (php's false).
fn do_fetch(
    cx: &mut Cx,
    stmt: &Stmt,
    how: i64,
    group_key: Option<&mut Value>,
) -> Result<Option<Value>, Thrown> {
    let how = if how == FETCH_USE_DEFAULT {
        stmt.default_fetch_type.get()
    } else {
        how
    };
    let flags = how & FETCH_FLAGS;
    let how = how & !FETCH_FLAGS;
    if !do_fetch_common(cx, stmt)? {
        return Ok(None);
    }
    if how == FETCH_BOUND {
        return Ok(Some(Value::Bool(true)));
    }
    if how == FETCH_LAZY {
        return Ok(Some(lazy_object(cx, stmt)?));
    }
    let dbh = stmt.dbh();
    let ncols = stmt.column_count.get() as i64;
    if how == FETCH_COLUMN {
        let fcol = stmt.fetch.borrow().column;
        let mut colno = fcol;
        if flags & (FETCH_GROUP | FETCH_UNIQUE) != 0 && fcol == -1 {
            colno = 1;
        }
        if colno < 0 {
            return Err(Thrown::error(
                "ValueError",
                "Column index must be greater than or equal to 0",
            ));
        }
        if colno >= ncols {
            return Err(Thrown::error("ValueError", "Invalid column index"));
        }
        let v = if flags == FETCH_GROUP && fcol == -1 {
            fetch_value(cx, stmt, 1, None)?
        } else if flags == FETCH_GROUP && colno != 0 {
            fetch_value(cx, stmt, 0, None)?
        } else {
            fetch_value(cx, stmt, colno, None)?
        };
        if let Some(gk) = group_key {
            let k = if flags == FETCH_GROUP && fcol > 0 {
                fetch_value(cx, stmt, colno, None)?
            } else {
                fetch_value(cx, stmt, 0, None)?
            };
            *gk = Value::str(crate::zend::zpp::zval_get_string(cx.host, &k)?);
        }
        return Ok(Some(v));
    }

    stmt.in_fetch.set(true);
    let r = do_fetch_row(cx, stmt, &dbh, how, flags, group_key);
    stmt.in_fetch.set(false);
    r
}

enum Target {
    Array(PhpArray),
    Obj(Vec<(Vec<u8>, Value)>),
    Object { obj: Value, scope: Vec<u8> },
    Func(Vec<Value>),
}

fn do_fetch_row(
    cx: &mut Cx,
    stmt: &Stmt,
    dbh: &Dbh,
    how: i64,
    flags: i64,
    group_key: Option<&mut Value>,
) -> Result<Option<Value>, Thrown> {
    let ncols = stmt.column_count.get() as i64;
    let mut idx = 0i64;
    let mut ce: Vec<u8> = Vec::new();
    let mut ctor_args: Option<PhpArray> = None;
    let mut target = match how {
        FETCH_ASSOC | FETCH_BOTH | FETCH_NUM | FETCH_NAMED | FETCH_USE_DEFAULT => {
            Target::Array(PhpArray::new())
        }
        FETCH_OBJ => Target::Obj(Vec::new()),
        FETCH_CLASS => {
            let fetch = stmt.fetch.borrow().clone();
            let mut cls = fetch.cls.clone();
            if flags & FETCH_CLASSTYPE != 0 {
                let name = fetch_value(cx, stmt, idx, None)?;
                idx += 1;
                cls = match &name {
                    Value::Str(s) => lookup_class(cx, s.as_bytes())?,
                    _ => None,
                };
                if cls.is_none() {
                    cls = Some(b"stdClass".to_vec());
                }
            } else if cls.is_none() {
                raise_impl_error(
                    cx,
                    dbh,
                    Some(stmt),
                    "HY000",
                    Some("No fetch class specified"),
                )?;
                return Ok(None);
            }
            ce = cls.unwrap_or_default();
            ctor_args = fetch.ctor_args.clone();
            if flags & FETCH_SERIALIZE != 0 {
                raise_impl_error(
                    cx,
                    dbh,
                    Some(stmt),
                    "HY000",
                    Some("cannot unserialize class"),
                )?;
                return Ok(None);
            }
            let obj = cx.host.reflect_new_instance(&ce, Vec::new(), false)?;
            if flags & FETCH_PROPS_LATE != 0 && has_ctor(cx.host, &ce) {
                call_ctor(cx, &obj, &ce, ctor_args.as_ref())?;
            }
            Target::Object {
                obj,
                scope: ce.clone(),
            }
        }
        FETCH_INTO => {
            let into = stmt.fetch.borrow().into.clone();
            let Some(obj) = into else {
                raise_impl_error(
                    cx,
                    dbh,
                    Some(stmt),
                    "HY000",
                    Some("No fetch-into object specified."),
                )?;
                return Ok(None);
            };
            Target::Object {
                obj,
                scope: Vec::new(),
            }
        }
        FETCH_FUNC => {
            if stmt.fetch.borrow().func.is_none() {
                raise_impl_error(
                    cx,
                    dbh,
                    Some(stmt),
                    "HY000",
                    Some("No fetch function specified"),
                )?;
                return Ok(None);
            }
            Target::Func(Vec::new())
        }
        _ => Target::Array(PhpArray::new()),
    };

    if let Some(gk) = group_key {
        let k = fetch_value(cx, stmt, idx, None)?;
        *gk = Value::str(crate::zend::zpp::zval_get_string(cx.host, &k)?);
        idx += 1;
    }

    while idx < ncols {
        let val = fetch_value(cx, stmt, idx, None)?;
        let name = stmt.column_name(idx as i32);
        match &mut target {
            Target::Array(a) => match how {
                FETCH_ASSOC => {
                    a.insert(ArrayKey::from_bytes(&name), val);
                }
                FETCH_BOTH | FETCH_USE_DEFAULT => {
                    a.insert(ArrayKey::from_bytes(&name), val.clone());
                    let k = ArrayKey::Int(idx);
                    if !a.contains_key(&k) {
                        a.insert(k, val);
                    }
                }
                FETCH_NAMED => {
                    let k = ArrayKey::Str(PhpStr::from_slice(&name));
                    match a.get_mut(&k) {
                        Some(cur) => {
                            if let Value::Array(arr) = cur {
                                arr.push(val);
                            } else {
                                let mut arr = PhpArray::new();
                                arr.push(std::mem::replace(cur, Value::Null));
                                arr.push(val);
                                *cur = Value::Array(arr);
                            }
                        }
                        None => {
                            a.insert(k, val);
                        }
                    }
                }
                _ => {
                    a.push(val);
                }
            },
            Target::Obj(props) => {
                if let Some(slot) = props.iter_mut().find(|(k, _)| *k == name) {
                    slot.1 = val;
                } else {
                    props.push((name, val));
                }
            }
            Target::Object { obj, scope } => {
                cx.host.reflect_set_prop(obj, scope, &name, val)?;
            }
            Target::Func(params) => params.push(val),
        }
        idx += 1;
    }

    Ok(Some(match target {
        Target::Array(a) => Value::Array(a),
        Target::Obj(props) => cx.host.make_object(props),
        Target::Object { obj, .. } => {
            if how == FETCH_CLASS
                && flags & (FETCH_PROPS_LATE | FETCH_SERIALIZE) == 0
                && has_ctor(cx.host, &ce)
            {
                call_ctor(cx, &obj, &ce, ctor_args.as_ref())?;
            }
            obj
        }
        Target::Func(params) => {
            let f = stmt.fetch.borrow().func.clone().unwrap_or(Value::Null);
            cx.host.call_callable(&f, &params)?
        }
    }))
}

/// pdo_verify_fetch_mode
fn verify_fetch_mode(
    cx: &mut Cx,
    default: i64,
    mode: i64,
    arg: usize,
    fetch_all: bool,
) -> Result<(), Thrown> {
    if !(0..FIRST_INVALID_FLAG).contains(&mode) {
        return Err(cx.value_error(arg, "mode", "must be a bitmask of PDO::FETCH_* constants"));
    }
    let mut flags = mode & FETCH_FLAGS;
    let mut m = mode & !FETCH_FLAGS;
    if m == FETCH_USE_DEFAULT {
        flags = default & FETCH_FLAGS;
        m = default & !FETCH_FLAGS;
    }
    if flags != 0 {
        let class_flags = flags & (FETCH_CLASSTYPE | FETCH_PROPS_LATE | FETCH_SERIALIZE) != 0;
        if class_flags && m != FETCH_CLASS {
            return Err(cx.value_error(arg, "mode", "cannot use PDO::FETCH_CLASSTYPE, PDO::FETCH_PROPS_LATE, or PDO::FETCH_SERIALIZE fetch flags with a fetch mode other than PDO::FETCH_CLASS"));
        }
        if flags & FETCH_SERIALIZE != 0 {
            cx.docref(
                crate::E_DEPRECATED,
                "The PDO::FETCH_SERIALIZE mode is deprecated",
            )?;
        }
    }
    match m {
        FETCH_FUNC if !fetch_all => Err(cx.value_error(
            arg,
            "mode",
            "PDO::FETCH_FUNC can only be used with PDOStatement::fetchAll()",
        )),
        FETCH_LAZY if fetch_all => Err(cx.value_error(
            arg,
            "mode",
            "PDO::FETCH_LAZY cannot be used with PDOStatement::fetchAll()",
        )),
        FETCH_INTO if fetch_all => Err(cx.value_error(
            arg,
            "mode",
            "PDO::FETCH_INTO cannot be used with PDOStatement::fetchAll()",
        )),
        FETCH_FUNC | FETCH_LAZY | FETCH_INTO | FETCH_ASSOC | FETCH_NUM | FETCH_BOTH | FETCH_OBJ
        | FETCH_BOUND | FETCH_COLUMN | FETCH_CLASS | FETCH_NAMED | FETCH_KEY_PAIR => Ok(()),
        _ => Err(cx.value_error(arg, "mode", "must be a bitmask of PDO::FETCH_* constants")),
    }
}

/// The parameter names of the methods whose argument is the fetch mode.
fn mode_pname(fname: &str) -> &'static str {
    if fname.ends_with("query") {
        "fetchMode"
    } else {
        "mode"
    }
}

/// pdo_stmt_setup_fetch_mode
pub(crate) fn setup_fetch_mode(
    cx: &mut Cx,
    stmt: &Stmt,
    mode: i64,
    mode_arg: usize,
    args: &[Value],
) -> Result<(), Thrown> {
    let flags = mode & FETCH_FLAGS;
    let fetch_type = mode & !FETCH_FLAGS;
    let arg1 = mode_arg + 1;
    let ctor_arg = mode_arg + 2;
    let total = mode_arg + args.len();
    let dbh = stmt.dbh();
    let pname = mode_pname(&cx.fname);
    verify_fetch_mode_named(
        cx,
        dbh.default_fetch_type.get(),
        mode,
        mode_arg,
        false,
        pname,
    )?;
    let count_error = |cx: &Cx, kind: &str, n: usize| {
        Thrown::error(
            "ArgumentCountError",
            format!(
                "{}() expects {kind} {n} arguments for the fetch mode provided, {total} given",
                cx.fname
            ),
        )
    };
    let arg_names = ["", "", "", ""];
    let _ = arg_names;
    let mut new = Fetch::default();
    match fetch_type {
        FETCH_USE_DEFAULT | FETCH_LAZY | FETCH_ASSOC | FETCH_NUM | FETCH_BOTH | FETCH_OBJ
        | FETCH_BOUND | FETCH_NAMED | FETCH_KEY_PAIR => {
            if !args.is_empty() {
                return Err(count_error(cx, "exactly", mode_arg));
            }
        }
        FETCH_COLUMN => {
            if args.len() != 1 {
                return Err(count_error(cx, "exactly", arg1));
            }
            let a = deref(&args[0]);
            let Value::Int(n) = a else {
                let msg = format!("must be of type int, {} given", cx.value_name(&a));
                return Err(variadic_type_error(cx, arg1, &msg));
            };
            if n < 0 {
                return Err(variadic_value_error(
                    cx,
                    arg1,
                    "must be greater than or equal to 0",
                ));
            }
            new.column = n;
        }
        FETCH_CLASS => {
            if flags & FETCH_CLASSTYPE == FETCH_CLASSTYPE {
                if !args.is_empty() {
                    return Err(count_error(cx, "exactly", mode_arg));
                }
            } else {
                if args.is_empty() {
                    return Err(count_error(cx, "at least", arg1));
                }
                if args.len() > 2 {
                    return Err(count_error(cx, "at most", ctor_arg));
                }
                let a = deref(&args[0]);
                let Value::Str(name) = &a else {
                    let msg = format!("must be of type string, {} given", cx.value_name(&a));
                    return Err(variadic_type_error(cx, arg1, &msg));
                };
                let Some(cls) = lookup_class(cx, name.as_bytes())? else {
                    return Err(variadic_type_error(cx, arg1, "must be a valid class"));
                };
                if args.len() == 2 {
                    let c = deref(&args[1]);
                    match &c {
                        Value::Null => {}
                        Value::Array(arr) => {
                            if !arr.is_empty() {
                                if !has_ctor(cx.host, &cls) {
                                    return Err(cx.value_error(3, "constructorArgs", "must be empty when class provided in argument #2 ($class) does not have a constructor"));
                                }
                                new.ctor_args = Some(arr.clone());
                            }
                        }
                        _ => {
                            let msg =
                                format!("must be of type ?array, {} given", cx.value_name(&c));
                            return Err(variadic_type_error(cx, ctor_arg, &msg));
                        }
                    }
                }
                new.cls = Some(cls);
            }
        }
        FETCH_INTO => {
            if total != arg1 {
                return Err(count_error(cx, "exactly", arg1));
            }
            let a = deref(&args[0]);
            if !matches!(a, Value::Object(_)) {
                let msg = format!("must be of type object, {} given", cx.value_name(&a));
                return Err(variadic_type_error(cx, arg1, &msg));
            }
            new.into = Some(a);
        }
        _ => {
            return Err(cx.value_error(
                mode_arg,
                pname,
                "must be one of the PDO::FETCH_* constants",
            ))
        }
    }
    *stmt.fetch.borrow_mut() = new;
    if fetch_type == FETCH_USE_DEFAULT {
        stmt.default_fetch_type.set(dbh.default_fetch_type.get());
    } else {
        stmt.default_fetch_type.set(mode);
    }
    Ok(())
}

fn verify_fetch_mode_named(
    cx: &mut Cx,
    default: i64,
    mode: i64,
    arg: usize,
    fetch_all: bool,
    pname: &str,
) -> Result<(), Thrown> {
    match verify_fetch_mode(cx, default, mode, arg, fetch_all) {
        Err(Thrown::Error { class, message, .. }) if pname != "mode" => Err(Thrown::error(
            class,
            message.replacen("($mode)", &format!("(${pname})"), 1),
        )),
        r => r,
    }
}

/// zend_argument_type_error for a variadic argument (php names it by the
/// variadic parameter, `...$args` or `...$fetchModeArgs`).
fn variadic_type_error(cx: &Cx, n: usize, msg: &str) -> Thrown {
    Thrown::type_error(format!("{}(): Argument #{n} {msg}", cx.fname))
}

fn variadic_value_error(cx: &Cx, n: usize, msg: &str) -> Thrown {
    Thrown::error("ValueError", format!("{}(): Argument #{n} {msg}", cx.fname))
}

/// The statement of `this`; PHP_STMT_GET_OBJ.
fn get_obj(host: &dyn Host, this: &Value) -> Result<Rc<Stmt>, Thrown> {
    let s = stmt_of(host, this).ok_or_else(|| Thrown::type_error("not a PDOStatement"))?;
    if s.dbh.borrow().is_none() {
        return Err(Thrown::error(
            "Error",
            format!("{} object is uninitialized", class_of(host, this)),
        ));
    }
    Ok(s)
}

/// `__pdo_stmt($method, $this, ...$args)`: PDOStatement's methods.
pub(super) fn fn_pdo_stmt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let method = String::from_utf8_lossy(&conv::to_string_bytes(&deref(&args[0]))).into_owned();
    let this = deref(&args[1]);
    let a = &args[2..];
    let mut cx = Cx::new(host, format!("PDOStatement::{method}"));
    let r = match stmt_of(&*cx.host, &this) {
        Some(s) => with_current(&s, &this, || stmt_method(&mut cx, &method, &this, a)),
        None => stmt_method(&mut cx, &method, &this, a),
    };
    cx.finish(r)
}

fn arg(a: &[Value], i: usize) -> Value {
    a.get(i).map(deref).unwrap_or(Value::Null)
}

fn stmt_method(cx: &mut Cx, method: &str, this: &Value, a: &[Value]) -> Result<Value, Thrown> {
    let stmt = get_obj(&*cx.host, this)?;
    let dbh = stmt.dbh();
    let b = Value::Bool;
    match method {
        "execute" => {
            let input = match arg(a, 0) {
                Value::Array(arr) => Some(arr),
                _ => None,
            };
            execute(cx, &stmt, input)
        }
        "fetch" => {
            let how = conv::to_int(&arg(a, 0));
            stmt.clear_err();
            verify_fetch_mode(cx, stmt.default_fetch_type.get(), how, 1, false)?;
            if how & !FETCH_FLAGS == FETCH_KEY_PAIR {
                let mut out = PhpArray::new();
                if !key_pair_fetch(cx, &stmt, &mut out)? {
                    handle_stmt_err(cx, &dbh, &stmt)?;
                    return Ok(b(false));
                }
                return Ok(Value::Array(out));
            }
            match do_fetch(cx, &stmt, how, None)? {
                Some(v) => Ok(v),
                None => {
                    handle_stmt_err(cx, &dbh, &stmt)?;
                    Ok(b(false))
                }
            }
        }
        "fetchObject" => {
            stmt.clear_err();
            let cls = match arg(a, 0) {
                Value::Null => b"stdClass".to_vec(),
                v => {
                    let name = conv::to_string_bytes(&v);
                    match lookup_class(cx, &name)? {
                        Some(c) => c,
                        None => {
                            return Err(cx.type_error_arg(
                                1,
                                "class",
                                &format!(
                                    "must be a valid class name, {} given",
                                    String::from_utf8_lossy(&name)
                                ),
                            ))
                        }
                    }
                }
            };
            let ctor = match arg(a, 1) {
                Value::Array(x) => Some(x),
                _ => None,
            };
            if ctor.as_ref().is_some_and(|c| !c.is_empty()) && !has_ctor(cx.host, &cls) {
                return Err(cx.value_error(2, "constructorArgs", "must be empty when class provided in argument #1 ($class) does not have a constructor"));
            }
            let old = stmt.fetch.borrow().clone();
            {
                let mut f = stmt.fetch.borrow_mut();
                f.cls = Some(cls);
                f.ctor_args = ctor;
            }
            let r = do_fetch(cx, &stmt, FETCH_CLASS, None);
            {
                let mut f = stmt.fetch.borrow_mut();
                f.cls = old.cls;
                f.ctor_args = old.ctor_args;
            }
            match r? {
                Some(v) => Ok(v),
                None => {
                    handle_stmt_err(cx, &dbh, &stmt)?;
                    Ok(b(false))
                }
            }
        }
        "fetchColumn" => {
            let col = conv::to_int(&arg(a, 0));
            stmt.clear_err();
            if !do_fetch_common(cx, &stmt)? {
                handle_stmt_err(cx, &dbh, &stmt)?;
                return Ok(b(false));
            }
            fetch_value(cx, &stmt, col, None)
        }
        "fetchAll" => fetch_all(cx, &stmt, &dbh, a),
        "bindValue" => {
            disallow_reentrant(&stmt)?;
            let p = new_param(cx, &arg(a, 0), arg(a, 1), conv::to_int(&arg(a, 2)), "param")?;
            Ok(b(register_bound_param(cx, p, &stmt, true)?))
        }
        "bindParam" | "bindColumn" => {
            disallow_reentrant(&stmt)?;
            let var = match arg(a, 1) {
                Value::Array(arr) => arr.get(&ArrayKey::Int(0)).cloned().unwrap_or(Value::Null),
                v => v,
            };
            let pname = if method == "bindParam" {
                "param"
            } else {
                "column"
            };
            let mut p = new_param(cx, &arg(a, 0), var, conv::to_int(&arg(a, 2)), pname)?;
            p.max_value_len = conv::to_int(&arg(a, 3));
            p.driver_params = match arg(a, 4) {
                Value::Null => None,
                v => Some(v),
            };
            Ok(b(register_bound_param(
                cx,
                p,
                &stmt,
                method == "bindParam",
            )?))
        }
        "rowCount" => Ok(Value::Int(stmt.row_count.get())),
        "errorCode" => {
            let c = stmt.error_code.borrow().clone();
            Ok(if c.is_empty() {
                Value::Null
            } else {
                Value::str(c)
            })
        }
        "errorInfo" => {
            let mut out = PhpArray::new();
            let code = stmt.error_code.borrow().clone();
            out.push(Value::str(code.as_str()));
            if code != ERR_NONE {
                if let Some(d) = dbh.driver() {
                    d.fetch_err(&dbh, Some(&stmt), &mut out);
                }
            }
            while out.len() < 3 {
                out.push(Value::Null);
            }
            Ok(Value::Array(out))
        }
        "setAttribute" => {
            let attr = conv::to_int(&arg(a, 0));
            let v = arg(a, 1);
            stmt.clear_err();
            if stmt.drv().set_attribute(cx, &stmt, attr, &v)? {
                return Ok(b(true));
            }
            handle_stmt_err(cx, &dbh, &stmt)?;
            Ok(b(false))
        }
        "getAttribute" => {
            let attr = conv::to_int(&arg(a, 0));
            stmt.clear_err();
            match stmt.drv().get_attribute(&stmt, attr)? {
                (-1, _) => {
                    handle_stmt_err(cx, &dbh, &stmt)?;
                    Ok(b(false))
                }
                (0, _) => {
                    if attr == ATTR_EMULATE_PREPARES {
                        return Ok(b(stmt.supports_placeholders.get() == PLACEHOLDER_NONE));
                    }
                    raise_impl_error(
                        cx,
                        &dbh,
                        Some(&stmt),
                        "IM001",
                        Some("driver doesn't support getting that attribute"),
                    )?;
                    Ok(b(false))
                }
                (_, v) => Ok(v),
            }
        }
        "columnCount" => Ok(Value::Int(stmt.column_count.get() as i64)),
        "getColumnMeta" => {
            let col = conv::to_int(&arg(a, 0));
            if col < 0 {
                return Err(cx.value_error(1, "column", "must be greater than or equal to 0"));
            }
            stmt.clear_err();
            let Some(mut meta) = stmt.drv().column_meta(cx, &stmt, col) else {
                handle_stmt_err(cx, &dbh, &stmt)?;
                return Ok(b(false));
            };
            let c = stmt
                .columns
                .borrow()
                .as_ref()
                .and_then(|c| c.get(col as usize).cloned());
            let Some(c) = c.filter(|_| col < stmt.column_count.get() as i64) else {
                raise_impl_error(cx, &dbh, Some(&stmt), "07009", Some("invalid column index"))?;
                return Ok(b(false));
            };
            meta.insert(ArrayKey::from_bytes(b"name"), Value::str(c.name));
            meta.insert(ArrayKey::from_bytes(b"len"), Value::Int(c.maxlen));
            meta.insert(ArrayKey::from_bytes(b"precision"), Value::Int(c.precision));
            Ok(Value::Array(meta))
        }
        "setFetchMode" => {
            let n = conv::to_int(&arg(a, 0)) as usize;
            let mode = conv::to_int(&arg(a, 1));
            let rest: Vec<Value> = a
                .iter()
                .skip(2)
                .take(n.saturating_sub(1))
                .cloned()
                .collect();
            if stmt.in_fetch.get() {
                return Err(Thrown::error(
                    "Error",
                    "Cannot change default fetch mode while fetching",
                ));
            }
            setup_fetch_mode(cx, &stmt, mode, 1, &rest)?;
            Ok(b(true))
        }
        "nextRowset" => {
            raise_impl_error(
                cx,
                &dbh,
                Some(&stmt),
                "IM001",
                Some("driver does not support multiple rowsets"),
            )?;
            Ok(b(false))
        }
        "closeCursor" => {
            disallow_reentrant(&stmt)?;
            stmt.clear_err();
            if !stmt.drv().close_cursor(&stmt) {
                handle_stmt_err(cx, &dbh, &stmt)?;
                return Ok(b(false));
            }
            stmt.executed.set(false);
            Ok(b(true))
        }
        "debugDumpParams" => debug_dump_params(cx, &stmt),
        "getIterator" => {
            let h = object_handlers::object_of(this).expect("object");
            Ok(Value::Object(stmt_iter_get(cx.host, &h, false)?))
        }
        _ => Err(Thrown::error(
            "Error",
            format!("Call to undefined method PDOStatement::{method}()"),
        )),
    }
}

/// The parameter of bindValue/bindParam/bindColumn (Z_PARAM_STR_OR_LONG).
fn new_param(
    cx: &mut Cx,
    key: &Value,
    v: Value,
    ty: i64,
    pname: &str,
) -> Result<BoundParam, Thrown> {
    let (name, paramno) = match key {
        Value::Int(n) => {
            if *n > 0 {
                (None, n - 1)
            } else {
                return Err(cx.value_error(1, pname, "must be greater than or equal to 1"));
            }
        }
        other => {
            let s = conv::to_string_bytes(other);
            if s.is_empty() {
                return Err(cx.value_error(1, pname, "must not be empty"));
            }
            (Some(s), -1)
        }
    };
    Ok(BoundParam {
        paramno,
        name,
        max_value_len: 0,
        param_type: ty,
        is_param: true,
        parameter: v,
        driver_params: None,
    })
}

/// PDOStatement::fetchAll
fn fetch_all(cx: &mut Cx, stmt: &Stmt, dbh: &Dbh, a: &[Value]) -> Result<Value, Thrown> {
    let nargs = conv::to_int(&arg(a, 0)) as usize;
    if nargs > 3 {
        return Err(Thrown::error(
            "ArgumentCountError",
            format!("{}() expects at most 3 arguments, {nargs} given", cx.fname),
        ));
    }
    let mut how = conv::to_int(&arg(a, 1));
    let arg2 = if nargs >= 2 {
        match arg(a, 2) {
            Value::Null => None,
            v => Some(v),
        }
    } else {
        None
    };
    let ctor_args = if nargs >= 3 {
        match arg(a, 3) {
            Value::Null => None,
            Value::Array(x) => Some(x),
            v => {
                let msg = format!("must be of type ?array, {} given", cx.value_name(&v));
                return Err(variadic_type_error(cx, 3, &msg));
            }
        }
    } else {
        None
    };
    verify_fetch_mode(cx, stmt.default_fetch_type.get(), how, 1, true)?;
    let mut fetch_mode = how & !FETCH_FLAGS;
    let mut flags = how & FETCH_FLAGS;
    let old = stmt.fetch.borrow().clone();
    let restore = |stmt: &Stmt| {
        let mut f = stmt.fetch.borrow_mut();
        f.cls = old.cls.clone();
        f.ctor_args = old.ctor_args.clone();
    };
    match fetch_mode {
        FETCH_CLASS => {
            let cls = match &arg2 {
                Some(Value::Str(s)) => match lookup_class(cx, s.as_bytes())? {
                    Some(c) => c,
                    None => return Err(variadic_type_error(cx, 2, "must be a valid class")),
                },
                Some(v) => {
                    let msg = format!("must be of type string, {} given", cx.value_name(v));
                    return Err(variadic_type_error(cx, 2, &msg));
                }
                None => b"stdClass".to_vec(),
            };
            if let Some(c) = ctor_args.filter(|c| !c.is_empty()) {
                if !has_ctor(cx.host, &cls) {
                    return Err(variadic_value_error(cx, 3, "must be empty when class provided in argument #2 ($class) does not have a constructor"));
                }
                stmt.fetch.borrow_mut().ctor_args = Some(c);
            }
            stmt.fetch.borrow_mut().cls = Some(cls);
        }
        FETCH_FUNC => {
            if nargs != 2 {
                return Err(Thrown::error(
                    "ArgumentCountError",
                    format!(
                        "{}() expects exactly 2 argument for PDO::FETCH_FUNC, {nargs} given",
                        cx.fname
                    ),
                ));
            }
            let Some(f) = arg2 else {
                return Err(variadic_type_error(cx, 2, "must be a callable, null given"));
            };
            if let Some(why) = cx.host.check_callable(&f)? {
                return Err(Thrown::type_error(why));
            }
            stmt.fetch.borrow_mut().func = Some(f);
        }
        FETCH_COLUMN => {
            if nargs > 2 {
                return Err(Thrown::error(
                    "ArgumentCountError",
                    format!(
                        "{}() expects at most 2 argument for the fetch mode provided, {nargs} given",
                        cx.fname
                    ),
                ));
            }
            match &arg2 {
                Some(Value::Int(n)) => {
                    if *n < 0 {
                        return Err(variadic_value_error(
                            cx,
                            2,
                            "must be greater than or equal to 0",
                        ));
                    }
                    stmt.fetch.borrow_mut().column = *n;
                }
                Some(v) => {
                    let msg = format!("must be of type int, {} given", cx.value_name(v));
                    return Err(variadic_type_error(cx, 2, &msg));
                }
                None => {
                    stmt.fetch.borrow_mut().column = if flags & (FETCH_GROUP | FETCH_UNIQUE) != 0 {
                        -1
                    } else {
                        0
                    };
                }
            }
        }
        _ => {
            if nargs > 1 {
                return Err(Thrown::error(
                    "ArgumentCountError",
                    format!(
                        "{}() expects exactly 1 argument for the fetch mode provided, {nargs} given",
                        cx.fname
                    ),
                ));
            }
        }
    }
    if fetch_mode == FETCH_USE_DEFAULT {
        flags |= stmt.default_fetch_type.get() & FETCH_FLAGS;
        fetch_mode = stmt.default_fetch_type.get() & !FETCH_FLAGS;
        how = fetch_mode | flags;
    }
    stmt.clear_err();
    let mut out = PhpArray::new();
    if fetch_mode == FETCH_KEY_PAIR {
        while key_pair_fetch(cx, stmt, &mut out)? {}
        handle_stmt_err(cx, dbh, stmt)?;
        return Ok(Value::Array(out));
    }
    let r = (|| -> Result<(), Thrown> {
        if flags & FETCH_GROUP != 0 || flags & FETCH_UNIQUE != 0 {
            loop {
                let mut gk = Value::Null;
                let Some(data) = do_fetch(cx, stmt, how | flags, Some(&mut gk))? else {
                    break;
                };
                let key = ArrayKey::from_bytes(&conv::to_string_bytes(&gk));
                if flags & FETCH_UNIQUE == FETCH_UNIQUE {
                    out.insert(key, data);
                } else {
                    match out.get_mut(&key) {
                        Some(Value::Array(g)) => {
                            g.push(data);
                        }
                        _ => {
                            let mut g = PhpArray::new();
                            g.push(data);
                            out.insert(key, Value::Array(g));
                        }
                    }
                }
            }
        } else {
            while let Some(data) = do_fetch(cx, stmt, how, None)? {
                out.push(data);
            }
        }
        Ok(())
    })();
    restore(stmt);
    r?;
    if stmt.has_error() {
        handle_error(cx, dbh, Some(stmt))?;
        return Ok(Value::Array(PhpArray::new()));
    }
    Ok(Value::Array(out))
}

/// PDOStatement::debugDumpParams
fn debug_dump_params(cx: &mut Cx, stmt: &Stmt) -> Result<Value, Thrown> {
    let mut out = Vec::new();
    let q = stmt.query_string.borrow().clone();
    out.extend_from_slice(format!("SQL: [{}] ", q.len()).as_bytes());
    out.extend_from_slice(&q);
    out.push(b'\n');
    if let Some(a) = stmt.active_query_string.borrow().as_ref() {
        if *a != q {
            out.extend_from_slice(format!("Sent SQL: [{}] ", a.len()).as_bytes());
            out.extend_from_slice(a);
            out.push(b'\n');
        }
    }
    let params = stmt.bound_params.borrow().clone();
    out.extend_from_slice(
        format!("Params:  {}\n", params.as_ref().map_or(0, |p| p.len())).as_bytes(),
    );
    if let Some(p) = params {
        for (k, param) in p.iter() {
            match k {
                PKey::Name(n) => {
                    out.extend_from_slice(format!("Key: Name: [{}] ", n.len()).as_bytes());
                    out.extend_from_slice(n);
                    out.push(b'\n');
                }
                PKey::Num(n) => out.extend_from_slice(format!("Key: Position #{n}:\n").as_bytes()),
            }
            let name = param.name.clone().unwrap_or_default();
            out.extend_from_slice(
                format!("paramno={}\nname=[{}] \"", param.paramno, name.len()).as_bytes(),
            );
            out.extend_from_slice(&name);
            out.extend_from_slice(
                format!(
                    "\"\nis_param={}\nparam_type={}\n",
                    param.is_param as i32, param.param_type as i32
                )
                .as_bytes(),
            );
        }
    }
    cx.host.write_out(&out);
    Ok(Value::Null)
}

// --- FETCH_LAZY: PDORow --------------------------------------------------------

/// The PDORow of a statement (`pdo_row_t`).
pub(crate) struct Row {
    stmt: Rc<Stmt>,
    /// The statement object, which the row keeps alive.
    _stmt_obj: Option<Value>,
}

/// pdo_get_lazy_object
fn lazy_object(cx: &mut Cx, stmt: &Stmt) -> Result<Value, Thrown> {
    if let Some(h) = stmt.lazy_object.borrow().as_ref().and_then(|w| w.upgrade()) {
        return Ok(Value::Object(h));
    }
    let Some(Value::Object(h)) = cx.host.instantiate(b"PDORow", Vec::new()) else {
        return Err(Thrown::error("Error", "Cannot instantiate PDORow"));
    };
    let me = current_stmt_rc(stmt).ok_or_else(|| Thrown::error("Error", "PDORow"))?;
    object_handlers::set_payload(
        &h,
        Rc::new(Row {
            stmt: me.0,
            _stmt_obj: me.1,
        }),
    );
    *stmt.lazy_object.borrow_mut() = Some(Rc::downgrade(&h));
    Ok(Value::Object(h))
}

thread_local! {
    /// The statement object a `__pdo_stmt` call runs on, for PDORow.
    static CURRENT: RefCell<Vec<(Rc<Stmt>, Value)>> = const { RefCell::new(Vec::new()) };
}

fn current_stmt_rc(stmt: &Stmt) -> Option<(Rc<Stmt>, Option<Value>)> {
    CURRENT.with(|c| {
        c.borrow()
            .iter()
            .rev()
            .find(|(s, _)| std::ptr::eq(Rc::as_ptr(s), stmt))
            .map(|(s, v)| (s.clone(), Some(v.clone())))
    })
}

/// Run `f` with `this` registered as the running statement object.
pub(crate) fn with_current<R>(stmt: &Rc<Stmt>, this: &Value, f: impl FnOnce() -> R) -> R {
    CURRENT.with(|c| c.borrow_mut().push((stmt.clone(), this.clone())));
    let r = f();
    CURRENT.with(|c| c.borrow_mut().pop());
    r
}

fn row_of(h: &ObjectHandle) -> Option<Rc<Row>> {
    object_handlers::payload_of::<Row>(h)
}

/// row_read_column_name / row_read_column_number
fn row_read(host: &mut dyn Host, row: &Row, name: &Value) -> Result<Option<Value>, Thrown> {
    let stmt = &row.stmt;
    let mut cx = Cx::new(host, "");
    let n = stmt.column_count.get() as i64;
    let col = match name {
        Value::Int(i) => Some(*i),
        other => {
            let s = conv::to_string_bytes(other);
            match ArrayKey::from_bytes(&s) {
                ArrayKey::Int(i) => Some(i),
                ArrayKey::Str(_) => {
                    let cols = stmt.columns.borrow();
                    cols.as_ref()
                        .and_then(|c| c.iter().position(|c| c.name == s))
                        .map(|i| i as i64)
                }
            }
        }
    };
    match col {
        Some(c) if c >= 0 && c < n => Ok(Some(fetch_value(&mut cx, stmt, c, None)?)),
        _ => Ok(None),
    }
}

/// `__pdo_row($op, $this, $name)`: PDORow's property handlers (0 read,
/// 1 write, 2 isset, 3 unset, 4 construct).
pub(super) fn fn_pdo_row(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = conv::to_int(&deref(&args[0]));
    let this = deref(&args[1]);
    let name = deref(&args[2]);
    let row = object_handlers::object_of(&this).and_then(|h| row_of(&h));
    match op {
        0 => {
            let Some(row) = row else {
                return Ok(Value::Null);
            };
            if conv::to_string_bytes(&name) == b"queryString" {
                return Ok(Value::str(row.stmt.query_string.borrow().clone()));
            }
            Ok(row_read(host, &row, &name)?.unwrap_or(Value::Null))
        }
        1 => Err(Thrown::error("Error", "Cannot write to PDORow property")),
        2 => {
            let Some(row) = row else {
                return Ok(Value::Bool(false));
            };
            Ok(Value::Bool(
                row_read(host, &row, &name)?.is_some_and(|v| !matches!(v, Value::Null)),
            ))
        }
        3 => Err(Thrown::error("Error", "Cannot unset PDORow property")),
        _ => Err(super::throw(host, "You may not create a PDORow manually")),
    }
}

fn row_dim_read(
    host: &mut dyn Host,
    h: &ObjectHandle,
    off: Option<&Value>,
    _t: object_handlers::DimType,
) -> Result<Option<Value>, Thrown> {
    let Some(off) = off else {
        return Err(Thrown::error("Error", "Cannot append to PDORow offset"));
    };
    let Some(row) = row_of(h) else {
        return Ok(Some(Value::Null));
    };
    Ok(Some(row_read(host, &row, off)?.unwrap_or(Value::Null)))
}

fn row_dim_write(
    _: &mut dyn Host,
    _: &ObjectHandle,
    off: Option<&Value>,
    _: Value,
) -> Result<(), Thrown> {
    Err(Thrown::error(
        "Error",
        if off.is_none() {
            "Cannot append to PDORow offset"
        } else {
            "Cannot write to PDORow offset"
        },
    ))
}

fn row_dim_exists(
    host: &mut dyn Host,
    h: &ObjectHandle,
    off: &Value,
    check_empty: bool,
) -> Result<bool, Thrown> {
    let Some(row) = row_of(h) else {
        return Ok(false);
    };
    if let Value::Int(c) = off {
        if !check_empty {
            return Ok(*c >= 0 && *c < row.stmt.column_count.get() as i64);
        }
    }
    Ok(match row_read(host, &row, off)? {
        None => false,
        Some(v) if check_empty => conv::to_bool(&v),
        Some(v) => !matches!(v, Value::Null),
    })
}

fn row_dim_delete(_: &mut dyn Host, _: &ObjectHandle, _: &Value) -> Result<(), Thrown> {
    Err(Thrown::error("Error", "Cannot unset PDORow offset"))
}

/// row_get_properties_for: the statement's properties and the row's
/// columns, for var_dump.
fn row_get_properties_for(
    host: &mut dyn Host,
    h: &ObjectHandle,
    purpose: object_handlers::PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    if purpose != object_handlers::PropPurpose::Debug {
        return Ok(None);
    }
    let Some(row) = row_of(h) else {
        return Ok(None);
    };
    let mut props = PhpArray::new();
    props.insert(
        ArrayKey::from_bytes(b"queryString"),
        Value::str(row.stmt.query_string.borrow().clone()),
    );
    let n = row.stmt.column_count.get();
    let mut cx = Cx::new(host, "");
    for i in 0..n {
        let name = row.stmt.column_name(i);
        if name == b"queryString" {
            continue;
        }
        let v = fetch_value(&mut cx, &row.stmt, i as i64, None)?;
        props.insert(ArrayKey::Str(PhpStr::from_slice(&name)), v);
    }
    Ok(Some(props))
}

// --- the statement iterator -----------------------------------------------------

/// `struct php_pdo_iterator`
struct StmtIter {
    stmt: Rc<Stmt>,
    obj: Value,
    key: Cell<i64>,
    fetch_ahead: RefCell<Option<Value>>,
}

impl StmtIter {
    fn advance(&self, host: &mut dyn Host) -> Result<(), Thrown> {
        *self.fetch_ahead.borrow_mut() = None;
        let mut cx = Cx::new(host, "PDOStatement::getIterator");
        let r = with_current(&self.stmt, &self.obj, || {
            do_fetch(&mut cx, &self.stmt, FETCH_USE_DEFAULT, None)
        });
        let r = match r {
            Ok(Some(v)) => {
                *self.fetch_ahead.borrow_mut() = Some(v);
                self.key.set(self.key.get().wrapping_add(1));
                Ok(())
            }
            Ok(None) => {
                let dbh = self.stmt.dbh();
                let r = handle_stmt_err(&mut cx, &dbh, &self.stmt);
                self.key.set(-1);
                r
            }
            Err(t) => Err(t),
        };
        cx.finish(r.map(|_| Value::Null)).map(|_| ())
    }
}

impl IteratorFuncs for StmtIter {
    fn valid(&self, _: &mut dyn Host) -> Result<bool, Thrown> {
        Ok(self.fetch_ahead.borrow().is_some())
    }
    fn current(&self, _: &mut dyn Host) -> Result<Option<Value>, Thrown> {
        Ok(self.fetch_ahead.borrow().clone())
    }
    fn key(&self, _: &mut dyn Host) -> Result<Option<Value>, Thrown> {
        Ok(Some(if self.key.get() == -1 {
            Value::Null
        } else {
            Value::Int(self.key.get())
        }))
    }
    fn move_forward(&self, host: &mut dyn Host) -> Result<(), Thrown> {
        self.advance(host)
    }
    fn rewind(&self, _: &mut dyn Host) -> Result<(), Thrown> {
        Ok(())
    }
}

/// pdo_stmt_iter_get
fn stmt_iter_get(
    host: &mut dyn Host,
    h: &ObjectHandle,
    by_ref: bool,
) -> Result<ObjectHandle, Thrown> {
    if by_ref {
        return Err(Thrown::error(
            "Error",
            "An iterator cannot be used with foreach by reference",
        ));
    }
    let obj = Value::Object(h.clone());
    let stmt = get_obj(&*host, &obj)?;
    let it = StmtIter {
        stmt,
        obj,
        key: Cell::new(-1),
        fetch_ahead: RefCell::new(None),
    };
    it.advance(host)?;
    if it.fetch_ahead.borrow().is_some() {
        it.key.set(0);
    }
    crate::zend::internal_iterator::create(host, Box::new(it))
}
