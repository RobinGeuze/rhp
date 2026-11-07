//! php_pdo_driver.h: what a PDO driver implements (`pdo_dbh_methods`,
//! `pdo_stmt_methods`) and the constants both sides share.

use rhp_values::{PhpArray, Value};

use super::stmt::{BoundParam, Stmt};
use super::{Cx, Dbh};
use crate::Thrown;

pub(crate) const PARAM_NULL: i64 = 0;
pub(crate) const PARAM_INT: i64 = 1;
pub(crate) const PARAM_STR: i64 = 2;
pub(crate) const PARAM_LOB: i64 = 3;
pub(crate) const PARAM_STMT: i64 = 4;
pub(crate) const PARAM_BOOL: i64 = 5;
pub(crate) const PARAM_FLAGS: i64 = 0xFFFF_0000;

/// PDO_PARAM_TYPE
pub(crate) fn param_type(t: i64) -> i64 {
    t & !PARAM_FLAGS
}

pub(crate) const FETCH_USE_DEFAULT: i64 = 0;
pub(crate) const FETCH_LAZY: i64 = 1;
pub(crate) const FETCH_ASSOC: i64 = 2;
pub(crate) const FETCH_NUM: i64 = 3;
pub(crate) const FETCH_BOTH: i64 = 4;
pub(crate) const FETCH_OBJ: i64 = 5;
pub(crate) const FETCH_BOUND: i64 = 6;
pub(crate) const FETCH_COLUMN: i64 = 7;
pub(crate) const FETCH_CLASS: i64 = 8;
pub(crate) const FETCH_INTO: i64 = 9;
pub(crate) const FETCH_FUNC: i64 = 10;
pub(crate) const FETCH_NAMED: i64 = 11;
pub(crate) const FETCH_KEY_PAIR: i64 = 12;
pub(crate) const FETCH_FLAGS: i64 = 0xFFFF_FFF0;
pub(crate) const FETCH_GROUP: i64 = 1 << 5;
pub(crate) const FETCH_UNIQUE: i64 = 1 << 6;
pub(crate) const FETCH_CLASSTYPE: i64 = 1 << 7;
pub(crate) const FETCH_PROPS_LATE: i64 = 1 << 8;
pub(crate) const FETCH_SERIALIZE: i64 = 1 << 9;
pub(crate) const FIRST_INVALID_FLAG: i64 = 1 << 10;

pub(crate) const ATTR_AUTOCOMMIT: i64 = 0;
pub(crate) const ATTR_TIMEOUT: i64 = 2;
pub(crate) const ATTR_ERRMODE: i64 = 3;
pub(crate) const ATTR_SERVER_VERSION: i64 = 4;
pub(crate) const ATTR_CLIENT_VERSION: i64 = 5;
pub(crate) const ATTR_CASE: i64 = 8;
pub(crate) const ATTR_CURSOR: i64 = 10;
pub(crate) const ATTR_ORACLE_NULLS: i64 = 11;
pub(crate) const ATTR_PERSISTENT: i64 = 12;
pub(crate) const ATTR_STATEMENT_CLASS: i64 = 13;
pub(crate) const ATTR_DRIVER_NAME: i64 = 16;
pub(crate) const ATTR_STRINGIFY_FETCHES: i64 = 17;
pub(crate) const ATTR_DEFAULT_FETCH_MODE: i64 = 19;
pub(crate) const ATTR_EMULATE_PREPARES: i64 = 20;
pub(crate) const ATTR_DRIVER_SPECIFIC: i64 = 1000;

pub(crate) const ERRMODE_SILENT: i64 = 0;
pub(crate) const ERRMODE_WARNING: i64 = 1;
pub(crate) const ERRMODE_EXCEPTION: i64 = 2;

pub(crate) const CASE_NATURAL: i64 = 0;
pub(crate) const CASE_UPPER: i64 = 1;
pub(crate) const CASE_LOWER: i64 = 2;

pub(crate) const NULL_EMPTY_STRING: i64 = 1;
pub(crate) const NULL_TO_STRING: i64 = 2;

pub(crate) const CURSOR_FWDONLY: i64 = 0;

pub(crate) const PARAM_EVT_EXEC_PRE: u32 = 2;
pub(crate) const PARAM_EVT_EXEC_POST: u32 = 3;
pub(crate) const PARAM_EVT_FETCH_PRE: u32 = 4;
pub(crate) const PARAM_EVT_FETCH_POST: u32 = 5;
pub(crate) const PARAM_EVT_NORMALIZE: u32 = 6;
pub(crate) const PARAM_EVT_ALLOC: u32 = 0;

