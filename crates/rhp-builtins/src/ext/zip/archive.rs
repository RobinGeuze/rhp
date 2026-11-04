//! `ZipArchive`'s methods (php_zip.c's `PHP_METHOD(ZipArchive, …)`),
//! dispatched by `__zip($method, $this, ...$args)`. The prelude declares the
//! parameter types, so the arguments arrive coerced.

use std::ffi::{c_int, c_void, CString, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::ptr;
use std::rc::Rc;

use rhp_values::{conv, ArrayKey, PhpArray, Value};

use super::sys::{self, zip_t};
use super::{
    c_string, check_path, deref, expand_filepath, file_error, must_not_be_empty, release,
    stat_array, zip_object, Archive, ZipObject, MAXPATHLEN,
};
use crate::ext::libxml::ffi;
use crate::{Host, Thrown};

struct Cx<'a> {
    host: &'a mut dyn Host,
    /// "ZipArchive::method", for messages.
    fname: String,
    this: Value,
    args: &'a [Value],
}

impl Cx<'_> {
    fn arg(&self, i: usize) -> Value {
        self.args.get(i).map(deref).unwrap_or(Value::Null)
    }
    fn bytes(&self, i: usize) -> Vec<u8> {
        conv::to_string_bytes(&self.arg(i))
    }
    fn int(&self, i: usize) -> i64 {
        conv::to_int(&self.arg(i))
    }
    /// The object's state (fn_zip checked `$this` is a ZipArchive).
    fn obj(&self) -> Rc<ZipObject> {
        zip_object(&*self.host, &self.this).expect("checked ZipArchive")
    }
    /// ZIP_FROM_OBJECT
    fn za(&self) -> Result<*mut zip_t, Thrown> {
        let za = self.obj().za();
        if za.is_null() {
            return Err(Thrown::error(
                "ValueError",
                "Invalid or uninitialized Zip object",
            ));
        }
        Ok(za)
    }
    fn warn(&mut self, msg: &str) -> Result<(), Thrown> {
        crate::warn(self.host, &self.fname, msg)
    }
    /// A warning raised while a callback's exception is pending, as php's
    /// `EG(exception)`: a user error handler is not called then
    /// (zend_call_function returns at once), so nothing shows; without one
    /// the warning is displayed.
    fn warn_pending(&mut self, msg: &str, pending: bool) -> Result<(), Thrown> {
        self.docref_pending(crate::E_WARNING, msg, pending)
    }
    fn docref_pending(&mut self, level: i64, msg: &str, pending: bool) -> Result<(), Thrown> {
        if pending {
            let handler = self.host.set_error_handler(Value::Null, 0);
            self.host.restore_error_handler();
            if !matches!(handler, Value::Null) {
                return Ok(());
            }
        }
        let m = format!("{}(): {msg}", self.fname);
        self.host.emit_diagnostic(level, &m)
    }
    fn value_error(&self, n: usize, pname: &str, msg: &str) -> Thrown {
        crate::zend::zpp::value_error(&self.fname, n, pname, msg)
    }
    fn empty(&self, n: usize, pname: &str) -> Thrown {
        must_not_be_empty(&self.fname, n, pname)
    }
    fn path(&self, i: usize, pname: &str) -> Result<Vec<u8>, Thrown> {
        let b = self.bytes(i);
        check_path(&self.fname, i + 1, pname, &b)?;
        Ok(b)
    }
}

