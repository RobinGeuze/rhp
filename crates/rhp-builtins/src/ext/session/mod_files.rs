//! ext/session/mod_files.c: the `files` save handler. One file per session
//! (`sess_<id>`) under session.save_path ("[N;[MODE;]]/path"), locked with
//! flock while the session is open.

use std::os::fd::{AsRawFd, OwnedFd};

use crate::zend::virtual_cwd;
use crate::{Host, Thrown, E_NOTICE, E_WARNING};

const FILE_PREFIX: &[u8] = b"sess_";

#[cfg(target_os = "macos")]
const MAXPATHLEN: usize = 1024;
#[cfg(not(target_os = "macos"))]
const MAXPATHLEN: usize = 4096;

/// `ps_files`.
pub(super) struct Files {
    last_key: Option<Vec<u8>>,
    basedir: Vec<u8>,
    dirdepth: usize,
    st_size: u64,
    filemode: u32,
    fd: Option<OwnedFd>,
}

/// `strtol(s, NULL, base)`: the value and whether it overflowed (ERANGE).
fn strtol(s: &[u8], base: u32) -> (i64, bool) {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let neg = match s.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut v: i128 = 0;
    let mut over = false;
    while let Some(d) = s.get(i).and_then(|&c| (c as char).to_digit(base)) {
        v = v * base as i128 + d as i128;
        if v > i64::MAX as i128 + 1 {
            over = true;
            v = i64::MAX as i128 + 1;
        }
        i += 1;
    }
    let v = if neg { -v } else { v };
    if v > i64::MAX as i128 {
        return (i64::MAX, true);
    }
    (v as i64, over)
}

/// The prefix of `s` up to its first NUL (a C string).
fn c_str(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
}

/// `php_session_valid_key`: `[a-zA-Z0-9,-]`, 1 to 256 characters (as a C
/// string).
pub(super) fn valid_key(key: &[u8]) -> bool {
    let key = c_str(key);
    key.iter()
        .all(|&c| c.is_ascii_alphanumeric() || c == b',' || c == b'-')
        && !key.is_empty()
        && key.len() <= super::MAX_SID_LENGTH
}

impl Files {
    /// ps_files_path_create.
    fn path(&self, key: &[u8]) -> Option<Vec<u8>> {
        if key.len() <= self.dirdepth
            || MAXPATHLEN
                < self.basedir.len() + 2 * self.dirdepth + key.len() + 5 + FILE_PREFIX.len() + 1
        {
            return None;
        }
        let mut buf = self.basedir.clone();
        buf.push(b'/');
        for &c in &key[..self.dirdepth] {
            buf.push(c);
            buf.push(b'/');
        }
        buf.extend_from_slice(FILE_PREFIX);
        buf.extend_from_slice(key);
        // A C string from here on.
        Some(c_str(&buf).to_vec())
    }

    /// ps_files_open: open and lock the file of `key`, unless it is the
    /// one already open.
    fn open_key(&mut self, host: &mut dyn Host, fname: &str, key: &[u8]) -> Result<(), Thrown> {
        if self.fd.is_some() && self.last_key.as_deref() == Some(key) {
            return Ok(());
        }
        self.last_key = None;
        self.fd = None;
        if !valid_key(key) {
            return super::docref(
                host,
                fname,
                E_WARNING,
                "Session ID is too long or contains illegal characters. Only the A-Z, a-z, 0-9, \"-\", and \",\" characters are allowed",
            );
        }
        let Some(buf) = self.path(key) else {
            let msg = format!("Failed to create session data file path. Too short session ID, invalid save_path or path length exceeds {MAXPATHLEN} characters");
            return super::docref(host, fname, E_WARNING, &msg);
        };
        self.last_key = Some(key.to_vec());
        let flags = libc::O_CREAT | libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        match virtual_cwd::open(&buf, flags, self.filemode) {
            Ok(fd) => {
                let raw = fd.as_raw_fd();
                let mut st: libc::stat = unsafe { std::mem::zeroed() };
                // SAFETY: `raw` is the open descriptor `fd` owns.
                let bad = unsafe { libc::fstat(raw, &mut st) } != 0 || {
                    let (uid, euid) = unsafe { (libc::getuid(), libc::geteuid()) };
                    st.st_uid != 0 && st.st_uid != uid && st.st_uid != euid && uid != 0
                };
                if bad {
                    drop(fd);
                    return super::docref(
                        host,
                        fname,
                        E_WARNING,
                        "Session data file is not created by your uid",
                    );
                }
                loop {
                    // SAFETY: as above.
                    if unsafe { libc::flock(raw, libc::LOCK_EX) } == 0
                        || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR)
                    {
                        break;
                    }
                }
                self.fd = Some(fd);
            }
            Err(e) => {
                let errno = e.raw_os_error().unwrap_or(0);
                let msg = format!(
                    "open({}, O_RDWR) failed: {} ({errno})",
                    String::from_utf8_lossy(&buf),
                    crate::main::streams::strerror(errno)
                );
                super::docref(host, fname, E_WARNING, &msg)?;
            }
        }
        Ok(())
    }

    /// ps_files_write.
    fn write_key(
        &mut self,
        host: &mut dyn Host,
        fname: &str,
        key: &[u8],
        val: &[u8],
    ) -> Result<bool, Thrown> {
        self.open_key(host, fname, key)?;
        let Some(fd) = &self.fd else {
            return Ok(false);
        };
        let raw = fd.as_raw_fd();
        // SAFETY: `raw` is open; the buffer outlives the call.
        let n = unsafe {
            if (val.len() as u64) < self.st_size {
                libc::ftruncate(raw, 0);
            }
            libc::pwrite(raw, val.as_ptr().cast(), val.len(), 0)
        };
        if n as usize != val.len() || n < 0 {
            if n < 0 {
                let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                let msg = format!(
                    "Write failed: {} ({errno})",
                    crate::main::streams::strerror(errno)
                );
                super::docref(host, fname, E_WARNING, &msg)?;
            } else {
                super::docref(
                    host,
                    fname,
                    E_WARNING,
                    "Write wrote less bytes than requested",
                )?;
            }
            return Ok(false);
        }
        Ok(true)
    }

    fn key_exists(&self, key: &[u8]) -> bool {
        self.path(key)
            .is_some_and(|p| virtual_cwd::stat(&p, true).is_ok())
    }
}

