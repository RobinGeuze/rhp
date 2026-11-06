//! ext/standard/dl.c: `dl()`, which php-cli and php-cgi add to `standard`
//! (see sapi/). rhp cannot load php extension modules: the checks and the
//! "Unable to load" warning are php's; a library that loads is refused as
//! unsupported.

use rhp_values::{conv, ops::deref, Value};

use crate::main::ini::{ini_parse_bool, ini_value};
use crate::{Host, Thrown};

/// dlopen(3) `path`: Ok(()) when it loads (closed again), else dlerror().
fn load_shlib(path: &[u8]) -> Result<bool, String> {
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