pub(super) fn fn_zip(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let method = conv::to_string_bytes(&deref(&args[0]));
    let method = String::from_utf8_lossy(&method).into_owned();
    let this = deref(&args[1]);
    let is_static = matches!(
        method.as_str(),
        "isCompressionMethodSupported" | "isEncryptionMethodSupported"
    );
    if !is_static && super::zip_object(host, &this).is_none() {
        return Err(Thrown::type_error(
            "__zip(): Argument #2 must be a ZipArchive",
        ));
    }
    let mut cx = Cx {
        host,
        fname: format!("ZipArchive::{method}"),
        this,
        args: &args[2..],
    };
    let b = Value::Bool;
    Ok(match method.as_str() {
        "open" => open(&mut cx)?,
        "setPassword" => set_password(&mut cx)?,
        "close" => close(&mut cx)?,
        "count" => {
            let za = cx.za()?;
            Value::Int(unsafe { sys::zip_get_num_entries(za, 0) })
        }
        "getStatusString" => get_status_string(&mut cx),
        "clearError" => {
            let o = cx.obj();
            let za = o.za();
            if za.is_null() {
                o.err_zip.set(0);
                o.err_sys.set(0);
            } else {
                unsafe { sys::zip_error_clear(za) };
            }
            Value::Null
        }
        "addEmptyDir" => add_empty_dir(&mut cx)?,
        "addFromString" => add_from_string(&mut cx)?,
        "addFile" => add_file_method(&mut cx)?,
        "replaceFile" => replace_file(&mut cx)?,
        "addGlob" => add_from_pattern(&mut cx, true)?,
        "addPattern" => add_from_pattern(&mut cx, false)?,
        "renameIndex" => {
            let index = cx.int(0);
            if index < 0 {
                return Ok(b(false));
            }
            let za = cx.za()?;
            let name = cx.bytes(1);
            if name.is_empty() {
                return Err(cx.empty(2, "new_name"));
            }
            let n = c_string(&name);
            b(unsafe { sys::zip_file_rename(za, index as u64, n.as_ptr(), 0) } == 0)
        }
        "renameName" => {
            let za = cx.za()?;
            let new = cx.bytes(1);
            if new.is_empty() {
                return Err(cx.empty(2, "new_name"));
            }
            let Some(sb) = stat_path(&cx, za, 0, 0)? else {
                return Ok(b(false));
            };
            let n = c_string(&new);
            b(unsafe { sys::zip_file_rename(za, sb.index, n.as_ptr(), 0) } == 0)
        }
        "setArchiveComment" => {
            let za = cx.za()?;
            let c = cx.bytes(0);
            if c.len() > 0xffff {
                return Err(cx.value_error(1, "comment", "must be less than 65535 bytes"));
            }
            let cs = c_string(&c);
            b(unsafe { sys::zip_set_archive_comment(za, cs.as_ptr(), c.len() as u16) } == 0)
        }
        "getArchiveComment" => {
            let za = cx.za()?;
            let flags = cx.int(0);
            let mut len: c_int = 0;
            let p = unsafe { sys::zip_get_archive_comment(za, &mut len, flags as u32) };
            if p.is_null() {
                b(false)
            } else {
                Value::str(unsafe { std::slice::from_raw_parts(p.cast::<u8>(), len as usize) })
            }
        }
        "setArchiveFlag" => {
            let za = cx.za()?;
            let (flag, value) = (cx.int(0), cx.int(1));
            b(unsafe { sys::zip_set_archive_flag(za, flag as u32, value as c_int) } == 0)
        }
        "getArchiveFlag" => {
            let za = cx.za()?;
            let (flag, flags) = (cx.int(0), cx.int(1));
            Value::Int(unsafe { sys::zip_get_archive_flag(za, flag as u32, flags as u32) } as i64)
        }
        "setCommentName" => {
            let name = cx.bytes(0);
            if name.is_empty() {
                return Err(cx.empty(1, "name"));
            }
            let za = cx.za()?;
            let comment = cx.bytes(1);
            if comment.len() > 0xffff {
                return Err(cx.value_error(2, "comment", "must be less than 65535 bytes"));
            }
            let idx = locate(za, &name, 0);
            if idx < 0 {
                return Ok(b(false));
            }
            b(set_file_comment(za, idx as u64, &comment))
        }
        "setCommentIndex" => {
            let za = cx.za()?;
            let index = cx.int(0);
            let comment = cx.bytes(1);
            if comment.len() > 0xffff {
                return Err(cx.value_error(2, "comment", "must be less than 65535 bytes"));
            }
            if stat_index(za, index, 0).is_none() {
                return Ok(b(false));
            }
            b(set_file_comment(za, index as u64, &comment))
        }
        "setExternalAttributesName" => {
            let za = cx.za()?;
            let name = cx.bytes(0);
            if name.is_empty() {
                return Err(cx.empty(1, "name"));
            }
            let idx = locate(za, &name, 0);
            if idx < 0 {
                return Ok(b(false));
            }
            let (opsys, attr, flags) = (cx.int(1), cx.int(2), cx.int(3));
            b(unsafe {
                sys::zip_file_set_external_attributes(
                    za,
                    idx as u64,
                    flags as u32,
                    (opsys & 0xff) as u8,
                    attr as u32,
                )
            } >= 0)
        }
        "setExternalAttributesIndex" => {
            let za = cx.za()?;
            let index = cx.int(0);
            if stat_index(za, index, 0).is_none() {
                return Ok(b(false));
            }
            let (opsys, attr, flags) = (cx.int(1), cx.int(2), cx.int(3));
            b(unsafe {
                sys::zip_file_set_external_attributes(
                    za,
                    index as u64,
                    flags as u32,
                    (opsys & 0xff) as u8,
                    attr as u32,
                )
            } >= 0)
        }
        "getExternalAttributesName" => {
            let za = cx.za()?;
            let name = cx.bytes(0);
            if name.is_empty() {
                return Err(cx.empty(1, "name"));
            }
            let idx = locate(za, &name, 0);
            if idx < 0 {
                return Ok(b(false));
            }
            external_attributes(za, idx as u64, cx.int(1))
        }
        "getExternalAttributesIndex" => {
            let za = cx.za()?;
            let index = cx.int(0);
            if stat_index(za, index, 0).is_none() {
                return Ok(b(false));
            }
            external_attributes(za, index as u64, cx.int(1))
        }
        "setEncryptionName" | "setEncryptionIndex" => set_encryption(&mut cx, &method)?,
        "getCommentName" => {
            let za = cx.za()?;
            let name = cx.bytes(0);
            if name.is_empty() {
                return Err(cx.empty(1, "name"));
            }
            // php keeps the index in an int.
            let idx = locate(za, &name, 0) as c_int;
            if idx < 0 {
                return Ok(b(false));
            }
            file_comment(za, idx as u64, cx.int(1))
        }
        "getCommentIndex" => {
            let za = cx.za()?;
            let index = cx.int(0);
            if stat_index(za, index, 0).is_none() {
                return Ok(b(false));
            }
            file_comment(za, index as u64, cx.int(1))
        }
        "setCompressionName" => {
            let za = cx.za()?;
            let name = cx.bytes(0);
            if name.is_empty() {
                return Err(cx.empty(1, "name"));
            }
            let idx = locate(za, &name, 0);
            if idx < 0 {
                return Ok(b(false));
            }
            let (method, flags) = (cx.int(1), cx.int(2));
            b(
                unsafe {
                    sys::zip_set_file_compression(za, idx as u64, method as i32, flags as u32)
                } == 0,
            )
        }
        "setCompressionIndex" => {
            let za = cx.za()?;
            let (index, method, flags) = (cx.int(0), cx.int(1), cx.int(2));
            b(unsafe {
                sys::zip_set_file_compression(za, index as u64, method as i32, flags as u32)
            } == 0)
        }
        "setMtimeName" => {
            let za = cx.za()?;
            let name = cx.bytes(0);
            if name.is_empty() {
                return Err(cx.empty(1, "name"));
            }
            let idx = locate(za, &name, 0);
            if idx < 0 {
                return Ok(b(false));
            }
            let (mtime, flags) = (cx.int(1), cx.int(2));
            b(unsafe {
                sys::zip_file_set_mtime(za, idx as u64, mtime as libc::time_t, flags as u32)
            } == 0)
        }
        "setMtimeIndex" => {
            let za = cx.za()?;
            let (index, mtime, flags) = (cx.int(0), cx.int(1), cx.int(2));
            b(unsafe {
                sys::zip_file_set_mtime(za, index as u64, mtime as libc::time_t, flags as u32)
            } == 0)
        }
        "deleteIndex" => {
            let za = cx.za()?;
            let index = cx.int(0);
            if index < 0 {
                return Ok(b(false));
            }
            b(unsafe { sys::zip_delete(za, index as u64) } >= 0)
        }
        "deleteName" => {
            let za = cx.za()?;
            if cx.bytes(0).is_empty() {
                return Ok(b(false));
            }
            let Some(sb) = stat_path(&cx, za, 0, 0)? else {
                return Ok(b(false));
            };
            b(unsafe { sys::zip_delete(za, sb.index) } == 0)
        }
        "statName" => {
            let za = cx.za()?;
            let _ = cx.path(0, "name")?;
            let flags = cx.int(1);
            match stat_path(&cx, za, 0, flags)? {
                Some(sb) => stat_array(&sb),
                None => b(false),
            }
        }
        "statIndex" => {
            let za = cx.za()?;
            match stat_index(za, cx.int(0), cx.int(1)) {
                Some(sb) => stat_array(&sb),
                None => b(false),
            }
        }
        "locateName" => {
            let name = cx.path(0, "name")?;
            let za = cx.za()?;
            if name.is_empty() {
                return Ok(b(false));
            }
            let idx = locate(za, &name, cx.int(1));
            if idx >= 0 {
                Value::Int(idx)
            } else {
                b(false)
            }
        }
        "getNameIndex" => {
            let za = cx.za()?;
            let p = unsafe { sys::zip_get_name(za, cx.int(0) as u64, cx.int(1) as u32) };
            if p.is_null() {
                b(false)
            } else {
                Value::str(unsafe { sys::cstr(p) }.to_vec())
            }
        }
        "unchangeArchive" => {
            let za = cx.za()?;
            b(unsafe { sys::zip_unchange_archive(za) } == 0)
        }
        "unchangeAll" => {
            let za = cx.za()?;
            b(unsafe { sys::zip_unchange_all(za) } == 0)
        }
        "unchangeIndex" => {
            let za = cx.za()?;
            let index = cx.int(0);
            if index < 0 {
                return Ok(b(false));
            }
            b(unsafe { sys::zip_unchange(za, index as u64) } == 0)
        }
        "unchangeName" => {
            let za = cx.za()?;
            if cx.bytes(0).is_empty() {
                return Ok(b(false));
            }
            let Some(sb) = stat_path(&cx, za, 0, 0)? else {
                return Ok(b(false));
            };
            b(unsafe { sys::zip_unchange(za, sb.index) } == 0)
        }
        "extractTo" => extract_to(&mut cx)?,
        "getFromName" => get_from(&mut cx, true)?,
        "getFromIndex" => get_from(&mut cx, false)?,
        "getStreamIndex" => get_stream(&mut cx, false, true)?,
        "getStreamName" => get_stream(&mut cx, true, true)?,
        "getStream" => get_stream(&mut cx, true, false)?,
        "registerProgressCallback" => register_progress(&mut cx)?,
        "registerCancelCallback" => register_cancel(&mut cx)?,
        "isCompressionMethodSupported" => {
            let (m, enc) = (cx.int(0), conv::to_bool(&cx.arg(1)));
            b(unsafe { sys::zip_compression_method_supported(m as i32, enc as c_int) } != 0)
        }
        "isEncryptionMethodSupported" => {
            let (m, enc) = (cx.int(0), conv::to_bool(&cx.arg(1)));
            b(unsafe { sys::zip_encryption_method_supported(m as u16, enc as c_int) } != 0)
        }
        _ => Value::Null,
    })
}