pub(crate) const PLACEHOLDER_NONE: u8 = 0;

pub(crate) const ERR_NONE: &str = "00000";

/// pdo_attr_lval
pub(crate) fn attr_lval(options: Option<&PhpArray>, attr: i64, def: i64) -> i64 {
    options
        .and_then(|o| o.get(&rhp_values::ArrayKey::Int(attr)))
        .map(|v| rhp_values::conv::to_int(&rhp_values::ops::deref(v)))
        .unwrap_or(def)
}

/// `pdo_dbh_methods` (with the driver's `driver_data`).
pub(crate) trait DbhDriver {
    /// `preparer`: prepare `sql` for `stmt`; false with the error set.
    fn prepare(
        &self,
        cx: &mut Cx,
        dbh: &Dbh,
        sql: &[u8],
        stmt: &Stmt,
        options: Option<&PhpArray>,
    ) -> bool;
    /// `doer`: -1 on error.
    fn exec(&self, cx: &mut Cx, dbh: &Dbh, sql: &[u8]) -> i64;
    /// `quoter`: `None` on error.
    fn quote(&self, cx: &mut Cx, dbh: &Dbh, s: &[u8], ty: i64) -> Result<Option<Vec<u8>>, Thrown>;
    fn begin(&self, cx: &mut Cx, dbh: &Dbh) -> bool;
    fn commit(&self, cx: &mut Cx, dbh: &Dbh) -> bool;
    fn rollback(&self, cx: &mut Cx, dbh: &Dbh) -> bool;
    /// `set_attribute`
    fn set_attribute(&self, cx: &mut Cx, dbh: &Dbh, attr: i64, v: &Value) -> Result<bool, Thrown>;
    /// `last_id`
    fn last_id(&self, dbh: &Dbh, name: Option<&[u8]>) -> Option<Vec<u8>>;
    /// `fetch_err`: append the driver's code and message to `info`.
    fn fetch_err(&self, dbh: &Dbh, stmt: Option<&Stmt>, info: &mut PhpArray);
    /// `get_attribute`: -1 error, 0 unsupported, 1 with the value.
    fn get_attribute(&self, dbh: &Dbh, attr: i64) -> (i32, Value);
    /// `in_transaction`
    fn in_transaction(&self, dbh: &Dbh) -> bool;
    /// `rollback` from free_storage, where no PHP runs.
    fn rollback_on_free(&self);
    /// `closer` without a host (rolls back nothing).
    fn close(&self);
    /// For `get_gc`: the PHP values the driver holds (callbacks).
    fn gc_values(&self, _out: &mut Vec<Value>) {}
    /// Downcast to the driver's type (Pdo\Sqlite's methods).
    fn as_any(&self) -> &dyn std::any::Any;
}

/// `pdo_stmt_methods`.
pub(crate) trait StmtDriver {
    /// `executer`
    fn execute(&self, cx: &mut Cx, stmt: &Stmt) -> bool;
    /// `fetcher` (forward only)
    fn fetch(&self, cx: &mut Cx, stmt: &Stmt) -> bool;
    /// `describer`: the column's name.
    fn describe(&self, cx: &mut Cx, stmt: &Stmt, col: i32) -> Option<Vec<u8>>;
    /// `get_col`
    fn get_col(&self, cx: &mut Cx, stmt: &Stmt, col: i32) -> Value;
    /// `param_hook`
    fn param_hook(
        &self,
        cx: &mut Cx,
        stmt: &Stmt,
        param: &mut BoundParam,
        event: u32,
    ) -> Result<bool, Thrown>;
    /// `set_attribute`: `None` when unsupported.
    fn set_attribute(&self, cx: &mut Cx, stmt: &Stmt, attr: i64, v: &Value)
        -> Result<bool, Thrown>;
    /// `get_attribute`: -1 error, 0 unsupported, 1 with the value.
    fn get_attribute(&self, stmt: &Stmt, attr: i64) -> Result<(i32, Value), Thrown>;
    /// `get_column_meta`: `None` is FAILURE.
    fn column_meta(&self, cx: &mut Cx, stmt: &Stmt, col: i64) -> Option<PhpArray>;
    /// `cursor_closer`
    fn close_cursor(&self, stmt: &Stmt) -> bool;
}
