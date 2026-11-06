//! The deprecated procedural API: `zip_open` and friends, on "Zip
//! Directory" and "Zip Entry" resources.

use std::cell::Cell;
use std::ffi::{c_int, CString};
use std::ptr;
use std::rc::{Rc, Weak};

use rhp_values::{ResourceData, Value};

use super::sys::{self, zip_file_t, zip_t};
use super::{check_path, deref, expand_filepath, must_not_be_empty};
use crate::{Host, Thrown};

const DIR: &str = "Zip Directory";
const ENTRY: &str = "Zip Entry";

/// `zip_rsrc`
struct ZipDir {
    za: Cell<*mut zip_t>,
    index_current: Cell<u64>,
    num_files: i64,
}

impl Drop for ZipDir {
    /// php_zip_free_dir (its warning on a failed close needs a host).
    fn drop(&mut self) {
        let za = self.za.replace(ptr::null_mut());
        if !za.is_null() {
            unsafe { sys::zip_close(za) };
        }
    }
}

/// `zip_read_rsrc`
struct ZipEntry {
    zf: Cell<*mut zip_file_t>,
    name: Vec<u8>,
    size: u64,
    comp_size: u64,
    comp_method: u16,
    /// The directory resource, to check it still exists.
    dir: Weak<ResourceData>,
}

impl Drop for ZipEntry {
    /// php_zip_free_entry
    fn drop(&mut self) {
        let zf = self.zf.replace(ptr::null_mut());
        if !zf.is_null() {
            unsafe { sys::zip_fclose(zf) };
        }
    }
}

/// zpp `r`.
fn resource_arg(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<(), Thrown> {
    let v = args.get(i).map(deref).unwrap_or(Value::Null);
    if matches!(v, Value::Resource(_)) {
        return Ok(());
    }
    Err(Thrown::type_error(format!(
        "{fname}(): Argument #{} (${pname}) must be of type resource, {} given",
        i + 1,
        crate::zend::zpp::zval_value_name(host, &v)
    )))
}

/// zpp `r` plus zend_fetch_resource of type `ty`.
fn fetch<'h, T: 'static>(
    host: &'h mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
    ty: &str,
) -> Result<&'h mut T, Thrown> {
    resource_arg(host, args, i, fname, pname)?;
    let v = args.get(i).map(deref).unwrap_or(Value::Null);
    let Value::Resource(r) = &v else {
        unreachable!("checked");
    };
    let bad = || {
        Thrown::type_error(format!(
            "{fname}(): supplied resource is not a valid {ty} resource"
        ))
    };
    if r.type_name != ty {
        return Err(bad());
    }
    host.resource_get(&v)
        .and_then(|d| d.downcast_mut::<T>())
        .ok_or_else(bad)
}

fn zip_open(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "zip_open";
    let filename = crate::zend::zpp::str_arg(host, args, 0, fname, "filename")?;
    check_path(fname, 1, "filename", &filename)?;
    if filename.is_empty() {
        return Err(must_not_be_empty(fname, 1, "filename"));
    }
    if !crate::main::fopen_wrappers::check(host, fname, &filename, true)? {
        return Ok(Value::Bool(false));
    }
    let Some(resolved) = expand_filepath(&filename) else {
        crate::warn(host, fname, "No such file or directory")?;
        return Ok(Value::Bool(false));
    };
    let path = CString::new(resolved).expect("no NUL");
    let mut err: c_int = 0;
    let za = unsafe { sys::zip_open(path.as_ptr(), 0, &mut err) };
    if za.is_null() {
        return Ok(Value::Int(err as i64));
    }
    let num_files = unsafe { sys::zip_get_num_entries(za, 0) };
    Ok(host.resource_insert(
        DIR,
        Box::new(ZipDir {
            za: Cell::new(za),
            index_current: Cell::new(0),
            num_files,
        }),
    ))
}

fn zip_close(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "zip_close";
    fetch::<ZipDir>(host, args, 0, fname, "zip", DIR)?;
    // zend_list_close: the resource stays, closed.
    let v = deref(&args[0]);
    drop(host.resource_remove(&v));
    Ok(Value::Null)
}