/// zip_name_locate on php's `char *` (up to a NUL).
fn locate(za: *mut zip_t, name: &[u8], flags: i64) -> i64 {
    let n = c_string(name);
    unsafe { sys::zip_name_locate(za, n.as_ptr(), flags as u32) }
}

/// PHP_ZIP_STAT_INDEX
fn stat_index(za: *mut zip_t, index: i64, flags: i64) -> Option<sys::zip_stat_t> {
    let mut sb = sys::zip_stat_t::zeroed();
    (unsafe { sys::zip_stat_index(za, index as u64, flags as u32, &mut sb) } == 0).then_some(sb)
}

/// PHP_ZIP_STAT_PATH on argument `i` (always argument 1 in php's
/// messages): must not be empty.
fn stat_path(
    cx: &Cx<'_>,
    za: *mut zip_t,
    i: usize,
    flags: i64,
) -> Result<Option<sys::zip_stat_t>, Thrown> {
    let name = cx.bytes(i);
    if name.is_empty() {
        return Err(cx.empty(1, "name"));
    }
    let n = c_string(&name);
    let mut sb = sys::zip_stat_t::zeroed();
    Ok((unsafe { sys::zip_stat(za, n.as_ptr(), flags as u32, &mut sb) } == 0).then_some(sb))
}

/// php_zip_set_file_comment
fn set_file_comment(za: *mut zip_t, index: u64, comment: &[u8]) -> bool {
    unsafe {
        let mut cur_len: u32 = 0;
        let cur = sys::zip_file_get_comment(za, index, &mut cur_len, sys::ZIP_FL_ENC_RAW);
        // Resetting an unchanged inherited comment hits a libzip
        // use-after-free.
        if !cur.is_null()
            && cur_len as usize == comment.len()
            && std::slice::from_raw_parts(cur.cast::<u8>(), cur_len as usize) == comment
        {
            return true;
        }
        if comment.is_empty() {
            return sys::zip_file_set_comment(za, index, ptr::null(), 0, 0) == 0;
        }
        sys::zip_file_set_comment(za, index, comment.as_ptr().cast(), comment.len() as u16, 0) == 0
    }
}

fn file_comment(za: *mut zip_t, index: u64, flags: i64) -> Value {
    let mut len: u32 = 0;
    let p = unsafe { sys::zip_file_get_comment(za, index, &mut len, flags as u32) };
    if p.is_null() {
        // RETURN_STRINGL(NULL, 0)
        return Value::str("");
    }
    Value::str(unsafe { std::slice::from_raw_parts(p.cast::<u8>(), len as usize) })
}

/// The `[$opsys, $attr]` the prelude assigns to its reference arguments.
fn external_attributes(za: *mut zip_t, index: u64, flags: i64) -> Value {
    let (mut opsys, mut attr) = (0u8, 0u32);
    if unsafe {
        sys::zip_file_get_external_attributes(za, index, flags as u32, &mut opsys, &mut attr)
    } < 0
    {
        return Value::Bool(false);
    }
    let mut a = PhpArray::new();
    a.push(Value::Int(opsys as i64));
    a.push(Value::Int(attr as i64));
    Value::Array(a)
}

/// The libzip call in `f`, with the object's host reachable from the
/// archive's callbacks. Returns the call's result and what a callback
/// threw, which php leaves pending while the method finishes.
fn with_callbacks<R>(cx: &mut Cx<'_>, f: impl FnOnce() -> R) -> (R, Option<Thrown>) {
    let mut out = None;
    let r = ffi::with_host(cx.host, &cx.fname, || out = Some(f()));
    (out.expect("ran"), r.err())
}

fn open(cx: &mut Cx<'_>) -> Result<Value, Thrown> {
    let filename = cx.path(0, "filename")?;
    let mut flags = cx.int(1) as c_int;
    let o = cx.obj();
    if filename.is_empty() {
        return Err(cx.empty(1, "filename"));
    }
    let Some(resolved) = expand_filepath(&filename) else {
        cx.warn("No such file or directory")?;
        return Ok(Value::Bool(false));
    };
    // What a callback threw while the old archive closed: php's
    // EG(exception), which the rest of the method runs under.
    let mut pending = None;
    if let Some(a) = o.archive() {
        if a.close.get() {
            return Err(Thrown::error("Error", "Already being closed"));
        }
        let za = a.za();
        a.close.set(true);
        let (err, thrown) = with_callbacks(cx, || a.zip_close(za));
        a.close.set(false);
        if err != 0 {
            cx.warn_pending("Empty string as source", thrown.is_some())?;
            return match thrown {
                Some(t) => Err(t),
                None => Ok(Value::Bool(false)),
            };
        }
        // php_zip_object_detach_archive
        a.za.set(ptr::null_mut());
        o.archive.borrow_mut().take();
        release(cx.host, Some(&cx.fname), a)?;
        pending = thrown;
    }
    *o.filename.borrow_mut() = None;
    if flags & (sys::ZIP_TRUNCATE | sys::ZIP_RDONLY) == 0 {
        if let Ok(m) = std::fs::metadata(OsStr::from_bytes(&resolved)) {
            if m.len() == 0 {
                let msg = "Using empty file as ZipArchive is deprecated";
                cx.docref_pending(crate::E_DEPRECATED, msg, pending.is_some())?;
                // libzip 1.6.0 stopped accepting empty files.
                flags |= sys::ZIP_TRUNCATE;
            }
        }
    }
    let path = CString::new(resolved.clone()).expect("no NUL");
    let mut err: c_int = 0;
    let za = unsafe { sys::zip_open(path.as_ptr(), flags, &mut err) };
    if za.is_null() || err != 0 {
        return match pending {
            Some(t) => Err(t),
            None => Ok(Value::Int(err as i64)),
        };
    }
    *o.archive.borrow_mut() = Some(Archive::new(za, resolved.clone()));
    *o.filename.borrow_mut() = Some(resolved);
    match pending {
        Some(t) => Err(t),
        None => Ok(Value::Bool(true)),
    }
}

