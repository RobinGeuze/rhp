//! The `pdo_sqlite` extension on libsqlite3: a port of ext/pdo_sqlite
//! (pdo_sqlite.c, sqlite_driver.c, sqlite_statement.c). The driver plugs
//! into ext/pdo ([`crate::ext::pdo::driver`]); `Pdo\Sqlite` is declared in
//! PHP ([`PRELUDE_PHP`]) and its methods call `__pdo_sqlite`.

pub(crate) mod sqlite_driver;
mod sqlite_statement;
pub(crate) mod sys;
#[cfg(test)]
mod tests;

use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use rhp_values::{conv, Value};

use crate::ext::pdo::dbh::construct_check;
use crate::ext::pdo::{self, deref, Cx};
use crate::main::streams::ops::{OpsStream, StreamOps};
use crate::{Host, StatInfo, Thrown, E_WARNING};
use sqlite_driver::{c_string, handle_of, Collation, Func};

pub const PRELUDE_PHP: &str = include_str!("prelude.php");

/// PHP_MINFO_FUNCTION(pdo_sqlite)
pub fn info_rows() -> Vec<(String, String)> {
    vec![
        ("PDO Driver for SQLite 3.x".into(), "enabled".into()),
        ("SQLite Library".into(), sys::libversion()),
    ]
}

pub fn register(m: &mut crate::Registry) {
    crate::register_helpers(m, &[("__pdo_sqlite", fn_pdo_sqlite, 2, None)]);
}

fn arg(a: &[Value], i: usize) -> Value {
    a.get(i).map(deref).unwrap_or(Value::Null)
}

/// A callback kept for later calls, resolved now in the caller's scope as
/// php's `zend_fcall_info_cache` is (`[$this, 'privateMethod']` keeps
/// working when sqlite calls it). Only an array's method depends on the
/// scope; other callables stay as they are.
pub(crate) fn keep_callback(host: &dyn Host, cb: Value) -> Value {
    match cb {
        Value::Array(_) => host.wrap_callable(cb),
        cb => cb,
    }
}

/// `__pdo_sqlite($method, $this, ...$args)`: Pdo\Sqlite's own methods.
fn fn_pdo_sqlite(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let method = String::from_utf8_lossy(&conv::to_string_bytes(&deref(&args[0]))).into_owned();
    let this = deref(&args[1]);
    let a = &args[2..];
    if let Some(m) = method.strip_prefix("sqlite") {
        let mut cx = Cx::new(host, format!("PDO::{method}"));
        let r = driver_method(&mut cx, m, &this, a);
        return cx.finish(r);
    }
    let mut cx = Cx::new(host, format!("Pdo\\Sqlite::{method}"));
    let r = method_call(&mut cx, &method, &this, a);
    cx.finish(r)
}

