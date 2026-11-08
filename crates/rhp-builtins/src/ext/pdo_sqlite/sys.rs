//! The part of libsqlite3's API pdo_sqlite calls (sqlite3.h), declared by
//! hand. `build/native.rs` links the libsqlite3 php links.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_double, c_int, c_void};

#[repr(C)]
pub struct sqlite3 {
    _p: [u8; 0],
}
#[repr(C)]
pub struct sqlite3_stmt {
    _p: [u8; 0],
}
#[repr(C)]
pub struct sqlite3_context {
    _p: [u8; 0],
}
#[repr(C)]
pub struct sqlite3_value {
    _p: [u8; 0],
}
#[repr(C)]
pub struct sqlite3_blob {
    _p: [u8; 0],
}

pub const SQLITE_OK: c_int = 0;
pub const SQLITE_ERROR: c_int = 1;
pub const SQLITE_INTERRUPT: c_int = 9;
pub const SQLITE_NOTFOUND: c_int = 12;
pub const SQLITE_TOOBIG: c_int = 18;
pub const SQLITE_CONSTRAINT: c_int = 19;
pub const SQLITE_NOLFS: c_int = 22;
pub const SQLITE_ROW: c_int = 100;
pub const SQLITE_DONE: c_int = 101;

pub const SQLITE_DENY: c_int = 1;
pub const SQLITE_IGNORE: c_int = 2;
pub const SQLITE_ATTACH: c_int = 24;

pub const SQLITE_OPEN_READONLY: c_int = 0x1;
pub const SQLITE_OPEN_READWRITE: c_int = 0x2;
pub const SQLITE_OPEN_CREATE: c_int = 0x4;
pub const SQLITE_OPEN_URI: c_int = 0x40;

pub const SQLITE_INTEGER: c_int = 1;
pub const SQLITE_FLOAT: c_int = 2;
pub const SQLITE_TEXT: c_int = 3;
pub const SQLITE_BLOB: c_int = 4;
pub const SQLITE_NULL: c_int = 5;

pub const SQLITE_UTF8: c_int = 1;
pub const SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION: c_int = 1005;

pub type Destructor = isize;
/// `SQLITE_TRANSIENT`: sqlite copies the bytes.
pub const SQLITE_TRANSIENT: Destructor = -1;

pub type XFunc = unsafe extern "C" fn(*mut sqlite3_context, c_int, *mut *mut sqlite3_value);
pub type XFinal = unsafe extern "C" fn(*mut sqlite3_context);
pub type XCompare =
    unsafe extern "C" fn(*mut c_void, c_int, *const c_void, c_int, *const c_void) -> c_int;
pub type XAuth = unsafe extern "C" fn(
    *mut c_void,
    c_int,
    *const c_char,
    *const c_char,
    *const c_char,
    *const c_char,
) -> c_int;

