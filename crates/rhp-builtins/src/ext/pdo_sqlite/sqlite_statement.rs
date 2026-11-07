//! ext/pdo_sqlite/sqlite_statement.c: a prepared statement
//! (`pdo_sqlite_stmt`).

use std::cell::Cell;
use std::ffi::c_int;
use std::rc::Rc;

use rhp_values::{ArrayKey, PhpArray, Value};

use super::sqlite_driver::{
    c_string, Handle, ATTR_BUSY_STATEMENT, ATTR_EXPLAIN_STATEMENT, ATTR_READONLY_STATEMENT,
};
use super::sys;
use crate::ext::pdo::driver::*;
use crate::ext::pdo::stmt::BoundParam;
use crate::ext::pdo::{raise_impl_error, Cx, Stmt};
use crate::Thrown;

pub(crate) struct SqliteStmt {
    h: Rc<Handle>,
    stmt: Cell<*mut sys::sqlite3_stmt>,
    pre_fetched: Cell<bool>,
    done: Cell<bool>,
}

impl SqliteStmt {
    pub(crate) fn new(h: Rc<Handle>, stmt: *mut sys::sqlite3_stmt) -> SqliteStmt {
        SqliteStmt {
            h,
            stmt: Cell::new(stmt),
            pre_fetched: Cell::new(false),
            done: Cell::new(false),
        }
    }

    fn s(&self) -> *mut sys::sqlite3_stmt {
        self.stmt.get()
    }

    fn error(&self, stmt: &Stmt) {
        let dbh = stmt.dbh();
        self.h.error(&dbh, Some(stmt));
    }

    fn step(&self, cx: &mut Cx) -> c_int {
        let s = self.s();
        self.h.run(cx, || unsafe { sys::sqlite3_step(s) })
    }

    /// Bind `v` as text, after try_convert_to_string.
    fn bind_text(&self, cx: &mut Cx, stmt: &Stmt, param: &mut BoundParam) -> Result<bool, Thrown> {
        let idx = (param.paramno + 1) as c_int;
        let v = param.value();
        if matches!(v, Value::Null) {
            if unsafe { sys::sqlite3_bind_null(self.s(), idx) } == sys::SQLITE_OK {
                return Ok(true);
            }
            self.error(stmt);
            return Ok(false);
        }
        let s = match &v {
            Value::Str(s) => s.as_bytes().to_vec(),
            other => {
                let s = crate::zend::zpp::zval_get_string(cx.host, other)?;
                param.set_value(Value::str(s.clone()));
                s
            }
        };
        if unsafe {
            sys::sqlite3_bind_text(
                self.s(),
                idx,
                s.as_ptr().cast(),
                s.len() as c_int,
                sys::SQLITE_TRANSIENT,
            )
        } == sys::SQLITE_OK
        {
            return Ok(true);
        }
        self.error(stmt);
        Ok(false)
    }
}

impl Drop for SqliteStmt {
    /// pdo_sqlite_stmt_dtor
    fn drop(&mut self) {
        let s = self.stmt.replace(std::ptr::null_mut());
        if !s.is_null() {
            unsafe { sys::sqlite3_finalize(s) };
        }
    }
}

impl StmtDriver for SqliteStmt {
    /// pdo_sqlite_stmt_execute
    fn execute(&self, cx: &mut Cx, stmt: &Stmt) -> bool {
        let s = self.s();
        if stmt.executed.get() && !self.done.get() {
            unsafe { sys::sqlite3_reset(s) };
        }
        self.done.set(false);
        match self.step(cx) {
            sys::SQLITE_ROW => {
                self.pre_fetched.set(true);
                stmt.set_column_count(unsafe { sys::sqlite3_data_count(s) });
                true
            }
            sys::SQLITE_DONE => {
                stmt.set_column_count(unsafe { sys::sqlite3_column_count(s) });
                stmt.row_count
                    .set(unsafe { sys::sqlite3_changes(self.h.db.get()) } as i64);
                unsafe { sys::sqlite3_reset(s) };
                self.done.set(true);
                true
            }
            rc => {
                if rc == sys::SQLITE_ERROR {
                    unsafe { sys::sqlite3_reset(s) };
                }
                self.error(stmt);
                false
            }
        }
    }

    /// pdo_sqlite_stmt_fetch
    fn fetch(&self, cx: &mut Cx, stmt: &Stmt) -> bool {
        let s = self.s();
        if s.is_null() {
            return false;
        }
        if self.pre_fetched.get() {
            self.pre_fetched.set(false);
            return true;
        }
        if self.done.get() {
            return false;
        }
        match self.step(cx) {
            sys::SQLITE_ROW => true,
            sys::SQLITE_DONE => {
                self.done.set(true);
                unsafe { sys::sqlite3_reset(s) };
                false
            }
            rc => {
                if rc == sys::SQLITE_ERROR {
                    unsafe { sys::sqlite3_reset(s) };
                }
                self.error(stmt);
                false
            }
        }
    }