/// PDO_SQLite_Ext's methods (`PDO::sqliteCreateFunction()` …), which
/// pdo_hash_methods marks deprecated: the deprecation, the method's own
/// parameter parsing, then Pdo\Sqlite's implementation. `m` is the name
/// after `sqlite`.
fn driver_method(cx: &mut Cx, m: &str, this: &Value, a: &[Value]) -> Result<Value, Thrown> {
    use crate::zend::zpp;
    let fname = cx.fname.clone();
    // A connection of another driver (or none) has no such method.
    let connected = crate::ext::pdo::dbh_of(&*cx.host, this).and_then(|d| handle_of(&d));
    if connected.is_none() {
        return Err(Thrown::error(
            "Error",
            format!(
                "Call to undefined method {}::sqlite{m}()",
                crate::ext::pdo::class_of(&*cx.host, this)
            ),
        ));
    }
    let new = match m {
        "CreateFunction" => "createFunction",
        "CreateAggregate" => "createAggregate",
        _ => "createCollation",
    };
    cx.host.emit_diagnostic(
        crate::E_DEPRECATED,
        format!("Method {fname}() is deprecated since 8.5, use Pdo\\Sqlite::{new}() instead")
            .as_bytes(),
    )?;
    let (min, max) = match m {
        "CreateFunction" => (2, 4),
        "CreateAggregate" => (3, 4),
        _ => (2, 2),
    };
    if a.len() < min || a.len() > max {
        let (which, n) = match (a.len() < min, min == max) {
            (_, true) => ("exactly", min),
            (true, false) => ("at least", min),
            (false, false) => ("at most", max),
        };
        return Err(Thrown::error(
            "ArgumentCountError",
            format!(
                "{fname}() expects {which} {n} argument{}, {} given",
                if n == 1 { "" } else { "s" },
                a.len()
            ),
        ));
    }
    let mut args = a.to_vec();
    args[0] = Value::Str(rhp_values::PhpStr::from_vec(zpp::str_param(
        cx.host, a, 0, &fname, "name",
    )?));
    let ints: &[(usize, &str)] = match m {
        "CreateFunction" => &[(2, "numArgs"), (3, "flags")],
        "CreateAggregate" => &[(3, "numArgs")],
        _ => &[],
    };
    for &(i, p) in ints {
        if i < a.len() {
            args[i] = Value::Int(zpp::int_param(&*cx.host, a, i, &fname, p, 0)?);
        }
    }
    method_call(cx, new, this, &args)
}