extern "C" {
    pub fn sqlite3_libversion() -> *const c_char;
    pub fn sqlite3_open_v2(
        filename: *const c_char,
        db: *mut *mut sqlite3,
        flags: c_int,
        vfs: *const c_char,
    ) -> c_int;
    pub fn sqlite3_close_v2(db: *mut sqlite3) -> c_int;
    pub fn sqlite3_errcode(db: *mut sqlite3) -> c_int;
    pub fn sqlite3_errmsg(db: *mut sqlite3) -> *const c_char;
    pub fn sqlite3_exec(
        db: *mut sqlite3,
        sql: *const c_char,
        cb: *const c_void,
        arg: *mut c_void,
        err: *mut *mut c_char,
    ) -> c_int;
    pub fn sqlite3_changes(db: *mut sqlite3) -> c_int;
    pub fn sqlite3_last_insert_rowid(db: *mut sqlite3) -> i64;
    pub fn sqlite3_get_autocommit(db: *mut sqlite3) -> c_int;
    pub fn sqlite3_busy_timeout(db: *mut sqlite3, ms: c_int) -> c_int;
    pub fn sqlite3_extended_result_codes(db: *mut sqlite3, onoff: c_int) -> c_int;
    pub fn sqlite3_interrupt(db: *mut sqlite3);
    pub fn sqlite3_set_authorizer(db: *mut sqlite3, cb: Option<XAuth>, arg: *mut c_void) -> c_int;
    pub fn sqlite3_db_config(db: *mut sqlite3, op: c_int, ...) -> c_int;
    pub fn sqlite3_load_extension(
        db: *mut sqlite3,
        file: *const c_char,
        proc_: *const c_char,
        err: *mut *mut c_char,
    ) -> c_int;
    pub fn sqlite3_free(p: *mut c_void);

    pub fn sqlite3_prepare_v2(
        db: *mut sqlite3,
        sql: *const c_char,
        n: c_int,
        stmt: *mut *mut sqlite3_stmt,
        tail: *mut *const c_char,
    ) -> c_int;
    pub fn sqlite3_step(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_reset(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_finalize(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_data_count(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_column_count(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_column_name(stmt: *mut sqlite3_stmt, i: c_int) -> *const c_char;
    pub fn sqlite3_column_type(stmt: *mut sqlite3_stmt, i: c_int) -> c_int;
    pub fn sqlite3_column_int64(stmt: *mut sqlite3_stmt, i: c_int) -> i64;
    pub fn sqlite3_column_double(stmt: *mut sqlite3_stmt, i: c_int) -> c_double;
    pub fn sqlite3_column_blob(stmt: *mut sqlite3_stmt, i: c_int) -> *const c_void;
    pub fn sqlite3_column_text(stmt: *mut sqlite3_stmt, i: c_int) -> *const u8;
    pub fn sqlite3_column_bytes(stmt: *mut sqlite3_stmt, i: c_int) -> c_int;
    pub fn sqlite3_column_decltype(stmt: *mut sqlite3_stmt, i: c_int) -> *const c_char;
    pub fn sqlite3_column_table_name(stmt: *mut sqlite3_stmt, i: c_int) -> *const c_char;
    pub fn sqlite3_bind_parameter_index(stmt: *mut sqlite3_stmt, name: *const c_char) -> c_int;
    pub fn sqlite3_bind_null(stmt: *mut sqlite3_stmt, i: c_int) -> c_int;
    pub fn sqlite3_bind_int64(stmt: *mut sqlite3_stmt, i: c_int, v: i64) -> c_int;
    pub fn sqlite3_bind_text(
        stmt: *mut sqlite3_stmt,
        i: c_int,
        p: *const c_char,
        n: c_int,
        d: Destructor,
    ) -> c_int;
    pub fn sqlite3_bind_blob(
        stmt: *mut sqlite3_stmt,
        i: c_int,
        p: *const c_void,
        n: c_int,
        d: Destructor,
    ) -> c_int;
    pub fn sqlite3_stmt_readonly(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_stmt_busy(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_stmt_isexplain(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_stmt_explain(stmt: *mut sqlite3_stmt, mode: c_int) -> c_int;

    pub fn sqlite3_create_function(
        db: *mut sqlite3,
        name: *const c_char,
        argc: c_int,
        enc: c_int,
        app: *mut c_void,
        func: Option<XFunc>,
        step: Option<XFunc>,
        fin: Option<XFinal>,
    ) -> c_int;
    pub fn sqlite3_create_collation(
        db: *mut sqlite3,
        name: *const c_char,
        enc: c_int,
        arg: *mut c_void,
        cmp: Option<XCompare>,
    ) -> c_int;
    pub fn sqlite3_user_data(ctx: *mut sqlite3_context) -> *mut c_void;
    pub fn sqlite3_aggregate_context(ctx: *mut sqlite3_context, n: c_int) -> *mut c_void;
    pub fn sqlite3_value_type(v: *mut sqlite3_value) -> c_int;
    pub fn sqlite3_value_int(v: *mut sqlite3_value) -> c_int;
    pub fn sqlite3_value_double(v: *mut sqlite3_value) -> c_double;
    pub fn sqlite3_value_text(v: *mut sqlite3_value) -> *const u8;
    pub fn sqlite3_value_bytes(v: *mut sqlite3_value) -> c_int;
    pub fn sqlite3_result_int(ctx: *mut sqlite3_context, v: c_int);
    pub fn sqlite3_result_null(ctx: *mut sqlite3_context);
    pub fn sqlite3_result_double(ctx: *mut sqlite3_context, v: c_double);
    pub fn sqlite3_result_text(
        ctx: *mut sqlite3_context,
        p: *const c_char,
        n: c_int,
        d: Destructor,
    );
    pub fn sqlite3_result_error(ctx: *mut sqlite3_context, msg: *const c_char, n: c_int);

    pub fn sqlite3_blob_open(
        db: *mut sqlite3,
        dbname: *const c_char,
        table: *const c_char,
        column: *const c_char,
        row: i64,
        flags: c_int,
        blob: *mut *mut sqlite3_blob,
    ) -> c_int;
    pub fn sqlite3_blob_close(blob: *mut sqlite3_blob) -> c_int;
    pub fn sqlite3_blob_bytes(blob: *mut sqlite3_blob) -> c_int;
    pub fn sqlite3_blob_read(
        blob: *mut sqlite3_blob,
        p: *mut c_void,
        n: c_int,
        off: c_int,
    ) -> c_int;
    pub fn sqlite3_blob_write(
        blob: *mut sqlite3_blob,
        p: *const c_void,
        n: c_int,
        off: c_int,
    ) -> c_int;
}

/// A C string as bytes (empty for NULL).
///
/// # Safety
/// `p` is NULL or a NUL-terminated string that outlives the result.
pub unsafe fn cstr<'a>(p: *const c_char) -> &'a [u8] {
    if p.is_null() {
        return &[];
    }
    unsafe { std::ffi::CStr::from_ptr(p) }.to_bytes()
}

/// `sqlite3_libversion()`.
pub fn libversion() -> String {
    String::from_utf8_lossy(unsafe { cstr(sqlite3_libversion()) }).into_owned()
}
