//! The request's file-system context: its working directory (php's virtual
//! cwd, `Zend/zend_virtual_cwd.c` and `main/fopen_wrappers.c`'s
//! `expand_filepath`) and its file creation mask (`umask()`).
//!
//! rhp never changes the process's working directory or umask, so requests
//! on different threads can't disturb each other. The thread's live VM has
//! its own (php's ZTS `CWDG`, reset at every request start). Every path rhp
//! hands the OS, a C library or a child process goes through [`resolve`]
//! (or one of its wrappers here) first; error messages keep the path as the
//! script gave it. Until a request changes them, both are the process's,
//! and [`resolve`] leaves relative paths alone.
//!
//! The calls scripts make most (stat, open, access, opendir) start from a
//! descriptor of the working directory instead ([`at`]): the kernel then
//! walks only the relative path, as it does from a real working directory.
//!
//! A per-request umask can't be set for the process without disturbing
//! other threads, so files are created with the mode computed here
//! ([`create_mode`]); when the request's umask lets through bits the
//! process's takes away, the new file's permissions are fixed up after it
//! was created (see [`open`], [`mkdir`], [`library_created`]).

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, OsStr, OsString};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// A working directory of the request's own.
struct Cwd {
    /// Absolute, without symlinks or dot segments.
    path: Vec<u8>,
    /// The directory opened, for the `*at` calls; `None` when it can't be
    /// read (searching it is enough for chdir).
    fd: Option<OwnedFd>,
}

impl Cwd {
    fn new(path: Vec<u8>) -> Cwd {
        let fd = with_cstr(&path, |c| open_dir(libc::AT_FDCWD, c)).flatten();
        Cwd { path, fd }
    }
}

/// The directory `c` (from `dir`) opened for the `*at` calls.
fn open_dir(dir: libc::c_int, c: &CStr) -> Option<OwnedFd> {
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC;
    let fd = unsafe { libc::openat(dir, c.as_ptr(), flags) };
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
}

/// The path of the directory open at `fd`, as getcwd(3) names it.
fn fd_path(fd: &OwnedFd) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        let mut buf = [0u8; libc::PATH_MAX as usize];
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) } == -1 {
            return None;
        }
        let end = buf.iter().position(|&b| b == 0)?;
        Some(buf[..end].to_vec())
    }
    #[cfg(target_os = "linux")]
    {
        let link = std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).ok()?;
        let path = link.into_os_string().into_vec();
        (path.first() == Some(&b'/') && !path.ends_with(b" (deleted)")).then_some(path)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = fd;
        None
    }
}

/// Whether `path` still names the directory open at `fd`.
fn names_dir(path: &[u8], fd: &OwnedFd) -> bool {
    with_cstr(path, |c| unsafe {
        let mut named: libc::stat = std::mem::zeroed();
        let mut open: libc::stat = std::mem::zeroed();
        libc::stat(c.as_ptr(), &mut named) == 0
            && libc::fstat(fd.as_raw_fd(), &mut open) == 0
            && (named.st_dev, named.st_ino) == (open.st_dev, open.st_ino)
    })
    .unwrap_or(false)
}

thread_local! {
    /// The request's working directory. `None`: the process's.
    static CWD: RefCell<Option<Cwd>> = const { RefCell::new(None) };
    /// The request's umask. `None`: the process's.
    static UMASK: Cell<Option<u32>> = const { Cell::new(None) };
}

/// A new request starts in the process's directory, with its umask.
pub(crate) fn reset_state() {
    CWD.with(|c| *c.borrow_mut() = None);
    UMASK.set(None);
    // Read the process's umask before a script may ask for it.
    process_umask();
}

// --- working directory -------------------------------------------------------

/// Start the request in `dir` (php-cgi: the script's directory; an
/// embedder: the document root). A relative `dir` is taken from the
/// current one. `false`, and nothing changed, when it isn't a directory.
pub fn set_cwd(dir: &[u8]) -> bool {
    match std::fs::canonicalize(os_path(dir)) {
        Ok(real) if real.is_dir() => {
            let cwd = Cwd::new(real.into_os_string().into_vec());
            CWD.with(|c| *c.borrow_mut() = Some(cwd));
            true
        }
        _ => false,
    }
}