fn method_call(cx: &mut Cx, method: &str, this: &Value, a: &[Value]) -> Result<Value, Thrown> {
    match method {
        "createFunction" => {
            let fname = cx.fname.clone();
            crate::zend::zpp::require_callback(cx.host, &fname, 2, &arg(a, 1))?;
            let dbh = construct_check(&*cx.host, this)?;
            let h = handle_of(&dbh).ok_or_else(|| Thrown::type_error("not a sqlite connection"))?;
            let name = c_string(&conv::to_string_bytes(&arg(a, 0)));
            let argc = if a.len() > 2 {
                conv::to_int(&arg(a, 2))
            } else {
                -1
            } as c_int;
            let flags = if a.len() > 3 {
                conv::to_int(&arg(a, 3))
            } else {
                0
            } as c_int;
            let f = Box::new(Func {
                name,
                argc,
                func: Some(keep_callback(&*cx.host, arg(a, 1))),
                step: None,
                fini: None,
            });
            let rc = unsafe {
                sys::sqlite3_create_function(
                    h.db.get(),
                    f.name.as_ptr(),
                    argc,
                    flags | sys::SQLITE_UTF8,
                    &*f as *const Func as *mut c_void,
                    Some(sqlite_driver::func_callback),
                    None,
                    None,
                )
            };
            if rc == sys::SQLITE_OK {
                h.funcs.borrow_mut().push(f);
                return Ok(Value::Bool(true));
            }
            Ok(Value::Bool(false))
        }
        "createAggregate" => {
            let fname = cx.fname.clone();
            for (i, p) in [(1, "step"), (2, "finalize")] {
                let cb = arg(a, i);
                if let Some(why) = cx.host.check_callable(&cb)? {
                    return Err(Thrown::type_error(format!(
                        "{fname}(): Argument #{} (${p}) must be a valid callback, {why}",
                        i + 1
                    )));
                }
            }
            let dbh = construct_check(&*cx.host, this)?;
            let h = handle_of(&dbh).ok_or_else(|| Thrown::type_error("not a sqlite connection"))?;
            let name = c_string(&conv::to_string_bytes(&arg(a, 0)));
            let argc = if a.len() > 3 {
                conv::to_int(&arg(a, 3))
            } else {
                -1
            } as c_int;
            let f = Box::new(Func {
                name,
                argc,
                func: None,
                step: Some(keep_callback(&*cx.host, arg(a, 1))),
                fini: Some(keep_callback(&*cx.host, arg(a, 2))),
            });
            let rc = unsafe {
                sys::sqlite3_create_function(
                    h.db.get(),
                    f.name.as_ptr(),
                    argc,
                    sys::SQLITE_UTF8,
                    &*f as *const Func as *mut c_void,
                    None,
                    Some(sqlite_driver::step_callback),
                    Some(sqlite_driver::final_callback),
                )
            };
            if rc == sys::SQLITE_OK {
                h.funcs.borrow_mut().push(f);
                return Ok(Value::Bool(true));
            }
            Ok(Value::Bool(false))
        }
        "createCollation" => {
            let fname = cx.fname.clone();
            crate::zend::zpp::require_callback(cx.host, &fname, 2, &arg(a, 1))?;
            let dbh = construct_check(&*cx.host, this)?;
            let h = handle_of(&dbh).ok_or_else(|| Thrown::type_error("not a sqlite connection"))?;
            let c = Box::new(Collation {
                name: c_string(&conv::to_string_bytes(&arg(a, 0))),
                callback: keep_callback(&*cx.host, arg(a, 1)),
            });
            let rc = unsafe {
                sys::sqlite3_create_collation(
                    h.db.get(),
                    c.name.as_ptr(),
                    sys::SQLITE_UTF8,
                    &*c as *const Collation as *mut c_void,
                    Some(sqlite_driver::collation_callback),
                )
            };
            if rc == sys::SQLITE_OK {
                h.collations.borrow_mut().push(c);
                return Ok(Value::Bool(true));
            }
            Ok(Value::Bool(false))
        }
        "setAuthorizer" => {
            let cb = arg(a, 0);
            if !matches!(cb, Value::Null) {
                if let Some(why) = cx.host.check_callable(&cb)? {
                    return Err(Thrown::type_error(format!(
                        "{}(): Argument #1 ($callback) must be a valid callback or null, {why}",
                        cx.fname
                    )));
                }
            }
            let dbh = construct_check(&*cx.host, this)?;
            let h = handle_of(&dbh).ok_or_else(|| Thrown::type_error("not a sqlite connection"))?;
            *h.authorizer.borrow_mut() = match cb {
                Value::Null => None,
                v => Some(keep_callback(&*cx.host, v)),
            };
            Ok(Value::Null)
        }
        "loadExtension" => {
            let ext = conv::to_string_bytes(&arg(a, 0));
            if ext.contains(&0) {
                return Err(cx.value_error(1, "name", "must not contain any null bytes"));
            }
            if ext.is_empty() {
                return Err(cx.value_error(1, "name", "must not be empty"));
            }
            let dbh = construct_check(&*cx.host, this)?;
            let h = handle_of(&dbh).ok_or_else(|| Thrown::type_error("not a sqlite connection"))?;
            let Ok(full) = std::fs::canonicalize(crate::zend::virtual_cwd::os_path(&ext)) else {
                let msg = format!(
                    "Unable to load extension \"{}\"",
                    String::from_utf8_lossy(&ext)
                );
                return Err(pdo::throw(cx.host, &msg));
            };
            use std::os::unix::ffi::OsStrExt;
            let path = c_string(full.as_os_str().as_bytes());
            let db = h.db.get();
            let mut err: *mut c_char = ptr::null_mut();
            unsafe {
                sys::sqlite3_db_config(
                    db,
                    sys::SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION,
                    1 as c_int,
                    ptr::null_mut::<c_int>(),
                );
                let rc = sys::sqlite3_load_extension(db, path.as_ptr(), ptr::null(), &mut err);
                sys::sqlite3_db_config(
                    db,
                    sys::SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION,
                    0 as c_int,
                    ptr::null_mut::<c_int>(),
                );
                if rc != sys::SQLITE_OK {
                    let msg = format!(
                        "Unable to load extension \"{}\"",
                        String::from_utf8_lossy(sys::cstr(err))
                    );
                    sys::sqlite3_free(err.cast());
                    return Err(pdo::throw(cx.host, &msg));
                }
            }
            Ok(Value::Null)
        }
        "openBlob" => open_blob(cx, this, a),
        _ => Err(Thrown::error(
            "Error",
            format!("Call to undefined method {}()", cx.fname),
        )),
    }
}

