//! ext/standard/dl.c: `dl()`, which php-cli and php-cgi add to `standard`
//! (see sapi/). rhp cannot load php extension modules: the checks and the
//! "Unable to load" warning are php's; a library that loads is refused as
//! unsupported.
//!
//! A php extension module (a library exporting `get_module`) usually fails
//! dlopen() under rhp only because it links against php's engine symbols
//! (`compiler_globals`, ...), which rhp does not have. Such a module counts
//! as loaded but not runnable, like one that loads: an `extension=` entry
//! for it is skipped without the startup warning (as by a php built without
//! DL support, whose `extension=` entries do nothing). See
//! docs/design/extensions.md ("php extension modules").

use rhp_values::{conv, ops::deref, Value};

use crate::main::ini::{ini_parse_bool, ini_value};
use crate::{Host, Thrown};

/// Whether `path` is a php extension module: a shared library whose
/// dynamic symbol table exports `get_module` (`_get_module` on Mach-O), the
/// symbol php_load_extension looks up. Read without loading it.
fn is_php_module(path: &[u8]) -> bool {
    use object::Object;
    use std::os::unix::ffi::OsStrExt;
    let Ok(data) = std::fs::read(std::ffi::OsStr::from_bytes(path)) else {
        return false;
    };
    let Ok(file) = object::File::parse(&*data) else {
        return false;
    };
    file.exports().is_ok_and(|exports| {
        exports
            .iter()
            .any(|e| matches!(e.name(), b"get_module" | b"_get_module"))
    })
}

/// dlopen(3) `path`: Ok(whether it is a php module) when it loads (closed
/// again), else dlerror(). A php module that fails to load is Ok(true)
/// (see the module comment).
fn load_shlib(path: &[u8]) -> Result<bool, String> {
    load_shlib_raw(path).or_else(|e| {
        if is_php_module(path) {
            Ok(true)
        } else {
            Err(e)
        }
    })
}

fn load_shlib_raw(path: &[u8]) -> Result<bool, String> {
    let c = std::ffi::CString::new(path).map_err(|_| String::new())?;
    // SAFETY: a NUL-terminated path; the handle is closed right away.
    unsafe {
        let h = libc::dlopen(c.as_ptr(), libc::RTLD_LAZY | libc::RTLD_GLOBAL);
        if h.is_null() {
            let e = libc::dlerror();
            return Err(if e.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(e).to_string_lossy().into_owned()
            });
        }
        let module = !libc::dlsym(h, c"get_module".as_ptr()).is_null()
            || !libc::dlsym(h, c"_get_module".as_ptr()).is_null();
        libc::dlclose(h);
        Ok(module)
    }
}

/// php_load_extension for an `extension=` entry at startup
/// (MODULE_PERSISTENT): the "Unable to load" message when the library does
/// not load. One that loads, or a php module, is left alone (rhp cannot
/// run it).
pub(crate) fn startup_extension_error(filename: &[u8]) -> Option<String> {
    let dir = ini_value(b"extension_dir").flatten().unwrap_or_default();
    let has_slash = filename.contains(&b'/');
    if !has_slash && dir.is_empty() {
        return None;
    }
    let slash_suffix = !has_slash && dir.ends_with(b"/");
    let sep: &[u8] = if slash_suffix { b"" } else { b"/" };
    let first = if has_slash {
        filename.to_vec()
    } else {
        [&dir[..], sep, filename].concat()
    };
    let err1 = load_shlib(&first).err()?;
    let second = [&dir[..], sep, filename, b".so"].concat();
    let err2 = load_shlib(&second).err()?;
    let s = String::from_utf8_lossy;
    Some(format!(
        "PHP Startup: Unable to load dynamic library '{}' (tried: {} ({}), {} ({}))",
        s(filename),
        s(&first),
        err1,
        s(&second),
        err2
    ))
}

pub(crate) fn fn_dl(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let filename = conv::to_string_bytes(&deref(&args[0]));
    let on = |n: &[u8]| ini_value(n).flatten().is_some_and(|v| ini_parse_bool(&v));
    if !on(b"enable_dl") {
        crate::warn(host, "dl", "Dynamically loaded extensions aren't enabled")?;
        return Ok(Value::Bool(false));
    }
    if filename.len() >= libc::PATH_MAX as usize {
        crate::warn(
            host,
            "dl",
            &format!(
                "Filename exceeds the maximum allowed length of {} characters",
                libc::PATH_MAX
            ),
        )?;
        return Ok(Value::Bool(false));
    }
    // php_load_extension, MODULE_TEMPORARY. (It works on the C string.)
    let filename = &filename[..filename
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(filename.len())];
    if filename.contains(&b'/') {
        crate::warn(
            host,
            "dl",
            "Temporary module name should contain only filename",
        )?;
        return Ok(Value::Bool(false));
    }
    let dir = ini_value(b"extension_dir").flatten().unwrap_or_default();
    if dir.is_empty() {
        return Ok(Value::Bool(false));
    }
    let sep: &[u8] = if dir.ends_with(b"/") { b"" } else { b"/" };
    let first = [&dir[..], sep, filename].concat();
    let loaded = match load_shlib(&first) {
        Ok(m) => Ok(m),
        Err(err1) => {
            let second = [&dir[..], sep, filename, b".so"].concat();
            load_shlib(&second).map_err(|err2| {
                let s = String::from_utf8_lossy;
                format!(
                    "Unable to load dynamic library '{}' (tried: {} ({}), {} ({}))",
                    s(filename),
                    s(&first),
                    err1,
                    s(&second),
                    err2
                )
            })
        }
    };
    match loaded {
        Err(msg) => {
            crate::warn(host, "dl", &msg)?;
            Ok(Value::Bool(false))
        }
        Ok(false) => {
            let msg = format!(
                "Invalid library (maybe not a PHP library) '{}'",
                String::from_utf8_lossy(filename)
            );
            crate::warn(host, "dl", &msg)?;
            Ok(Value::Bool(false))
        }
        Ok(true) => Err(Thrown::unsupported(
            "dl",
            "rhp cannot load php extension modules",
        )),
    }
}