/// The request's working directory, by the name it had when the request
/// entered it.
pub fn getcwd() -> Option<Vec<u8>> {
    CWD.with(|c| c.borrow().as_ref().map(|c| c.path.clone()))
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|d| d.into_os_string().into_vec())
        })
}

/// `getcwd()`, as getcwd(3) answers it: the request's working directory
/// by its current name (it takes the new one when the directory was
/// renamed); ENOENT once the directory was removed.
pub fn current_dir() -> Result<Vec<u8>, i32> {
    let found = CWD.with(|c| {
        let c = c.borrow();
        let cwd = c.as_ref()?;
        let Some(fd) = &cwd.fd else {
            return Some(Ok(cwd.path.clone()));
        };
        if names_dir(&cwd.path, fd) {
            return Some(Ok(cwd.path.clone()));
        }
        Some(match fd_path(fd).filter(|p| names_dir(p, fd)) {
            Some(moved) => Ok(moved),
            None => Err(libc::ENOENT),
        })
    });
    match found {
        Some(Ok(path)) => {
            CWD.with(|c| {
                if let Some(cwd) = c.borrow_mut().as_mut() {
                    if cwd.path != path {
                        cwd.path = path.clone();
                    }
                }
            });
            Ok(path)
        }
        Some(Err(e)) => Err(e),
        None => std::env::current_dir()
            .map(|d| d.into_os_string().into_vec())
            .map_err(|e| e.raw_os_error().unwrap_or(libc::ENOENT)),
    }
}

/// `chdir()`: what chdir(2) would do, for the request only. The error is
/// chdir(2)'s errno.
pub fn chdir(path: &[u8]) -> Result<(), i32> {
    if path.is_empty() {
        return Err(libc::ENOENT);
    }
    // fstatat walks the path from the request's directory as chdir(2) does
    // from a real one (ENOENT, ENOTDIR, EACCES, ELOOP, ENAMETOOLONG); the
    // target must be a directory this process may search. Its name is the
    // one getcwd(3) would give, taken from the directory opened.
    let opened = with_at(path, |dir, c| unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        if libc::fstatat(dir, c.as_ptr(), &mut st, 0) != 0 {
            return Err(crate::main::streams::errno());
        }
        if st.st_mode & libc::S_IFMT != libc::S_IFDIR {
            return Err(libc::ENOTDIR);
        }
        if libc::faccessat(dir, c.as_ptr(), libc::X_OK, libc::AT_EACCESS) != 0 {
            return Err(crate::main::streams::errno());
        }
        Ok(open_dir(dir, c))
    })
    .ok_or(libc::ENOENT)??;
    let cwd = match opened.as_ref().and_then(fd_path) {
        Some(path) => Cwd { path, fd: opened },
        None => {
            // Searchable but not readable: name it from the path.
            let real = std::fs::canonicalize(os_path(path))
                .map_err(|e| e.raw_os_error().unwrap_or(libc::ENOENT))?;
            Cwd::new(real.into_os_string().into_vec())
        }
    };
    CWD.with(|c| *c.borrow_mut() = Some(cwd));
    Ok(())
}

/// `path` as the OS must see it: a relative path taken from the request's
/// working directory. Absolute and empty paths, and every path while the
/// request is still in the process's directory, are returned as given.
/// Only the directory is prefixed: `..` and symlinks resolve as the OS
/// resolves them from the working directory.
pub fn resolve(path: &[u8]) -> Cow<'_, [u8]> {
    if path.is_empty() || path[0] == b'/' {
        return Cow::Borrowed(path);
    }
    CWD.with(|c| match &*c.borrow() {
        None => Cow::Borrowed(path),
        Some(cwd) => Cow::Owned(join(&cwd.path, path)),
    })
}

/// Run `f` with `path` as an `*at` system call takes it: the directory
/// descriptor to start from (`AT_FDCWD`, or the request's directory) and
/// the path from there. `None` for a path holding a NUL byte.
pub fn with_at<R>(path: &[u8], f: impl FnOnce(libc::c_int, &CStr) -> R) -> Option<R> {
    let relative = !path.is_empty() && path[0] != b'/';
    let dir = if relative {
        CWD.with(|c| match &*c.borrow() {
            None => Some(libc::AT_FDCWD),
            Some(Cwd { fd: Some(fd), .. }) => Some(fd.as_raw_fd()),
            Some(_) => None,
        })
    } else {
        Some(libc::AT_FDCWD)
    };
    match dir {
        Some(dir) => with_cstr(path, |c| f(dir, c)),
        None => with_cstr(&resolve(path), |c| f(libc::AT_FDCWD, c)),
    }
}