/// Pdo\Sqlite::openBlob
fn open_blob(cx: &mut Cx, this: &Value, a: &[Value]) -> Result<Value, Thrown> {
    let dbh = construct_check(&*cx.host, this)?;
    let h = handle_of(&dbh).ok_or_else(|| Thrown::type_error("not a sqlite connection"))?;
    let names = ["table", "column", "", "dbname"];
    for i in [0, 1, 3] {
        if conv::to_string_bytes(&arg(a, i)).contains(&0) {
            return Err(cx.value_error(i + 1, names[i], "must not contain any null bytes"));
        }
    }
    let table = c_string(&conv::to_string_bytes(&arg(a, 0)));
    let column = c_string(&conv::to_string_bytes(&arg(a, 1)));
    let rowid = conv::to_int(&arg(a, 2));
    let dbname = match arg(a, 3) {
        Value::Null => c_string(b"main"),
        v => c_string(&conv::to_string_bytes(&v)),
    };
    let flags = if a.len() > 4 {
        conv::to_int(&arg(a, 4))
    } else {
        sys::SQLITE_OPEN_READONLY as i64
    };
    let rw = (flags & sys::SQLITE_OPEN_READWRITE as i64 != 0) as c_int;
    let db = h.db.get();
    let mut blob: *mut sys::sqlite3_blob = ptr::null_mut();
    let rc = unsafe {
        sys::sqlite3_blob_open(
            db,
            dbname.as_ptr(),
            table.as_ptr(),
            column.as_ptr(),
            rowid,
            rw,
            &mut blob,
        )
    };
    if rc != sys::SQLITE_OK {
        let msg = format!(
            "Unable to open blob: {}",
            String::from_utf8_lossy(unsafe { sys::cstr(sys::sqlite3_errmsg(db)) })
        );
        cx.host.emit_diagnostic(crate::E_WARNING, msg.as_bytes())?;
        return Ok(Value::Bool(false));
    }
    let size = unsafe { sys::sqlite3_blob_bytes(blob) }.max(0) as usize;
    let ops = BlobStream {
        blob,
        position: 0,
        size,
        readonly: flags & sys::SQLITE_OPEN_READONLY as i64 != 0,
        _conn: h,
    };
    let mode: &[u8] = if rw != 0 { b"r+b" } else { b"rb" };
    Ok(cx
        .host
        .stream_from_ops(OpsStream::new(Box::new(ops), mode, Vec::new()))
        .unwrap_or(Value::Bool(false)))
}

/// `php_stream_pdosqlite3_data` with php_stream_pdosqlite3_ops.
struct BlobStream {
    blob: *mut sys::sqlite3_blob,
    position: usize,
    size: usize,
    readonly: bool,
    /// The connection stays open while the blob does.
    _conn: std::rc::Rc<sqlite_driver::Handle>,
}

impl StreamOps for BlobStream {
    fn label(&self) -> &'static str {
        "PDOSQLite"
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
        if self.readonly {
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
        match whence {
            libc::SEEK_SET => {
                if offset < 0 || self.size < offset as usize {
                    self.position = self.size;
                    (-1, -1)
                } else {
                    self.position = offset as usize;
                    (0, offset)
                }
            }
            libc::SEEK_END => {
                if offset > 0 {
                    self.position = self.size;
                    (-1, -1)
                } else if self.size < (-offset) as usize {
                    self.position = 0;
                    (-1, -1)
                } else {
                    self.position = self.size - (-offset) as usize;
                    (0, self.position as i64)
                }
            }
            _ => (-1, self.position as i64),
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