    /// pdo_sqlite_stmt_describe
    fn describe(&self, _cx: &mut Cx, stmt: &Stmt, col: i32) -> Option<Vec<u8>> {
        let s = self.s();
        if col >= unsafe { sys::sqlite3_column_count(s) } {
            self.error(stmt);
            return None;
        }
        Some(unsafe { sys::cstr(sys::sqlite3_column_name(s, col)) }.to_vec())
    }

    /// pdo_sqlite_stmt_get_col
    fn get_col(&self, _cx: &mut Cx, stmt: &Stmt, col: i32) -> Value {
        let s = self.s();
        if s.is_null() {
            return Value::Null;
        }
        if col >= unsafe { sys::sqlite3_data_count(s) } {
            self.error(stmt);
            return Value::Null;
        }
        unsafe {
            match sys::sqlite3_column_type(s, col) {
                sys::SQLITE_NULL => Value::Null,
                sys::SQLITE_INTEGER => Value::Int(sys::sqlite3_column_int64(s, col)),
                sys::SQLITE_FLOAT => Value::Float(sys::sqlite3_column_double(s, col)),
                sys::SQLITE_BLOB => {
                    let p = sys::sqlite3_column_blob(s, col) as *const u8;
                    let n = sys::sqlite3_column_bytes(s, col);
                    if p.is_null() {
                        Value::str("")
                    } else {
                        Value::str(std::slice::from_raw_parts(p, n.max(0) as usize))
                    }
                }
                _ => {
                    let p = sys::sqlite3_column_text(s, col);
                    let n = sys::sqlite3_column_bytes(s, col);
                    if p.is_null() {
                        Value::str("")
                    } else {
                        Value::str(std::slice::from_raw_parts(p, n.max(0) as usize))
                    }
                }
            }
        }
    }

    /// pdo_sqlite_stmt_param_hook
    fn param_hook(
        &self,
        cx: &mut Cx,
        stmt: &Stmt,
        param: &mut BoundParam,
        event: u32,
    ) -> Result<bool, Thrown> {
        if event != PARAM_EVT_EXEC_PRE {
            return Ok(true);
        }
        let s = self.s();
        if stmt.executed.get() && !self.done.get() {
            unsafe { sys::sqlite3_reset(s) };
            self.done.set(true);
        }
        if !param.is_param {
            return Ok(true);
        }
        if param.paramno == -1 {
            let name = c_string(param.name.as_deref().unwrap_or_default());
            param.paramno =
                unsafe { sys::sqlite3_bind_parameter_index(s, name.as_ptr()) } as i64 - 1;
        }
        let idx = (param.paramno + 1) as c_int;
        match param_type(param.param_type) {
            PARAM_STMT => Ok(false),
            PARAM_NULL => {
                if unsafe { sys::sqlite3_bind_null(s, idx) } == sys::SQLITE_OK {
                    return Ok(true);
                }
                self.error(stmt);
                Ok(false)
            }
            PARAM_INT | PARAM_BOOL => {
                let v = param.value();
                let rc = if matches!(v, Value::Null) {
                    unsafe { sys::sqlite3_bind_null(s, idx) }
                } else {
                    let n = rhp_values::conv::to_int(&v);
                    param.set_value(Value::Int(n));
                    unsafe { sys::sqlite3_bind_int64(s, idx, n) }
                };
                if rc == sys::SQLITE_OK {
                    return Ok(true);
                }
                self.error(stmt);
                Ok(false)
            }
            PARAM_LOB => {
                let v = param.value();
                let data = match &v {
                    Value::Resource(_) => {
                        if !cx.host.stream_is_open(&v) {
                            let dbh = stmt.dbh();
                            raise_impl_error(
                                cx,
                                &dbh,
                                Some(stmt),
                                "HY105",
                                Some("Expected a stream resource"),
                            )?;
                            return Ok(false);
                        }
                        let mut all = Vec::new();
                        while let Some(chunk) = cx.host.stream_read(&v, 8192) {
                            if chunk.is_empty() {
                                break;
                            }
                            all.extend_from_slice(&chunk);
                        }
                        param.set_value(Value::str(all.clone()));
                        all
                    }
                    Value::Null => {
                        if unsafe { sys::sqlite3_bind_null(s, idx) } == sys::SQLITE_OK {
                            return Ok(true);
                        }
                        self.error(stmt);
                        return Ok(false);
                    }
                    Value::Str(x) => x.as_bytes().to_vec(),
                    other => {
                        let x = crate::zend::zpp::zval_get_string(cx.host, other)?;
                        param.set_value(Value::str(x.clone()));
                        x
                    }
                };
                Ok(unsafe {
                    sys::sqlite3_bind_blob(
                        s,
                        idx,
                        data.as_ptr().cast(),
                        data.len() as c_int,
                        sys::SQLITE_TRANSIENT,
                    )
                } == sys::SQLITE_OK)
            }
            _ => self.bind_text(cx, stmt, param),
        }
    }