/// `f` with `bytes` as a C string, kept on the stack when short (as std
/// does for its paths).
fn with_cstr<R>(bytes: &[u8], f: impl FnOnce(&CStr) -> R) -> Option<R> {
    const STACK: usize = 384;
    if bytes.len() < STACK {
        let mut buf = [0u8; STACK];
        buf[..bytes.len()].copy_from_slice(bytes);
        CStr::from_bytes_with_nul(&buf[..=bytes.len()]).ok().map(f)
    } else {
        CString::new(bytes).ok().map(|c| f(&c))
    }
}

/// `stat(2)` (`lstat(2)` when not `follow`) of `path` from the request's
/// directory.
pub fn stat(path: &[u8], follow: bool) -> io::Result<libc::stat> {
    let flags = if follow { 0 } else { libc::AT_SYMLINK_NOFOLLOW };
    with_at(path, |dir, c| unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        if libc::fstatat(dir, c.as_ptr(), &mut st, flags) == 0 {
            Ok(st)
        } else {
            Err(io::Error::last_os_error())
        }
    })
    .unwrap_or_else(|| Err(nul_error()))
}

/// `access(2)` of `path` from the request's directory.
pub fn access(path: &[u8], mode: libc::c_int) -> bool {
    with_at(
        path,
        |dir, c| unsafe { libc::faccessat(dir, c.as_ptr(), mode, 0) } == 0,
    )
    .unwrap_or(false)
}

/// The whole file at `path` from the request's directory.
pub fn read_file(path: &[u8]) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let mut f = std::fs::File::from(open(path, libc::O_RDONLY | libc::O_CLOEXEC, 0)?);
    let mut out = Vec::new();
    f.read_to_end(&mut out)?;
    Ok(out)
}

/// [`resolve`] as a `Path`.
pub fn os_path(path: &[u8]) -> Cow<'_, Path> {
    match resolve(path) {
        Cow::Borrowed(b) => Cow::Borrowed(Path::new(OsStr::from_bytes(b))),
        Cow::Owned(v) => Cow::Owned(PathBuf::from(OsString::from_vec(v))),
    }
}

/// [`resolve`] as a C string; `None` for a path holding a NUL byte.
pub fn c_path(path: &[u8]) -> Option<CString> {
    CString::new(resolve(path).into_owned()).ok()
}

/// `dir/path` (`/path` when `dir` is the root).
fn join(dir: &[u8], path: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(dir.len() + 1 + path.len());
    out.extend_from_slice(dir);
    if out.last() != Some(&b'/') {
        out.push(b'/');
    }
    out.extend_from_slice(path);
    out
}

/// `path` from the request's working directory: absolute, as the OS sees
/// it, even while the request is in the process's directory.
pub fn absolute(path: &[u8]) -> Vec<u8> {
    if path.starts_with(b"/") {
        return path.to_vec();
    }
    match getcwd() {
        Some(cwd) => join(&cwd, path),
        None => path.to_vec(),
    }
}