fn set_password(cx: &mut Cx<'_>) -> Result<Value, Thrown> {
    let za = cx.za()?;
    let pw = cx.bytes(0);
    if pw.is_empty() {
        return Ok(Value::Bool(false));
    }
    let p = c_string(&pw);
    Ok(Value::Bool(
        unsafe { sys::zip_set_default_password(za, p.as_ptr()) } == 0,
    ))
}

fn close(cx: &mut Cx<'_>) -> Result<Value, Thrown> {
    let za = cx.za()?;
    let o = cx.obj();
    let a = o.archive().expect("open archive");
    if a.close.get() {
        return Err(Thrown::error("Error", "Already being closed"));
    }
    a.close.set(true);
    let (err, thrown) = with_callbacks(cx, || a.zip_close(za));
    a.close.set(false);
    if err != 0 {
        let msg = String::from_utf8_lossy(unsafe { sys::cstr(sys::zip_strerror(za)) }).into_owned();
        let warned = cx.warn_pending(&msg, thrown.is_some());
        unsafe {
            let e = sys::zip_get_error(za);
            o.err_zip.set(sys::zip_error_code_zip(e));
            o.err_sys.set(sys::zip_error_code_system(e));
            sys::zip_error_fini(e);
            sys::zip_discard(za);
        }
        if let Err(t) = warned {
            detach(cx, &o, a)?;
            return Err(t);
        }
    } else {
        o.err_zip.set(0);
        o.err_sys.set(0);
    }
    *o.filename.borrow_mut() = None;
    detach(cx, &o, a)?;
    if let Some(t) = thrown {
        return Err(t);
    }
    Ok(Value::Bool(err == 0))
}

/// php_zip_object_detach_archive after the archive was closed or discarded.
fn detach(cx: &mut Cx<'_>, o: &ZipObject, a: Rc<Archive>) -> Result<(), Thrown> {
    a.za.set(ptr::null_mut());
    o.archive.borrow_mut().take();
    release(cx.host, Some(&cx.fname), a)
}

fn get_status_string(cx: &mut Cx<'_>) -> Value {
    let o = cx.obj();
    let za = o.za();
    let s = unsafe {
        if za.is_null() {
            let mut e = sys::zip_error_t {
                zip_err: 0,
                sys_err: 0,
                str_: ptr::null_mut(),
            };
            sys::zip_error_init(&mut e);
            sys::zip_error_set(&mut e, o.err_zip.get(), o.err_sys.get());
            let s = sys::cstr(sys::zip_error_strerror(&mut e)).to_vec();
            sys::zip_error_fini(&mut e);
            s
        } else {
            let e = sys::zip_get_error(za);
            let s = sys::cstr(sys::zip_error_strerror(e)).to_vec();
            sys::zip_error_fini(e);
            s
        }
    };
    Value::str(s)
}

fn add_empty_dir(cx: &mut Cx<'_>) -> Result<Value, Thrown> {
    let za = cx.za()?;
    let mut dir = cx.bytes(0);
    let flags = cx.int(1);
    if dir.is_empty() {
        return Ok(Value::Bool(false));
    }
    if dir.last() != Some(&b'/') {
        // php appends to the C string, so a name with a NUL loses the rest.
        let end = dir.iter().position(|&c| c == 0).unwrap_or(dir.len());
        dir.truncate(end);
        dir.push(b'/');
    }
    let o = cx.obj();
    let d = c_string(&dir);
    let id = unsafe { sys::zip_dir_add(za, d.as_ptr(), flags as u32) };
    o.last_id.set(id);
    if id == -1 {
        return Ok(Value::Bool(false));
    }
    unsafe { sys::zip_error_clear(za) };
    Ok(Value::Bool(true))
}

fn add_from_string(cx: &mut Cx<'_>) -> Result<Value, Thrown> {
    let za = cx.za()?;
    let name = cx.bytes(0);
    let content = cx.bytes(1);
    let flags = cx.int(2);
    let o = cx.obj();
    let a = o.archive().expect("open archive");
    // libzip reads the buffer until the archive closes.
    let buf: Box<[u8]> = content.into_boxed_slice();
    let (data, len) = (buf.as_ptr(), buf.len());
    a.buffers.borrow_mut().push(buf);
    let zs = unsafe { sys::zip_source_buffer(za, data.cast::<c_void>(), len as u64, 0) };
    if zs.is_null() {
        return Ok(Value::Bool(false));
    }
    let n = c_string(&name);
    let id = unsafe { sys::zip_file_add(za, n.as_ptr(), zs, flags as u32) };
    o.last_id.set(id);
    if id == -1 {
        unsafe { sys::zip_source_free(zs) };
        return Ok(Value::Bool(false));
    }
    unsafe { sys::zip_error_clear(za) };
    Ok(Value::Bool(true))
}

/// php_zip_add_file: add `filename` as `entry` (or replace entry
/// `replace`). -1 on failure.
/// What php_zip_add_file adds: a file's range, as a new entry or in place
/// of entry `replace` (when not negative).
struct FileSource<'a> {
    filename: &'a [u8],
    entry: &'a [u8],
    start: i64,
    len: i64,
    replace: i64,
    flags: i64,
}

