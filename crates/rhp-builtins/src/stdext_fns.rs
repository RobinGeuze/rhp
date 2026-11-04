//! Standard-library builtins that tooling (phpunit, psalm, php-cs-fixer,
//! rector) needs: the seeded Mt19937 engine (`mt_srand`/`mt_rand`/`rand`,
//! `shuffle`, `array_rand`, `str_shuffle`), `array_change_key_case`,
//! `array_multisort`, `settype`, `fnmatch`, process introspection
//! (`getrusage`, `gc_status`, `sys_getloadavg`, `memory_reset_peak_usage`,
//! `gc_mem_caches`), `getopt` and `readline`.

use rhp_values::Value;

use crate::ext::random::{MT_RAND_MT19937, MT_RAND_PHP};
use crate::zend::zpp::{int_param, str_param};
use crate::{Host, Thrown};

// --- filesystem --------------------------------------------------------------

const MAXPATHLEN: usize = libc::PATH_MAX as usize;

fn fn_fnmatch(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let pattern = str_param(host, args, 0, "fnmatch", "pattern")?;
    let filename = str_param(host, args, 1, "fnmatch", "filename")?;
    let flags = int_param(host, args, 2, "fnmatch", "flags", 0)?;
    if pattern.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            "fnmatch(): Argument #1 ($pattern) must not contain any null bytes",
        ));
    }
    if filename.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            "fnmatch(): Argument #2 ($filename) must not contain any null bytes",
        ));
    }
    if filename.len() >= MAXPATHLEN {
        crate::warn(
            host,
            "fnmatch",
            &format!("Filename exceeds the maximum allowed length of {MAXPATHLEN} characters"),
        )?;
        return Ok(Value::Bool(false));
    }
    if pattern.len() >= MAXPATHLEN {
        crate::warn(
            host,
            "fnmatch",
            &format!("Pattern exceeds the maximum allowed length of {MAXPATHLEN} characters"),
        )?;
        return Ok(Value::Bool(false));
    }
    let p = std::ffi::CString::new(pattern).unwrap();
    let f = std::ffi::CString::new(filename).unwrap();
    let rc = unsafe { libc::fnmatch(p.as_ptr(), f.as_ptr(), flags as libc::c_int) };
    Ok(Value::Bool(rc == 0))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("fnmatch", fn_fnmatch)]);
}

/// The `FNM_*` / `MT_RAND_*` constants.
pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("FNM_NOESCAPE", Value::Int(libc::FNM_NOESCAPE as i64)),
        ("FNM_PATHNAME", Value::Int(libc::FNM_PATHNAME as i64)),
        ("FNM_PERIOD", Value::Int(libc::FNM_PERIOD as i64)),
        ("FNM_CASEFOLD", Value::Int(libc::FNM_CASEFOLD as i64)),
        ("CASE_LOWER", Value::Int(0)),
        ("CASE_UPPER", Value::Int(1)),
        ("MT_RAND_MT19937", Value::Int(MT_RAND_MT19937)),
        ("MT_RAND_PHP", Value::Int(MT_RAND_PHP)),
    ]
}