/// virtual_file_ex(CWD_EXPAND) of an absolute path: `.` and `..` folded
/// and repeated slashes dropped, lexically; at least "/".
pub fn normalize(path: &[u8]) -> Vec<u8> {
    let mut parts: Vec<&[u8]> = Vec::new();
    for seg in path.split(|&b| b == b'/') {
        match seg {
            b"" | b"." => {}
            b".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let mut out = Vec::with_capacity(path.len() + 1);
    for p in parts {
        out.push(b'/');
        out.extend_from_slice(p);
    }
    if out.is_empty() {
        out.push(b'/');
    }
    out
}

/// php's `expand_filepath` with CWD_EXPAND: [`absolute`], then
/// [`normalize`]d (symlinks kept).
pub fn expand_path(path: &[u8]) -> Vec<u8> {
    normalize(&absolute(path))
}

/// php's `expand_filepath` (CWD_FILEPATH): [`expand_path`], with the
/// symlinks in its longest existing leading part resolved. A URL
/// (`scheme://…`) is returned as given.
pub fn expand_filepath(path: &[u8]) -> Vec<u8> {
    if path.windows(3).any(|w| w == b"://") {
        return path.to_vec();
    }
    expand_local(path)
}

/// [`expand_filepath`] of any path, a `scheme://` one included (php's
/// plain-files fallback for an unknown wrapper).
pub fn expand_local(path: &[u8]) -> Vec<u8> {
    let full = expand_path(path);
    let parts: Vec<&[u8]> = full
        .split(|&b| b == b'/')
        .filter(|s| !s.is_empty())
        .collect();
    let join = |segs: &[&[u8]]| {
        let mut out = Vec::new();
        for seg in segs {
            out.push(b'/');
            out.extend_from_slice(seg);
        }
        out
    };
    for keep in (1..=parts.len()).rev() {
        if let Ok(real) = std::fs::canonicalize(OsStr::from_bytes(&join(&parts[..keep]))) {
            let mut out = real.into_os_string().into_vec();
            out.extend_from_slice(&join(&parts[keep..]));
            return out;
        }
    }
    full
}

/// `realpath()` (php's tsrm_realpath): the canonical path of an existing
/// file; "" is the working directory.
pub fn realpath(path: &[u8]) -> Option<Vec<u8>> {
    let path = if path.is_empty() { b"." as &[u8] } else { path };
    std::fs::canonicalize(os_path(path))
        .ok()
        .map(|p| p.into_os_string().into_vec())
}

// --- umask -------------------------------------------------------------------

/// The process's umask, read once. Linux reports it in /proc; elsewhere
/// reading it means setting it, which the first engine does once.
fn process_umask() -> u32 {
    static MASK: OnceLock<u32> = OnceLock::new();
    *MASK.get_or_init(|| {
        #[cfg(target_os = "linux")]
        if let Ok(s) = std::fs::read_to_string("/proc/self/status") {
            let field = s.lines().find_map(|l| l.strip_prefix("Umask:"));
            if let Some(m) = field.and_then(|f| u32::from_str_radix(f.trim(), 8).ok()) {
                return m;
            }
        }
        // 0777 for the moment it's set: a file another thread creates
        // then can only come out stricter, never looser.
        unsafe {
            let m = libc::umask(0o777);
            libc::umask(m);
            m as u32
        }
    })
}

/// `umask()`: the request's file creation mask.
pub fn umask() -> u32 {
    UMASK.get().unwrap_or_else(process_umask)
}

/// The bits of a mask the kernel keeps: macOS keeps the setuid, setgid
/// and sticky bits too (`ALLPERMS`), Linux only the permissions.
#[cfg(target_os = "macos")]
const MASK_BITS: u32 = 0o7777;
#[cfg(not(target_os = "macos"))]
const MASK_BITS: u32 = 0o777;

/// `umask($mask)`: set the request's mask; returns the old one.
pub fn set_umask(mask: u32) -> u32 {
    let old = umask();
    UMASK.set(Some(mask & MASK_BITS));
    old
}

/// The permission bits a file created with `mode` should have.
pub fn create_mode(mode: u32) -> u32 {
    mode & !umask() & 0o7777
}

/// The bits of `mode` the process's umask takes away but the request's
/// keeps: what a newly created file needs added back. 0 while the request
/// uses the process's umask.
fn lost_bits(mode: u32) -> u32 {
    match UMASK.get() {
        Some(m) => mode & !m & process_umask() & 0o777,
        None => 0,
    }
}

/// Whether files created by C libraries (which use the process's umask)
/// need [`library_created`]'s fix-up.
fn umask_differs() -> bool {
    UMASK.get().is_some_and(|m| m != process_umask())
}

/// Give a file just created with `mode` the permission bits the request's
/// umask allows (`fd` when open, else by path).
fn fix_created(fd: Option<libc::c_int>, path: &CStr, mode: u32) {
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        let r = match fd {
            Some(fd) => libc::fstat(fd, &mut st),
            None => libc::lstat(path.as_ptr(), &mut st),
        };
        if r != 0 || st.st_mode & libc::S_IFMT == libc::S_IFLNK {
            return;
        }
        let have = st.st_mode as u32 & 0o7777;
        let want = (have & 0o7000) | create_mode(mode) & 0o777;
        if want != have {
            match fd {
                Some(fd) => libc::fchmod(fd, want as libc::mode_t),
                None => libc::chmod(path.as_ptr(), want as libc::mode_t),
            };
        }
    }
}

/// `fd`, a file a C library call (`mkstemp`) just created with `mode`
/// under the process's umask, gets the request's instead.
pub fn created_fd(fd: libc::c_int, mode: u32) {
    if umask_differs() {
        fix_created(Some(fd), c"", mode);
    }
}

/// `open(2)` of `path` from the request's directory, creating a file with
/// `mode` under the request's umask.
pub fn open(path: &[u8], flags: libc::c_int, mode: u32) -> io::Result<OwnedFd> {
    with_at(path, |dir, c| open_at(dir, c, flags, mode)).unwrap_or_else(|| Err(nul_error()))
}

/// What std reports for a path holding a NUL byte.
fn nul_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte")
}