fn add_file(cx: &mut Cx<'_>, o: &ZipObject, src: FileSource<'_>) -> Result<i32, Thrown> {
    let FileSource {
        filename,
        entry,
        start,
        len,
        replace,
        flags,
    } = src;
    let za = o.za();
    let Some(resolved) = expand_filepath(filename) else {
        cx.warn("No such file or directory")?;
        return Ok(-1);
    };
    if crate::streams::sys_stat(&resolved, false).is_none() {
        cx.warn("No such file or directory")?;
        return Ok(-1);
    }
    let path = CString::new(resolved).expect("no NUL");
    let mut flags = flags as u32;
    let open_now = unsafe { sys::rhp_zip_fl_open_file_now() };
    let zs = if flags & open_now != 0 {
        let fd = unsafe { libc::fopen(path.as_ptr(), c"rb".as_ptr()) };
        if fd.is_null() {
            return Ok(-1);
        }
        flags ^= open_now;
        let zs = unsafe { sys::zip_source_filep(za, fd, start as u64, len) };
        if zs.is_null() {
            unsafe { libc::fclose(fd) };
            return Ok(-1);
        }
        zs
    } else {
        let zs = unsafe { sys::zip_source_file(za, path.as_ptr(), start as u64, len) };
        if zs.is_null() {
            return Ok(-1);
        }
        zs
    };
    if replace >= 0 {
        if unsafe { sys::zip_file_replace(za, replace as u64, zs, flags) } < 0 {
            unsafe { sys::zip_source_free(zs) };
            return Ok(-1);
        }
        unsafe { sys::zip_error_clear(za) };
        return Ok(1);
    }
    let n = c_string(entry);
    let id = unsafe { sys::zip_file_add(za, n.as_ptr(), zs, flags) };
    o.last_id.set(id);
    if id < 0 {
        unsafe { sys::zip_source_free(zs) };
        return Ok(-1);
    }
    unsafe { sys::zip_error_clear(za) };
    Ok(1)
}

fn add_file_method(cx: &mut Cx<'_>) -> Result<Value, Thrown> {
    let filename = cx.path(0, "filepath")?;
    if filename.is_empty() {
        return Err(cx.empty(1, "filepath"));
    }
    let mut entry = cx.bytes(1);
    if entry.is_empty() {
        entry = filename.clone();
    }
    let (start, len, flags) = (cx.int(2), cx.int(3), cx.int(4));
    // php has no ZIP_FROM_OBJECT here: on a closed object the path checks
    // still run, then libzip's NULL checks fail.
    let o = cx.obj();
    Ok(Value::Bool(
        add_file(
            cx,
            &o,
            FileSource {
                filename: &filename,
                entry: &entry,
                start,
                len,
                replace: -1,
                flags,
            },
        )? >= 0,
    ))
}

fn replace_file(cx: &mut Cx<'_>) -> Result<Value, Thrown> {
    let filename = cx.path(0, "filepath")?;
    let index = cx.int(1);
    if filename.is_empty() {
        return Err(cx.empty(1, "filepath"));
    }
    if index < 0 {
        return Err(cx.value_error(2, "index", "must be greater than or equal to 0"));
    }
    let (start, len, flags) = (cx.int(2), cx.int(3), cx.int(4));
    let o = cx.obj();
    Ok(Value::Bool(
        add_file(
            cx,
            &o,
            FileSource {
                filename: &filename,
                entry: b"",
                start,
                len,
                replace: index,
                flags,
            },
        )? >= 0,
    ))
}

/// zip_options
#[derive(Default)]
struct Options {
    remove_all_path: i64,
    remove_path: Option<Vec<u8>>,
    add_path: Option<Vec<u8>>,
    flags: u32,
    comp_method: i64,
    comp_flags: i64,
    enc_method: i64,
    enc_password: Option<Vec<u8>>,
}

/// php_zip_parse_options; `Err` for php's exceptions.
fn parse_options(cx: &mut Cx<'_>, options: &PhpArray, o: &mut Options) -> Result<(), Thrown> {
    let get = |k: &str| options.get(&ArrayKey::from_bytes(k.as_bytes())).map(deref);
    let tname = |cx: &Cx<'_>, v: &Value| crate::zend::zpp::zval_value_name(cx.host, v);
    let warn_type = |cx: &mut Cx<'_>, opt: &str, want: &str, v: &Value| {
        let t = tname(cx, v);
        cx.warn(&format!(
            "Option \"{opt}\" must be of type {want}, {t} given"
        ))
    };
    if let Some(v) = get("remove_all_path") {
        if !matches!(v, Value::Bool(_)) {
            warn_type(cx, "remove_all_path", "bool", &v)?;
        }
        o.remove_all_path = conv::to_int(&v);
    }
    if let Some(v) = get("comp_method") {
        if !matches!(v, Value::Int(_)) {
            warn_type(cx, "comp_method", "int", &v)?;
        }
        o.comp_method = conv::to_int(&v);
        if let Some(v) = get("comp_flags") {
            if !matches!(v, Value::Int(_)) {
                warn_type(cx, "comp_flags", "int", &v)?;
            }
            o.comp_flags = conv::to_int(&v);
        }
    }
    if let Some(v) = get("enc_method") {
        if !matches!(v, Value::Int(_)) {
            warn_type(cx, "enc_method", "int", &v)?;
        }
        o.enc_method = conv::to_int(&v);
        if let Some(v) = get("enc_password") {
            let Value::Str(s) = &v else {
                return Err(Thrown::type_error(format!(
                    "Option \"enc_password\" must be of type string, {} given",
                    tname(cx, &v)
                )));
            };
            o.enc_password = Some(s.as_bytes().to_vec());
        }
    }
    for (key, slot) in [("remove_path", 0), ("add_path", 1)] {
        if let Some(v) = get(key) {
            let Value::Str(s) = &v else {
                return Err(Thrown::type_error(format!(
                    "Option \"{key}\" must be of type string, {} given",
                    tname(cx, &v)
                )));
            };
            let s = s.as_bytes();
            if s.is_empty() {
                return Err(Thrown::error(
                    "ValueError",
                    format!("Option \"{key}\" must not be empty"),
                ));
            }
            if s.len() >= MAXPATHLEN {
                return Err(Thrown::error(
                    "ValueError",
                    format!(
                        "Option \"{key}\" must be less than {} bytes",
                        MAXPATHLEN - 1
                    ),
                ));
            }
            if slot == 0 {
                o.remove_path = Some(s.to_vec());
            } else {
                o.add_path = Some(s.to_vec());
            }
        }
    }
    if let Some(v) = get("flags") {
        let Value::Int(n) = v else {
            return Err(Thrown::type_error(format!(
                "Option \"flags\" must be of type int, {} given",
                tname(cx, &v)
            )));
        };
        o.flags = n as u32;
    }
    Ok(())
}

/// A pattern's matches (`Ok(None)` for none, `Err` for php's -1).
type Found = Result<Option<Vec<Vec<u8>>>, ()>;