/// PS_OPEN_FUNC(files): `None` is FAILURE.
pub(super) fn open(
    host: &mut dyn Host,
    fname: &str,
    save_path: &[u8],
) -> Result<Option<Files>, Thrown> {
    let mut save_path = c_str(save_path).to_vec();
    if save_path.is_empty() {
        save_path = crate::ext::standard::file::temp_dir();
        let func = fname.strip_suffix("()").unwrap_or(fname);
        if crate::main::fopen_wrappers::active()
            && !crate::main::fopen_wrappers::check(host, func, &save_path, true)?
        {
            return Ok(None);
        }
    }
    // "N;MODE;/path": the last part may hold more `;`.
    let mut argv: Vec<&[u8]> = Vec::new();
    let mut rest = &save_path[..];
    while let Some(i) = rest.iter().position(|&c| c == b';') {
        argv.push(&rest[..i]);
        rest = &rest[i + 1..];
        if argv.len() > 1 {
            break;
        }
    }
    argv.push(rest);
    let mut dirdepth = 0usize;
    let mut filemode = 0o600u32;
    if argv.len() > 1 {
        let (d, over) = strtol(argv[0], 10);
        if over {
            host.emit_diagnostic(
                E_WARNING,
                b"The first parameter in session.save_path is invalid",
            )?;
            return Ok(None);
        }
        dirdepth = d as usize;
    }
    if argv.len() > 2 {
        let (m, over) = strtol(argv[1], 8);
        if over || !(0..=0o7777).contains(&m) {
            host.emit_diagnostic(
                E_WARNING,
                b"The second parameter in session.save_path is invalid",
            )?;
            return Ok(None);
        }
        filemode = m as u32;
    }
    Ok(Some(Files {
        last_key: None,
        basedir: argv[argv.len() - 1].to_vec(),
        dirdepth,
        st_size: 0,
        filemode,
        fd: None,
    }))
}

/// PS_CLOSE_FUNC(files): the file closes (and unlocks).
pub(super) fn close(f: Files) {
    drop(f);
}

/// PS_READ_FUNC(files): `None` is FAILURE.
pub(super) fn read(
    host: &mut dyn Host,
    fname: &str,
    f: &mut Files,
    key: &[u8],
) -> Result<Option<Vec<u8>>, Thrown> {
    f.open_key(host, fname, key)?;
    let Some(fd) = &f.fd else {
        return Ok(None);
    };
    let raw = fd.as_raw_fd();
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `raw` is open.
    if unsafe { libc::fstat(raw, &mut st) } != 0 {
        return Ok(None);
    }
    f.st_size = st.st_size as u64;
    if st.st_size == 0 {
        return Ok(Some(Vec::new()));
    }
    let mut buf = vec![0u8; st.st_size as usize];
    // SAFETY: the buffer holds `len` bytes.
    let n = unsafe { libc::pread(raw, buf.as_mut_ptr().cast(), buf.len(), 0) };
    if n != st.st_size as isize {
        if n < 0 {
            let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            let msg = format!(
                "Read failed: {} ({errno})",
                crate::main::streams::strerror(errno)
            );
            super::docref(host, fname, E_WARNING, &msg)?;
        } else {
            super::docref(
                host,
                fname,
                E_WARNING,
                "Read returned less bytes than requested",
            )?;
        }
        return Ok(None);
    }
    Ok(Some(buf))
}

