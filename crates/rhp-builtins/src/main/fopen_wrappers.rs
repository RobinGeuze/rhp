//! main/fopen_wrappers.c's open_basedir: the directive's runtime handler
//! (OnUpdateBaseDir, which may only tighten it) and php_check_open_basedir,
//! which every path-taking operation runs before it touches the file.
//!
//! The parsed setting lives in a thread-local next to the ini table, so a
//! request without open_basedir pays one flag read per check.

use std::cell::{Cell, RefCell};

use crate::main::streams::{self, plain_wrapper};
use crate::zend::virtual_cwd;
use crate::{Host, Thrown, E_WARNING};

thread_local! {
    /// open_basedir's value; `None` when unset or empty.
    static BASEDIR: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    /// `BASEDIR.is_some()`, for the hot paths.
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
}

/// php's MAXPATHLEN.
const MAXPATHLEN: usize = libc::PATH_MAX as usize;

/// A new request: no open_basedir until the ini says so.
pub(crate) fn reset_state() {
    set(None);
}

/// The directive's new value (startup, or what OnUpdateBaseDir accepted).
pub(crate) fn set(v: Option<&[u8]>) {
    let v = v.filter(|v| !v.is_empty()).map(<[u8]>::to_vec);
    ACTIVE.set(v.is_some());
    BASEDIR.with(|b| *b.borrow_mut() = v);
}

/// Whether open_basedir is in effect.
#[inline]
pub fn active() -> bool {
    ACTIVE.get()
}

/// OnUpdateBaseDir at run time: every entry of `v` must lie inside the
/// current setting. On success the setting becomes the entries expanded
/// (what the checks and their warnings use; ini_get() shows `v`); false
/// when php's handler fails.
pub(crate) fn on_update(v: &[u8]) -> bool {
    match expanded_tightening(v) {
        Some(resolved) => {
            set(Some(&resolved));
            true
        }
        None => false,
    }
}

/// OnUpdateBaseDir's check: `v` with each entry expanded, if every entry
/// lies inside the current setting.
fn expanded_tightening(v: &[u8]) -> Option<Vec<u8>> {
    if v.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for entry in entries(v) {
        // expand_filepath fails on an empty entry
        let resolved = virtual_cwd::expand_filepath_checked(entry).ok()?;
        if denial(&resolved).is_some() {
            return None;
        }
        if !out.is_empty() {
            out.push(b':');
        }
        out.extend_from_slice(&resolved);
    }
    Some(out)
}

/// The entries of an open_basedir list as php's strchr loop walks them:
/// split at ':', where a trailing ':' adds no empty entry.
fn entries(v: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut parts: Vec<&[u8]> = v.split(|&b| b == b':').collect();
    if parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts.into_iter()
}

/// Why php_check_open_basedir refuses a path, with the warning it raises
/// (an over-long path warns even when the caller asked for no warning).
enum Denied {
    TooLong(String),
    Outside(String),
}

/// php_check_open_basedir_ex without the warning: the refusal, if any.
fn denial(path: &[u8]) -> Option<Denied> {
    BASEDIR.with(|b| {
        let b = b.borrow();
        let basedir = b.as_deref()?;
        if path.len() > MAXPATHLEN - 1 {
            return Some(Denied::TooLong(format!(
                "File name is longer than the maximum allowed path length on this platform ({MAXPATHLEN}): {}",
                String::from_utf8_lossy(path)
            )));
        }
        if entries(basedir)
            .filter(|e| !e.is_empty())
            .any(|dir| check_specific(dir, path))
        {
            return None;
        }
        Some(Denied::Outside(format!(
            "open_basedir restriction in effect. File({}) is not within the allowed path(s): ({})",
            String::from_utf8_lossy(path),
            String::from_utf8_lossy(basedir)
        )))
    })
}

/// The warning php_check_open_basedir raises for `path`, or `None` when
/// `path` may be used (for a caller that raises it itself).
pub fn refusal(path: &[u8]) -> Option<String> {
    if !active() {
        return None;
    }
    refuse(path).map(|d| match d {
        Denied::TooLong(m) | Denied::Outside(m) => m,
    })
}