/// php_zip_glob, without the directories `GLOB_ONLYDIR` drops.
fn zip_glob(cx: &mut Cx<'_>, pattern: &[u8], flags: i64) -> Result<Found, Thrown> {
    if pattern.len() >= MAXPATHLEN {
        cx.warn(&format!(
            "Pattern exceeds the maximum allowed length of {MAXPATHLEN} characters"
        ))?;
        return Ok(Err(()));
    }
    if (crate::fs_fns::GLOB_AVAILABLE & flags) != flags {
        cx.warn("At least one of the passed flags is invalid or not supported on this platform")?;
        return Ok(Err(()));
    }
    let (ret, paths) = crate::fs_fns::php_glob(pattern, flags);
    if ret != 0 || paths.is_empty() {
        return Ok(Ok(None));
    }
    let mut out = Vec::new();
    for p in paths {
        if flags & crate::fs_fns::GLOB_ONLYDIR != 0 {
            match crate::streams::sys_stat(&p, false) {
                Some(s) if s.is_dir => {}
                _ => continue,
            }
        }
        out.push(p);
    }
    Ok(Ok(Some(out)))
}

/// php_zip_pcre: the regular files in `path` whose name matches.
fn zip_pcre(cx: &mut Cx<'_>, regexp: &[u8], path: &[u8]) -> Result<Found, Thrown> {
    let Some(mut names) = crate::streams::sys_readdir(path) else {
        let reason = crate::streams::last_strerror();
        crate::warn_arg(
            cx.host,
            &cx.fname,
            path,
            &format!("Failed to open directory: {reason}"),
        )?;
        return Ok(Err(()));
    };
    if names.is_empty() {
        return Ok(Ok(None));
    }
    names.sort();
    let Some(re) = crate::preg_fns::compile_warn(cx.host, &cx.fname, regexp)? else {
        cx.warn("Invalid expression")?;
        return Ok(Err(()));
    };
    let mut out = Vec::new();
    for name in &names {
        if name == b"." || name == b".." {
            continue;
        }
        if path.len() + name.len() + 1 >= MAXPATHLEN {
            cx.warn(&format!(
                "add_path string too long (max: {}, {} given)",
                MAXPATHLEN - 1,
                path.len() + name.len() + 1
            ))?;
            break;
        }
        if !matches!(re.find_at(name, 0), Ok(Some(_))) {
            continue;
        }
        let mut full = path.to_vec();
        full.push(b'/');
        full.extend_from_slice(name);
        match crate::streams::sys_stat(&full, false) {
            None => {
                cx.warn(&format!("Cannot read <{}>", String::from_utf8_lossy(&full)))?;
                continue;
            }
            Some(s) if s.is_dir => continue,
            Some(_) => {}
        }
        out.push(full);
    }
    Ok(Ok(Some(out)))
}

/// php_zip_add_from_pattern
fn add_from_pattern(cx: &mut Cx<'_>, glob: bool) -> Result<Value, Thrown> {
    let pattern = cx.path(0, "pattern")?;
    let (glob_flags, path) = if glob {
        (cx.int(1), b".".to_vec())
    } else {
        (0, cx.bytes(1))
    };
    let options = match cx.arg(2) {
        Value::Array(a) => a,
        _ => PhpArray::new(),
    };
    if pattern.is_empty() {
        return Err(cx.empty(1, "pattern"));
    }
    let mut opts = Options {
        comp_method: -1,
        enc_method: -1,
        flags: 8192,
        ..Default::default()
    };
    if !options.is_empty() {
        parse_options(cx, &options, &mut opts)?;
    }
    let found = if glob {
        zip_glob(cx, &pattern, glob_flags)?
    } else {
        zip_pcre(cx, &pattern, &path)?
    };
    let files = match found {
        Err(()) => return Ok(Value::Bool(false)),
        Ok(None) => return Ok(Value::Array(PhpArray::new())),
        Ok(Some(f)) => f,
    };
    let o = cx.obj();
    let za = o.za();
    let mut result = PhpArray::new();
    for f in &files {
        result.push(Value::str(f.clone()));
    }
    for file in &files {
        let stripped: Vec<u8> = if opts.remove_all_path != 0 {
            basename(file)
        } else if let Some(rp) = opts
            .remove_path
            .as_ref()
            .filter(|rp| file.len() > rp.len() && file.starts_with(rp))
        {
            if file[rp.len()] == b'/' {
                file[rp.len() + 1..].to_vec()
            } else {
                file[rp.len()..].to_vec()
            }
        } else {
            file.clone()
        };
        let mut entry = Vec::new();
        if let Some(ap) = &opts.add_path {
            if ap.len() + stripped.len() > MAXPATHLEN {
                cx.warn(&format!(
                    "Entry name too long (max: {}, {} given)",
                    MAXPATHLEN - 1,
                    ap.len() + stripped.len()
                ))?;
                return Ok(Value::Bool(false));
            }
            entry.extend_from_slice(ap);
        }
        entry.extend_from_slice(&stripped);
        // snprintf into MAXPATHLEN bytes, as a C string.
        entry.truncate(MAXPATHLEN - 1);
        if let Some(n) = entry.iter().position(|&c| c == 0) {
            entry.truncate(n);
        }
        let src = FileSource {
            filename: file,
            entry: &entry,
            start: 0,
            len: 0,
            replace: -1,
            flags: opts.flags as i64,
        };
        if add_file(cx, &o, src)? < 0 {
            return Ok(Value::Bool(false));
        }
        let last = o.last_id.get() as u64;
        if opts.comp_method >= 0
            && unsafe {
                sys::zip_set_file_compression(
                    za,
                    last,
                    opts.comp_method as i32,
                    opts.comp_flags as u32,
                )
            } != 0
        {
            return Ok(Value::Bool(false));
        }
        if opts.enc_method >= 0 {
            if unsafe { sys::zip_file_set_encryption(za, last, sys::ZIP_EM_NONE, ptr::null()) } < 0
            {
                cx.warn("password reset failed")?;
                return Ok(Value::Bool(false));
            }
            let pw = opts.enc_password.as_deref().map(c_string);
            let pwp = pw.as_ref().map_or(ptr::null(), |p| p.as_ptr());
            if unsafe { sys::zip_file_set_encryption(za, last, opts.enc_method as u16, pwp) } != 0 {
                return Ok(Value::Bool(false));
            }
        }
    }
    Ok(Value::Array(result))
}

fn set_encryption(cx: &mut Cx<'_>, method: &str) -> Result<Value, Thrown> {
    let za = cx.za()?;
    let idx = if method == "setEncryptionName" {
        let name = cx.bytes(0);
        if name.is_empty() {
            return Err(cx.empty(1, "name"));
        }
        let idx = locate(za, &name, 0);
        if idx < 0 {
            return Ok(Value::Bool(false));
        }
        idx
    } else {
        cx.int(0)
    };
    let enc = cx.int(1);
    let pw = match cx.arg(2) {
        Value::Null => None,
        v => Some(c_string(&conv::to_string_bytes(&v))),
    };
    if unsafe { sys::zip_file_set_encryption(za, idx as u64, sys::ZIP_EM_NONE, ptr::null()) } < 0 {
        cx.warn("password reset failed")?;
        return Ok(Value::Bool(false));
    }
    let pwp = pw.as_ref().map_or(ptr::null(), |p| p.as_ptr());
    Ok(Value::Bool(
        unsafe { sys::zip_file_set_encryption(za, idx as u64, enc as u16, pwp) } == 0,
    ))
}