/// PS_WRITE_FUNC(files).
pub(super) fn write(
    host: &mut dyn Host,
    fname: &str,
    f: &mut Files,
    key: &[u8],
    val: &[u8],
) -> Result<bool, Thrown> {
    f.write_key(host, fname, key, val)
}

/// PS_UPDATE_TIMESTAMP_FUNC(files): touch the file, or write it when it
/// does not exist.
pub(super) fn update_timestamp(
    host: &mut dyn Host,
    fname: &str,
    f: &mut Files,
    key: &[u8],
    val: &[u8],
) -> Result<bool, Thrown> {
    let Some(path) = f.path(key) else {
        return Ok(false);
    };
    let touched = virtual_cwd::c_path(&path)
        // SAFETY: a valid C string; NULL times means now.
        .is_some_and(|c| unsafe { libc::utime(c.as_ptr(), std::ptr::null()) } == 0);
    if !touched {
        return f.write_key(host, fname, key, val);
    }
    Ok(true)
}

/// PS_DESTROY_FUNC(files).
pub(super) fn destroy(f: &mut Files, key: &[u8]) -> bool {
    let Some(path) = f.path(key) else {
        return false;
    };
    if f.fd.is_some() {
        f.fd = None;
        let gone = virtual_cwd::c_path(&path)
            // SAFETY: a valid C string.
            .is_some_and(|c| unsafe { libc::unlink(c.as_ptr()) } == 0);
        if !gone && virtual_cwd::access(&path, libc::F_OK) {
            return false;
        }
    }
    true
}

/// PS_GC_FUNC(files): how many files went (-1: none looked at).
pub(super) fn gc(
    host: &mut dyn Host,
    fname: &str,
    f: &mut Files,
    maxlifetime: i64,
) -> Result<i64, Thrown> {
    if f.dirdepth != 0 {
        return Ok(-1);
    }
    let dir = f.basedir.clone();
    let rd = match std::fs::read_dir(virtual_cwd::os_path(&dir)) {
        Ok(rd) => rd,
        Err(e) => {
            let errno = e.raw_os_error().unwrap_or(0);
            let msg = format!(
                "ps_files_cleanup_dir: opendir({}) failed: {} ({errno})",
                String::from_utf8_lossy(&dir),
                crate::main::streams::strerror(errno)
            );
            super::docref(host, fname, E_NOTICE, &msg)?;
            return Ok(-1);
        }
    };
    if dir.len() >= MAXPATHLEN {
        let msg = format!(
            "ps_files_cleanup_dir: dirname({}) is too long",
            String::from_utf8_lossy(&dir)
        );
        super::docref(host, fname, E_NOTICE, &msg)?;
        return Ok(-1);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let mut n = 0;
    for e in rd.flatten() {
        use std::os::unix::ffi::OsStrExt;
        let name = e.file_name();
        let name = name.as_bytes();
        if !name.starts_with(FILE_PREFIX) || name.len() + dir.len() + 2 >= MAXPATHLEN {
            continue;
        }
        let mut path = dir.clone();
        path.push(b'/');
        path.extend_from_slice(name);
        if let Ok(st) = virtual_cwd::stat(&path, true) {
            if now - st.st_mtime > maxlifetime {
                if let Some(c) = virtual_cwd::c_path(&path) {
                    // SAFETY: a valid C string.
                    unsafe { libc::unlink(c.as_ptr()) };
                }
                n += 1;
            }
        }
    }
    Ok(n)
}

/// PS_CREATE_SID_FUNC(files): a new id no file has yet.
pub(super) fn create_sid(
    host: &mut dyn Host,
    f: Option<&Files>,
) -> Result<Option<Vec<u8>>, Thrown> {
    let mut maxfail = 3;
    loop {
        let Some(sid) = super::create_id(host)? else {
            maxfail -= 1;
            if maxfail < 0 {
                return Ok(None);
            }
            continue;
        };
        if f.is_some_and(|f| f.key_exists(&sid)) {
            maxfail -= 1;
            if maxfail < 0 {
                return Ok(None);
            }
            continue;
        }
        return Ok(Some(sid));
    }
}

/// PS_VALIDATE_SID_FUNC(files): the session exists.
pub(super) fn validate_sid(f: Option<&Files>, key: &[u8]) -> bool {
    f.is_some_and(|f| f.key_exists(key))
}