fn zip_read(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "zip_read";
    let d = fetch::<ZipDir>(host, args, 0, fname, "zip", DIR)?;
    let za = d.za.get();
    if za.is_null() {
        return Ok(Value::Bool(false));
    }
    let i = d.index_current.get();
    if i as i64 >= d.num_files {
        return Ok(Value::Bool(false));
    }
    let mut sb = sys::zip_stat_t::zeroed();
    if unsafe { sys::zip_stat_index(za, i, 0, &mut sb) } != 0 {
        return Ok(Value::Bool(false));
    }
    let zf = unsafe { sys::zip_fopen_index(za, i, 0) };
    if zf.is_null() {
        return Ok(Value::Bool(false));
    }
    d.index_current.set(i + 1);
    let dir = match deref(&args[0]) {
        Value::Resource(r) => Rc::downgrade(&r),
        _ => Weak::new(),
    };
    let entry = ZipEntry {
        zf: Cell::new(zf),
        name: unsafe { sys::cstr(sb.name) }.to_vec(),
        size: sb.size,
        comp_size: sb.comp_size,
        comp_method: sb.comp_method,
        dir,
    };
    Ok(host.resource_insert(ENTRY, Box::new(entry)))
}

fn zip_entry_open(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "zip_entry_open";
    resource_arg(host, args, 0, fname, "zip_dp")?;
    resource_arg(host, args, 1, fname, "zip_entry")?;
    if args.len() > 2 {
        crate::zend::zpp::str_arg(host, args, 2, fname, "mode")?;
    }
    let open = fetch::<ZipEntry>(host, args, 1, fname, "zip_entry", ENTRY)?
        .zf
        .get()
        .is_null();
    fetch::<ZipDir>(host, args, 0, fname, "zip_dp", DIR)?;
    Ok(Value::Bool(!open))
}

fn zip_entry_close(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "zip_entry_close";
    fetch::<ZipEntry>(host, args, 0, fname, "zip_entry", ENTRY)?;
    let v = deref(&args[0]);
    drop(host.resource_remove(&v));
    Ok(Value::Bool(true))
}

fn zip_entry_read(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "zip_entry_read";
    resource_arg(host, args, 0, fname, "zip_entry")?;
    let mut len = match args.get(1) {
        Some(_) => crate::zend::zpp::long_arg(host, args, 1, fname, "len")?,
        None => 0,
    };
    let e = fetch::<ZipEntry>(host, args, 0, fname, "zip_entry", ENTRY)?;
    if len <= 0 {
        len = 1024;
    }
    let zf = e.zf.get();
    if zf.is_null() {
        return Ok(Value::Bool(false));
    }
    let mut buf = vec![0u8; len as usize];
    let n = unsafe { sys::zip_fread(zf, buf.as_mut_ptr().cast(), buf.len() as u64) };
    if n > 0 {
        buf.truncate(n as usize);
        Ok(Value::str(buf))
    } else {
        Ok(Value::str(""))
    }
}

/// php_zip_entry_get_info
fn entry_info(host: &mut dyn Host, args: &[Value], fname: &str, opt: u8) -> Result<Value, Thrown> {
    let e = fetch::<ZipEntry>(host, args, 0, fname, "zip_entry", ENTRY)?;
    let dir_alive = e.dir.upgrade().is_some();
    if e.zf.get().is_null() || !dir_alive {
        return Ok(Value::Bool(false));
    }
    Ok(match opt {
        0 => Value::str(e.name.clone()),
        1 => Value::Int(e.comp_size as i64),
        2 => Value::Int(e.size as i64),
        _ => match e.comp_method {
            0 => Value::str("stored"),
            1 => Value::str("shrunk"),
            2..=5 => Value::str("reduced"),
            6 => Value::str("imploded"),
            7 => Value::str("tokenized"),
            8 => Value::str("deflated"),
            9 => Value::str("deflatedX"),
            10 => Value::str("implodedX"),
            _ => Value::Bool(false),
        },
    })
}

fn zip_entry_name(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    entry_info(host, args, "zip_entry_name", 0)
}

fn zip_entry_compressedsize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "zip_entry_compressedsize";
    entry_info(host, args, f, 1)
}

fn zip_entry_filesize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "zip_entry_filesize";
    entry_info(host, args, f, 2)
}

fn zip_entry_compressionmethod(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "zip_entry_compressionmethod";
    entry_info(host, args, f, 3)
}

pub(super) fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("zip_open", zip_open),
            ("zip_close", zip_close),
            ("zip_read", zip_read),
            ("zip_entry_open", zip_entry_open),
            ("zip_entry_close", zip_entry_close),
            ("zip_entry_read", zip_entry_read),
            ("zip_entry_name", zip_entry_name),
            ("zip_entry_compressedsize", zip_entry_compressedsize),
            ("zip_entry_filesize", zip_entry_filesize),
            ("zip_entry_compressionmethod", zip_entry_compressionmethod),
        ],
    );
}