/// php_zip_extract_file: `file` (entry `idx`, or by name when negative)
/// below `dest`.
fn extract_file(
    cx: &mut Cx<'_>,
    za: *mut zip_t,
    dest: &[u8],
    file: &[u8],
    idx: i64,
) -> Result<bool, Thrown> {
    let idx = if idx < 0 {
        let i = locate(za, file, 0);
        if i < 0 {
            return Ok(false);
        }
        i
    } else {
        idx
    };
    // virtual_file_ex(CWD_EXPAND) from an empty cwd, then
    // php_zip_make_relative_path.
    let file_c = &file[..file.iter().position(|&c| c == 0).unwrap_or(file.len())];
    // virtual_file_ex refuses an empty path, leaving the state empty.
    if file_c.is_empty() {
        return Ok(false);
    }
    let expanded = cwd_expand(file_c);
    let Some(cleaned) = make_relative_path(&expanded) else {
        return Ok(false);
    };
    let Some(sb) = stat_index(za, idx, 0) else {
        return Ok(false);
    };
    if cleaned.len() >= MAXPATHLEN {
        return Ok(false);
    }
    let is_dir_only = cleaned.len() > 1 && cleaned.last() == Some(&b'/');
    let dirname_full = if is_dir_only {
        [dest, b"/", &cleaned].concat()
    } else {
        let dir = zend_dirname(&cleaned);
        if dir.is_empty() || dir == b"." {
            dest.to_vec()
        } else {
            [dest, b"/", &dir].concat()
        }
    };
    if crate::streams::sys_stat(&dirname_full, false).is_none()
        && !mkdir_recursive(cx, &dirname_full, true)?
    {
        return Ok(false);
    }
    if is_dir_only {
        return Ok(true);
    }
    let base = basename(&cleaned);
    let full = [&dirname_full[..], b"/", &base].concat();
    if full.len() > MAXPATHLEN {
        cx.warn(&format!(
            "Full extraction path exceed MAXPATHLEN ({MAXPATHLEN})"
        ))?;
        return Ok(false);
    }
    let zf = unsafe { sys::zip_fopen_index(za, idx as u64, 0) };
    if zf.is_null() {
        return Ok(false);
    }
    let Some(stream) = cx.host.stream_open(&full, b"w+b") else {
        crate::open_failed(cx.host, &cx.fname, &full)?;
        unsafe { sys::zip_fclose(zf) };
        return Ok(false);
    };
    let mut buf = vec![0u8; 8192];
    let mut n: i64;
    loop {
        n = unsafe { sys::zip_fread(zf, buf.as_mut_ptr().cast(), buf.len() as u64) };
        if n <= 0 {
            break;
        }
        if cx.host.stream_write(&stream, &buf[..n as usize]) != Some(n as usize) {
            n = -1;
            break;
        }
    }
    if n < 0 {
        let (code, msg) = file_error(zf);
        if code != sys::ZIP_ER_OK {
            cx.warn(&format!(
                "Cannot extract \"{}\": \"{msg}\"",
                String::from_utf8_lossy(file_c)
            ))?;
        }
        cx.host.stream_close(&stream);
        unsafe { sys::zip_fclose(zf) };
        return Ok(false);
    }
    cx.host.stream_close(&stream);
    // The plain-files wrapper's metadata(TOUCH).
    if let Some(c) = crate::zend::virtual_cwd::c_path(&full) {
        let t = libc::utimbuf {
            actime: sb.mtime,
            modtime: sb.mtime,
        };
        unsafe { libc::utime(c.as_ptr(), &t) };
    }
    Ok(unsafe { sys::zip_fclose(zf) } == 0)
}

/// php_basename without a suffix (the C locale's bytes).
fn basename(path: &[u8]) -> Vec<u8> {
    let end = path.iter().rposition(|&b| b != b'/').map_or(0, |i| i + 1);
    let start = path[..end]
        .iter()
        .rposition(|&b| b == b'/')
        .map_or(0, |i| i + 1);
    path[start..end].to_vec()
}

/// zend_dirname
fn zend_dirname(path: &[u8]) -> Vec<u8> {
    let mut end = path.len();
    while end > 1 && path[end - 1] == b'/' {
        end -= 1;
    }
    match path[..end].iter().rposition(|&b| b == b'/') {
        None => b".".to_vec(),
        Some(0) => b"/".to_vec(),
        Some(i) => {
            let mut j = i;
            while j > 1 && path[j - 1] == b'/' {
                j -= 1;
            }
            path[..j].to_vec()
        }
    }
}