/// php_check_open_basedir_ex(path, warn) from builtin `fname`: true when
/// `path` may be used. A refusal sets errno (EPERM, EINVAL for a path
/// too long) and raises php's warning.
pub fn check(host: &mut dyn Host, fname: &str, path: &[u8], warn: bool) -> Result<bool, Thrown> {
    if !active() {
        return Ok(true);
    }
    match refuse(path) {
        None => Ok(true),
        Some(Denied::TooLong(m)) => {
            crate::warn(host, fname, &m)?;
            Ok(false)
        }
        Some(Denied::Outside(m)) => {
            if warn {
                crate::warn(host, fname, &m)?;
            }
            Ok(false)
        }
    }
}

/// The plain-files opener's check of `local` for builtin `fname`: on a
/// refusal also php's "Failed to open stream" warning for `shown`.
pub fn check_open(
    host: &mut dyn Host,
    fname: &str,
    local: &[u8],
    shown: &[u8],
) -> Result<bool, Thrown> {
    if check(host, fname, local, true)? {
        return Ok(true);
    }
    crate::warn_arg(
        host,
        fname,
        shown,
        &format!("Failed to open stream: {}", streams::last_strerror()),
    )?;
    Ok(false)
}

/// [`check`] from code without the host (a wrapper operation): the warning
/// is queued for the builtin that ran it (see [`plain_wrapper::push_diag`]).
pub fn check_queued(path: &[u8], warn: bool) -> bool {
    if !active() {
        return true;
    }
    match refuse(path) {
        None => true,
        Some(Denied::TooLong(m)) => {
            plain_wrapper::push_diag(E_WARNING, true, m);
            false
        }
        Some(Denied::Outside(m)) => {
            if warn {
                plain_wrapper::push_diag(E_WARNING, true, m);
            }
            false
        }
    }
}

/// [`denial`], recording php's errno for the refusal.
fn refuse(path: &[u8]) -> Option<Denied> {
    let d = denial(path)?;
    let e = match d {
        Denied::TooLong(_) => libc::EINVAL,
        Denied::Outside(_) => libc::EPERM,
    };
    streams::note_io_err(&std::io::Error::from_raw_os_error(e));
    Some(d)
}

/// php_check_specific_open_basedir: whether `path` lies within `basedir`
/// ("." is the working directory). The deepest existing ancestor of the
/// expanded path is resolved and compared by prefix.
fn check_specific(basedir: &[u8], path: &[u8]) -> bool {
    let cwd;
    let local: &[u8] = if basedir == b"." {
        match virtual_cwd::current_dir() {
            Ok(c) => {
                cwd = c;
                &cwd
            }
            Err(_) => basedir,
        }
    } else {
        basedir
    };
    if path.len() > MAXPATHLEN - 1 {
        return false;
    }
    let Ok(mut resolved_name) = virtual_cwd::expand_filepath_checked(path) else {
        return false;
    };
    let mut path_tmp = resolved_name.clone();
    let mut first = true;
    loop {
        if let Some(real) = virtual_cwd::realpath_strict(&path_tmp) {
            resolved_name = real;
            break;
        }
        if first {
            // a broken symlink: go on from its target
            if let Ok(target) = std::fs::read_link(virtual_cwd::os_path(&path_tmp)) {
                use std::os::unix::ffi::OsStringExt;
                path_tmp = target.into_os_string().into_vec();
            }
        }
        let Some(slash) = path_tmp.iter().rposition(|&b| b == b'/') else {
            // none of the components exist
            return false;
        };
        path_tmp.truncate(slash);
        first = false;
        if path_tmp.is_empty() {
            break;
        }
    }
    let Ok(mut resolved_basedir) = virtual_cwd::expand_filepath_checked(local) else {
        return false;
    };
    if !basedir.ends_with(b"/") || !resolved_basedir.ends_with(b"/") {
        resolved_basedir.push(b'/');
    }
    // the path as given ended in a slash (only when it exists as is)
    if first && path_tmp.ends_with(b"/") && !resolved_name.ends_with(b"/") {
        resolved_name.push(b'/');
    }
    let blen = resolved_basedir.len();
    if resolved_name.starts_with(&resolved_basedir) {
        // the base dir ends in '/', so a longer name continues inside it
        return true;
    }
    // "/openbasedir/" and "/openbasedir" are the same directory
    blen == resolved_name.len() + 1 && resolved_basedir[..blen - 1] == resolved_name[..]
}