    /// pdo_sqlite_stmt_set_attribute
    fn set_attribute(
        &self,
        cx: &mut Cx,
        _stmt: &Stmt,
        attr: i64,
        v: &Value,
    ) -> Result<bool, Thrown> {
        if attr != ATTR_EXPLAIN_STATEMENT {
            return Ok(false);
        }
        let Value::Int(m) = v else {
            return Err(Thrown::type_error(format!(
                "explain mode must be of type int, {} given",
                cx.value_name(v)
            )));
        };
        if !(0..=2).contains(m) {
            return Err(Thrown::error(
                "ValueError",
                "explain mode must be one of the Pdo\\Sqlite::EXPLAIN_MODE_* constants",
            ));
        }
        Ok(unsafe { sys::sqlite3_stmt_explain(self.s(), *m as c_int) } == sys::SQLITE_OK)
    }

    /// pdo_sqlite_stmt_get_attribute
    fn get_attribute(&self, _stmt: &Stmt, attr: i64) -> Result<(i32, Value), Thrown> {
        let s = self.s();
        Ok(match attr {
            ATTR_READONLY_STATEMENT => (
                1,
                Value::Bool(unsafe { sys::sqlite3_stmt_readonly(s) } != 0),
            ),
            ATTR_BUSY_STATEMENT => (1, Value::Bool(unsafe { sys::sqlite3_stmt_busy(s) } != 0)),
            ATTR_EXPLAIN_STATEMENT => (
                1,
                Value::Int(unsafe { sys::sqlite3_stmt_isexplain(s) } as i64),
            ),
            _ => (0, Value::Null),
        })
    }

    /// pdo_sqlite_stmt_col_meta
    fn column_meta(&self, _cx: &mut Cx, stmt: &Stmt, col: i64) -> Option<PhpArray> {
        let s = self.s();
        if s.is_null() || !stmt.executed.get() {
            return None;
        }
        if col >= unsafe { sys::sqlite3_column_count(s) } as i64 {
            self.error(stmt);
            return None;
        }
        let col = col as c_int;
        let mut out = PhpArray::new();
        let mut flags = PhpArray::new();
        let put = |out: &mut PhpArray, k: &str, v: Value| {
            out.insert(ArrayKey::from_bytes(k.as_bytes()), v);
        };
        match unsafe { sys::sqlite3_column_type(s, col) } {
            sys::SQLITE_NULL => {
                put(&mut out, "native_type", Value::str("null"));
                put(&mut out, "pdo_type", Value::Int(PARAM_NULL));
            }
            sys::SQLITE_FLOAT => {
                put(&mut out, "native_type", Value::str("double"));
                put(&mut out, "pdo_type", Value::Int(PARAM_STR));
            }
            sys::SQLITE_BLOB => {
                flags.push(Value::str("blob"));
                put(&mut out, "native_type", Value::str("string"));
                put(&mut out, "pdo_type", Value::Int(PARAM_STR));
            }
            sys::SQLITE_TEXT => {
                put(&mut out, "native_type", Value::str("string"));
                put(&mut out, "pdo_type", Value::Int(PARAM_STR));
            }
            sys::SQLITE_INTEGER => {
                put(&mut out, "native_type", Value::str("integer"));
                put(&mut out, "pdo_type", Value::Int(PARAM_INT));
            }
            _ => {}
        }
        let decl = unsafe { sys::sqlite3_column_decltype(s, col) };
        if !decl.is_null() {
            put(
                &mut out,
                "sqlite:decl_type",
                Value::str(unsafe { sys::cstr(decl) }),
            );
        }
        let table = unsafe { sys::sqlite3_column_table_name(s, col) };
        if !table.is_null() {
            put(&mut out, "table", Value::str(unsafe { sys::cstr(table) }));
        }
        put(&mut out, "flags", Value::Array(flags));
        Some(out)
    }

    /// pdo_sqlite_stmt_cursor_closer
    fn close_cursor(&self, _stmt: &Stmt) -> bool {
        unsafe { sys::sqlite3_reset(self.s()) };
        true
    }
}