/// php_stream_mkdir(path, 0777, PHP_STREAM_MKDIR_RECURSIVE) on the
/// plain-files wrapper: each missing directory, warning with the error
/// when `report`.
fn mkdir_recursive(cx: &mut Cx<'_>, path: &[u8], report: bool) -> Result<bool, Thrown> {
    let Some(full) = expand_filepath(path) else {
        cx.warn("Invalid path")?;
        return Ok(false);
    };
    let mut prefixes: Vec<&[u8]> = Vec::new();
    for (i, &b) in full.iter().enumerate() {
        if b == b'/' && i > 0 && full[i - 1] != b'/' {
            prefixes.push(&full[..i]);
        }
    }
    prefixes.push(&full);
    let first_missing = prefixes
        .iter()
        .rposition(|p| crate::streams::sys_stat(p, false).is_some())
        .map_or(0, |i| i + 1);
    for p in &prefixes[first_missing..] {
        if p.contains(&0) {
            return Ok(false);
        }
        if let Err(e) = crate::zend::virtual_cwd::mkdir(p, 0o777) {
            let e = e.raw_os_error().unwrap_or(0);
            if e != libc::EEXIST {
                if report {
                    cx.warn(&crate::streams::strerror(e))?;
                }
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// virtual_file_ex(CWD_EXPAND) from an empty cwd: [`crate::zend::virtual_cwd::normalize`],
/// keeping a trailing slash.
fn cwd_expand(path: &[u8]) -> Vec<u8> {
    let mut out = crate::zend::virtual_cwd::normalize(path);
    if path.ends_with(b"/") && out != b"/" {
        out.push(b'/');
    }
    out
}

/// php_zip_make_relative_path
fn make_relative_path(path: &[u8]) -> Option<Vec<u8>> {
    if path.is_empty() {
        return None;
    }
    if path[0] == b'/' {
        return Some(path[1..].to_vec());
    }
    let mut i = path.len();
    loop {
        while i > 0 && path.get(i) != Some(&b'/') {
            i -= 1;
        }
        if i == 0 {
            return Some(path.to_vec());
        }
        if i >= 2 && path[i - 1] == b'.' {
            return Some(path[i + 1..].to_vec());
        }
        i -= 1;
    }
}

fn extract_to(cx: &mut Cx<'_>) -> Result<Value, Thrown> {
    let pathto = cx.path(0, "pathto")?;
    let files = cx.arg(1);
    match &files {
        Value::Null | Value::Array(_) | Value::Str(_) => {}
        v => {
            return Err(Thrown::type_error(format!(
                "{}(): Argument #2 ($files) must be of type array|string|null, {} given",
                cx.fname,
                crate::zend::zpp::zval_value_name(cx.host, v)
            )))
        }
    }
    let za = cx.za()?;
    if pathto.is_empty() {
        return Ok(Value::Bool(false));
    }
    if crate::streams::sys_stat(&pathto, false).is_none() && !mkdir_recursive(cx, &pathto, false)? {
        return Ok(Value::Bool(false));
    }
    match files {
        Value::Str(s) => {
            if !extract_file(cx, za, &pathto, s.as_bytes(), -1)? {
                return Ok(Value::Bool(false));
            }
        }
        Value::Array(a) => {
            if a.is_empty() {
                return Ok(Value::Bool(false));
            }
            for (_, v) in a.iter() {
                if let Value::Str(s) = deref(v) {
                    if !extract_file(cx, za, &pathto, s.as_bytes(), -1)? {
                        return Ok(Value::Bool(false));
                    }
                }
            }
        }
        _ => {
            let count = unsafe { sys::zip_get_num_entries(za, 0) };
            if count == -1 {
                cx.warn("Illegal archive")?;
                return Ok(Value::Bool(false));
            }
            for i in 0..count {
                let p = unsafe { sys::zip_get_name(za, i as u64, sys::ZIP_FL_UNCHANGED) };
                if p.is_null() {
                    return Ok(Value::Bool(false));
                }
                let name = unsafe { sys::cstr(p) }.to_vec();
                if !extract_file(cx, za, &pathto, &name, i)? {
                    return Ok(Value::Bool(false));
                }
            }
        }
    }
    Ok(Value::Bool(true))
}

/// php_zip_get_from
fn get_from(cx: &mut Cx<'_>, by_name: bool) -> Result<Value, Thrown> {
    let mut len = cx.int(1);
    let flags = cx.int(2);
    let (za, sb, name) = if by_name {
        let name = cx.path(0, "name")?;
        let za = cx.za()?;
        let Some(sb) = stat_path(cx, za, 0, flags)? else {
            return Ok(Value::Bool(false));
        };
        (za, sb, Some(name))
    } else {
        let za = cx.za()?;
        let Some(sb) = stat_index(za, cx.int(0), flags) else {
            return Ok(Value::Bool(false));
        };
        (za, sb, None)
    };
    if len < 1 {
        len = sb.size as i64;
    }
    let zf = match &name {
        None => unsafe { sys::zip_fopen_index(za, cx.int(0) as u64, flags as u32) },
        Some(n) => {
            let n = c_string(n);
            unsafe { sys::zip_fopen(za, n.as_ptr(), flags as u32) }
        }
    };
    if zf.is_null() {
        return Ok(Value::Bool(false));
    }
    let mut buf = vec![0u8; len.max(0) as usize];
    // zip_fread() may return short reads; a truncated entry must not pass
    // for a complete one.
    let mut n: i64 = 0;
    while (n as usize) < buf.len() {
        let rd = unsafe {
            sys::zip_fread(
                zf,
                buf.as_mut_ptr().add(n as usize).cast(),
                (buf.len() - n as usize) as u64,
            )
        };
        if rd < 0 {
            n = -1;
            break;
        }
        if rd == 0 {
            break;
        }
        n += rd;
    }
    if n >= 0 && n as u64 == sb.size {
        // Read past the end so libzip checks the CRC.
        let mut tmp = 0u8;
        if unsafe { sys::zip_fread(zf, (&mut tmp as *mut u8).cast(), 1) } < 0 {
            n = -1;
        }
    }
    if n < 0 {
        let (_, msg) = file_error(zf);
        unsafe { sys::zip_fclose(zf) };
        cx.warn(&format!("Cannot read entry: {msg}"))?;
        return Ok(Value::Bool(false));
    }
    unsafe { sys::zip_fclose(zf) };
    buf.truncate(n as usize);
    Ok(Value::str(buf))
}

/// php_zip_get_stream
fn get_stream(cx: &mut Cx<'_>, by_name: bool, accept_flags: bool) -> Result<Value, Thrown> {
    let flags = if accept_flags { cx.int(1) } else { 0 };
    if by_name {
        cx.path(0, "name")?;
    }
    let za = cx.za()?;
    let sb = if by_name {
        stat_path(cx, za, 0, flags)?
    } else {
        stat_index(za, cx.int(0), flags)
    };
    let Some(sb) = sb else {
        return Ok(Value::Bool(false));
    };
    let o = cx.obj();
    let Some(a) = o.archive() else {
        return Ok(Value::Bool(false));
    };
    match super::stream::open_entry(a, &sb, b"rb", flags as u32) {
        Some(s) => Ok(cx.host.stream_from_ops(s).unwrap_or(Value::Bool(false))),
        None => Ok(Value::Bool(false)),
    }
}

fn register_progress(cx: &mut Cx<'_>) -> Result<Value, Thrown> {
    let rate = conv::to_float(&cx.arg(0));
    let cb = cx.arg(1);
    crate::require_callback(cx.host, &cx.fname, 2, &cb)?;
    let za = cx.za()?;
    let a = cx.obj().archive().expect("open archive");
    let ud = Rc::as_ptr(&a) as *mut c_void;
    if unsafe {
        sys::zip_register_progress_callback_with_state(
            za,
            rate,
            Some(super::progress_callback),
            Some(super::progress_free),
            ud,
        )
    } != 0
    {
        return Ok(Value::Bool(false));
    }
    *a.progress.borrow_mut() = Some(cb);
    Ok(Value::Bool(true))
}

fn register_cancel(cx: &mut Cx<'_>) -> Result<Value, Thrown> {
    let cb = cx.arg(0);
    crate::require_callback(cx.host, &cx.fname, 1, &cb)?;
    let za = cx.za()?;
    let a = cx.obj().archive().expect("open archive");
    let ud = Rc::as_ptr(&a) as *mut c_void;
    if unsafe {
        sys::zip_register_cancel_callback_with_state(
            za,
            Some(super::cancel_callback),
            Some(super::cancel_free),
            ud,
        )
    } != 0
    {
        return Ok(Value::Bool(false));
    }
    *a.cancel.borrow_mut() = Some(cb);
    Ok(Value::Bool(true))
}

#[cfg(test)]
mod tests;