/// [`open`] of `c` from `dir`.
fn open_at(dir: libc::c_int, c: &CStr, flags: libc::c_int, mode: u32) -> io::Result<OwnedFd> {
    let sys = |flags: libc::c_int| {
        let mode = create_mode(mode) as libc::c_uint;
        let fd = unsafe { libc::openat(dir, c.as_ptr(), flags, mode) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { OwnedFd::from_raw_fd(fd) })
        }
    };
    let fixed = |fd: OwnedFd| {
        fix_created(Some(fd.as_raw_fd()), c, mode);
        fd
    };
    if flags & libc::O_CREAT == 0 || lost_bits(mode) == 0 {
        return sys(flags);
    }
    if flags & libc::O_EXCL != 0 {
        return sys(flags).map(fixed);
    }
    // Only a file this call creates gets its bits fixed: try creating it
    // exclusively first.
    match sys(flags | libc::O_EXCL) {
        Ok(fd) => Ok(fixed(fd)),
        Err(e) if e.raw_os_error() == Some(libc::EEXIST) => match sys(flags & !libc::O_CREAT) {
            // Gone again, or a dangling symlink: create it as asked. Only
            // a file created between the two calls would wrongly be fixed.
            Err(e) if e.raw_os_error() == Some(libc::ENOENT) => sys(flags).map(fixed),
            r => r,
        },
        Err(e) => Err(e),
    }
}

/// Write `data` to `path` (created or truncated, as `fopen($path, "wb")`).
pub fn write_file(path: &[u8], data: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC;
    std::fs::File::from(open(path, flags, 0o666)?).write_all(data)
}

/// Append `data` to `path`, creating it with `mode` (a log file).
pub fn append_file(path: &[u8], data: &[u8], mode: u32) -> io::Result<()> {
    use std::io::Write;
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND | libc::O_CLOEXEC;
    std::fs::File::from(open(path, flags, mode)?).write_all(data)
}

/// `mkdir(2)` of `path` from the request's directory, with `mode` under
/// the request's umask.
pub fn mkdir(path: &[u8], mode: u32) -> io::Result<()> {
    let c = c_path(path).ok_or_else(nul_error)?;
    if unsafe { libc::mkdir(c.as_ptr(), create_mode(mode) as libc::mode_t) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if lost_bits(mode) != 0 {
        fix_created(None, &c, mode);
    }
    Ok(())
}

/// Run `create`, a C library call that may create the file `path` with
/// `mode` under the process's umask (libxml2, libzip, OpenSSL, …), then
/// give a file it created the request's umask instead.
pub fn library_created<R>(path: &[u8], mode: u32, create: impl FnOnce() -> R) -> R {
    if !umask_differs() {
        return create();
    }
    let Some(c) = c_path(path) else {
        return create();
    };
    let existed = unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        libc::lstat(c.as_ptr(), &mut st) == 0
    };
    let r = create();
    if !existed {
        fix_created(None, &c, mode);
    }
    r
}

/// The umask a child process must start with when it isn't the process's.
pub fn child_umask() -> Option<u32> {
    UMASK.get().filter(|_| umask_differs())
}

/// The directory a child process must start in when it isn't the
/// process's.
pub fn child_cwd() -> Option<Vec<u8>> {
    CWD.with(|c| c.borrow().as_ref().map(|c| c.path.clone()))
}

#[cfg(test)]
mod tests;
