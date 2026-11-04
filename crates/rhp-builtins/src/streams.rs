//! Stream-wrapper layer. A path either names a wrapper via a `scheme://` prefix
//! or (with no scheme) uses the default `file://` wrapper. Native wrappers
//! implement [`StreamWrapper`]; the VM holds a per-run registry (runtime-mutable
//! via `stream_wrapper_register`, later) and routes file I/O + include/require
//! through it. Builtins reach the registry through `Host::stream_*`.
//!
//! Stage 1 (this file) is path-based only — enough for `include`/`require`,
//! `file_get_contents`, and `is_file`/`is_dir` over `file://` and `phar://`.
//! Resource-backed streams (`fopen`/`fread`/`gzopen`) are a later stage.

use std::any::Any;
use std::path::Path;
use std::rc::Rc;

/// A stat result for a stream URL: the convenience flags plus every field of
/// php's `stat()` array (`dev` … `blocks`).
#[derive(Clone, Default, Debug)]
pub struct StatInfo {
    pub is_dir: bool,
    /// A *regular* file (php `is_file`): false for directories and for special
    /// files such as `/dev/null` (character devices), sockets, fifos.
    pub is_regular: bool,
    pub size: u64,
    pub mtime: i64,
    pub dev: i64,
    pub ino: i64,
    /// Full `st_mode` (type bits + permission bits).
    pub mode: u32,
    pub nlink: i64,
    pub uid: i64,
    pub gid: i64,
    pub rdev: i64,
    pub atime: i64,
    pub ctime: i64,
    pub blksize: i64,
    pub blocks: i64,
}

impl StatInfo {
    /// Build from a raw `libc::stat`.
    // `as` casts: field widths differ across platforms (macOS vs Linux).
    #[allow(clippy::unnecessary_cast)]
    pub fn from_libc(st: &libc::stat) -> StatInfo {
        let mode = st.st_mode as u32;
        let fmt = mode & libc::S_IFMT as u32;
        StatInfo {
            is_dir: fmt == libc::S_IFDIR as u32,
            is_regular: fmt == libc::S_IFREG as u32,
            size: st.st_size as u64,
            mtime: st.st_mtime as i64,
            dev: st.st_dev as i64,
            ino: st.st_ino as i64,
            mode,
            nlink: st.st_nlink as i64,
            uid: st.st_uid as i64,
            gid: st.st_gid as i64,
            rdev: st.st_rdev as i64,
            atime: st.st_atime as i64,
            ctime: st.st_ctime as i64,
            blksize: st.st_blksize as i64,
            blocks: st.st_blocks as i64,
        }
    }

    /// Is this a symlink (only meaningful for an `lstat` result)?
    #[allow(clippy::unnecessary_cast)]
    pub fn is_link(&self) -> bool {
        self.mode & libc::S_IFMT as u32 == libc::S_IFLNK as u32
    }
}

// --- the errno of the last failed local-filesystem operation ---------------
// php builds its warnings ("Failed to open stream: No such file or
// directory", "stat failed for …") from errno right at the failing syscall;
// the wrappers record it here so the builtin that sees the failure can.

thread_local! {
    static LAST_OS_ERR: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
}

/// Record the errno of a failed syscall (`errno` read now).
pub fn note_errno() {
    let e = std::io::Error::last_os_error();
    LAST_OS_ERR.with(|c| c.set(e.raw_os_error()));
}

/// Record the errno carried by an `io::Error`.
pub fn note_io_err(e: &std::io::Error) {
    LAST_OS_ERR.with(|c| c.set(e.raw_os_error()));
}

/// Forget any recorded errno (before an operation whose failure is reported).
pub fn clear_errno() {
    LAST_OS_ERR.with(|c| c.set(None));
}

/// The recorded errno, if the last failure recorded one.
pub fn last_errno() -> Option<i32> {
    LAST_OS_ERR.with(|c| c.get())
}

/// How many resource ids php's open of `url` registers: a compress.zlib://
/// stream wraps an inner plain-file stream.
pub fn open_resource_ids(url: &[u8]) -> u32 {
    if split_scheme(url).0 == b"compress.zlib" {
        2
    } else {
        1
    }
}

/// `strerror(code)`.
pub fn strerror(code: i32) -> String {
    unsafe {
        let p = libc::strerror(code);
        if p.is_null() {
            return format!("Unknown error {code}");
        }
        std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

/// `strerror` of the recorded errno; php's generic text when none was.
pub fn last_strerror() -> String {
    match last_errno() {
        Some(c) => strerror(c),
        None => "operation failed".to_string(),
    }
}

/// `stat(2)` (or `lstat(2)` when `link`) a local path.
pub fn sys_stat(path: &[u8], link: bool) -> Option<StatInfo> {
    let c = std::ffi::CString::new(path.to_vec()).ok()?;
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        let r = if link {
            libc::lstat(c.as_ptr(), &mut st)
        } else {
            libc::stat(c.as_ptr(), &mut st)
        };
        if r == 0 {
            Some(StatInfo::from_libc(&st))
        } else {
            note_errno();
            None
        }
    }
}

/// `fstat(2)` an open descriptor.
pub fn sys_fstat(fd: i32) -> Option<StatInfo> {
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        if libc::fstat(fd, &mut st) == 0 {
            Some(StatInfo::from_libc(&st))
        } else {
            None
        }
    }
}

/// Read every entry of a local directory in OS `readdir(3)` order, including
/// `.` and `..` (exactly what php's plain-files dir stream yields).
pub fn sys_readdir(path: &[u8]) -> Option<Vec<Vec<u8>>> {
    let c = std::ffi::CString::new(path.to_vec()).ok()?;
    let mut out = Vec::new();
    unsafe {
        let dir = libc::opendir(c.as_ptr());
        if dir.is_null() {
            note_errno();
            return None;
        }
        loop {
            let ent = libc::readdir(dir);
            if ent.is_null() {
                break;
            }
            let name = std::ffi::CStr::from_ptr((*ent).d_name.as_ptr());
            out.push(name.to_bytes().to_vec());
        }
        libc::closedir(dir);
    }
    Some(out)
}

/// A native stream wrapper, keyed by scheme (`file`, `phar`, …).
pub trait StreamWrapper: Any {
    /// Does the URL name an existing file or directory?
    fn exists(&self, url: &[u8]) -> bool;
    /// Whole-file read; `None` if missing / not readable.
    fn get_contents(&self, url: &[u8]) -> Option<Vec<u8>>;
    /// Stat the URL; `None` if missing.
    fn stat(&self, url: &[u8]) -> Option<StatInfo>;
    /// Stat without following a final symlink (`lstat`). Default: `stat`.
    fn lstat(&self, url: &[u8]) -> Option<StatInfo> {
        self.stat(url)
    }
    /// List a directory's entries in the order php's dir stream yields them;
    /// `None` if the URL is not an openable directory.
    fn opendir(&self, _url: &[u8]) -> Option<Vec<Vec<u8>>> {
        None
    }
    /// Whole-file write (writable wrappers only); returns bytes written.
    fn put_contents(&self, _url: &[u8], _data: &[u8]) -> Option<usize> {
        None
    }
    /// The canonical URL for `url` (used as `__FILE__`/`__DIR__` and the `_once`
    /// key for includes). Default: none (caller keeps the URL as given). The
    /// `phar://` wrapper rewrites an alias to the archive's real path, matching
    /// php.
    fn canonical(&self, _url: &[u8]) -> Option<Vec<u8>> {
        None
    }
    /// Whether [`StreamWrapper::open_stream`] is the wrapper's opener, so
    /// whole-file readers open a stream (with the call's context) instead
    /// of [`StreamWrapper::get_contents`].
    fn has_stream_opener(&self) -> bool {
        false
    }
    /// php's `wops->stream_opener` for a wrapper whose streams run on their
    /// own ops (zip entries) instead of a whole-file buffer: `None` when the
    /// wrapper has none, else the stream or the failure. `options` are the
    /// stream context's options; diagnostics go to `diags`.
    fn open_stream(
        &self,
        _url: &[u8],
        _mode: &[u8],
        _options: &rhp_values::Value,
        _diags: &mut Vec<(i64, String)>,
    ) -> Option<Option<crate::opsstream::OpsStream>> {
        None
    }
    /// For wrapper-specific control ops (e.g. `Phar::mapPhar` reaching its own
    /// archive table). Implementors return `self`.
    fn as_any(&self) -> &dyn Any;
}

/// A module's built-in wrapper registration: scheme + a per-run factory.
pub struct WrapperReg {
    pub scheme: &'static str,
    pub factory: fn() -> Rc<dyn StreamWrapper>,
}

/// Split a path into `(scheme, rest)`. A scheme is a leading
/// `[A-Za-z][A-Za-z0-9+.-]*` immediately followed by `://`; with none, the
/// scheme is `b"file"` and `rest` is the whole path.
pub fn split_scheme(path: &[u8]) -> (&[u8], &[u8]) {
    // find "://"
    if let Some(pos) = find(path, b"://") {
        let scheme = &path[..pos];
        if !scheme.is_empty()
            && scheme[0].is_ascii_alphabetic()
            && scheme
                .iter()
                .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'))
        {
            return (scheme, path);
        }
    }
    (b"file", path)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

// --- the default `file://` wrapper ------------------------------------------

/// Strip a leading `file://` (keeping an absolute leading slash) for the local
/// filesystem path.
pub fn file_path(url: &[u8]) -> &[u8] {
    url.strip_prefix(b"file://").unwrap_or(url)
}

/// The default local-filesystem wrapper.
pub struct FileWrapper;

impl StreamWrapper for FileWrapper {
    fn exists(&self, url: &[u8]) -> bool {
        Path::new(&*String::from_utf8_lossy(file_path(url))).exists()
    }
    fn get_contents(&self, url: &[u8]) -> Option<Vec<u8>> {
        std::fs::read(&*String::from_utf8_lossy(file_path(url)))
            .map_err(|e| note_io_err(&e))
            .ok()
    }
    fn stat(&self, url: &[u8]) -> Option<StatInfo> {
        sys_stat(file_path(url), false)
    }
    fn lstat(&self, url: &[u8]) -> Option<StatInfo> {
        sys_stat(file_path(url), true)
    }
    fn opendir(&self, url: &[u8]) -> Option<Vec<Vec<u8>>> {
        sys_readdir(file_path(url))
    }
    fn put_contents(&self, url: &[u8], data: &[u8]) -> Option<usize> {
        std::fs::write(&*String::from_utf8_lossy(file_path(url)), data)
            .map_err(|e| note_io_err(&e))
            .ok()
            .map(|_| data.len())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A fresh `file://` wrapper instance.
pub fn file_wrapper() -> Rc<dyn StreamWrapper> {
    Rc::new(FileWrapper)
}

// --- plain-file defaults -----------------------------------------------------
// These implement only `file://` via std::fs; any other scheme fails. They back
// the `Host` stream defaults (hosts without a wrapper registry) and the inner
// file access of the compress.zlib:// and compress.bzip2:// wrappers.

pub fn default_get_contents(url: &[u8]) -> Option<Vec<u8>> {
    let (scheme, _) = split_scheme(url);
    if scheme == b"file" {
        FileWrapper.get_contents(url)
    } else {
        None
    }
}

pub fn default_exists(url: &[u8]) -> bool {
    let (scheme, _) = split_scheme(url);
    scheme == b"file" && FileWrapper.exists(url)
}

pub fn default_stat(url: &[u8]) -> Option<StatInfo> {
    let (scheme, _) = split_scheme(url);
    if scheme == b"file" {
        FileWrapper.stat(url)
    } else {
        None
    }
}

pub fn default_put_contents(url: &[u8], data: &[u8]) -> Option<usize> {
    let (scheme, _) = split_scheme(url);
    if scheme == b"file" {
        FileWrapper.put_contents(url, data)
    } else {
        None
    }
}

// --- resource-backed local files (fopen on the plain-files wrapper) ----------

/// php's plain-files stream read chunk size.
const CHUNK: usize = 8192;

/// An open local file behind an `fopen`ed stream resource, with php's
/// plain-files semantics: a read buffer over the descriptor, a logical
/// position (`ftell`), and an eof flag that is set only when an underlying
/// `read(2)` returns 0.
pub struct LocalFile {
    file: std::fs::File,
    rbuf: Vec<u8>,
    rpos: usize,
    pos: i64,
    eof: bool,
    readable: bool,
    writable: bool,
    /// The mode string as passed to `fopen` (for `stream_get_meta_data`).
    pub mode: Vec<u8>,
    /// The URI as passed to `fopen`.
    pub uri: Vec<u8>,
}

/// Parse an fopen mode into `(open_flags, readable, writable)` exactly as
/// php's `php_stream_parse_fopen_modes`; `None` for an unknown mode.
pub fn parse_fopen_mode(mode: &[u8]) -> Option<(libc::c_int, bool, bool)> {
    let mut flags = match mode.first()? {
        b'r' => 0,
        b'w' => libc::O_TRUNC | libc::O_CREAT,
        b'a' => libc::O_CREAT | libc::O_APPEND,
        b'x' => libc::O_CREAT | libc::O_EXCL,
        b'c' => libc::O_CREAT,
        _ => return None,
    };
    let (r, w);
    if mode.contains(&b'+') {
        flags |= libc::O_RDWR;
        (r, w) = (true, true);
    } else if flags != 0 {
        flags |= libc::O_WRONLY;
        (r, w) = (false, true);
    } else {
        flags |= libc::O_RDONLY;
        (r, w) = (true, false);
    }
    if mode.contains(&b'e') {
        flags |= libc::O_CLOEXEC;
    }
    if mode.contains(&b'n') {
        flags |= libc::O_NONBLOCK;
    }
    Some((flags, r, w))
}

impl LocalFile {
    /// `open(2)` a local path with php's fopen-mode rules (perms 0666 & ~umask).
    pub fn open(path: &[u8], mode: &[u8]) -> Option<LocalFile> {
        use std::os::unix::io::FromRawFd;
        let (flags, readable, writable) = parse_fopen_mode(mode)?;
        let c = std::ffi::CString::new(file_path(path).to_vec()).ok()?;
        let fd = unsafe { libc::open(c.as_ptr(), flags, 0o666 as libc::c_uint) };
        if fd < 0 {
            note_errno();
            return None;
        }
        Some(LocalFile {
            file: unsafe { std::fs::File::from_raw_fd(fd) },
            rbuf: Vec::new(),
            rpos: 0,
            pos: 0,
            eof: false,
            readable,
            writable,
            mode: mode.to_vec(),
            uri: path.to_vec(),
        })
    }

    pub fn fd(&self) -> i32 {
        use std::os::unix::io::AsRawFd;
        self.file.as_raw_fd()
    }

    /// Pull one chunk into the read buffer; sets eof when `read(2)` returns 0
    /// (or fails). Returns the number of bytes added.
    fn fill(&mut self) -> usize {
        use std::io::Read;
        if self.rpos >= self.rbuf.len() {
            self.rbuf.clear();
            self.rpos = 0;
        }
        let mut chunk = [0u8; CHUNK];
        loop {
            match self.file.read(&mut chunk) {
                Ok(0) => {
                    self.eof = true;
                    return 0;
                }
                Ok(n) => {
                    self.rbuf.extend_from_slice(&chunk[..n]);
                    return n;
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    self.eof = true;
                    return 0;
                }
            }
        }
    }

    fn take(&mut self, n: usize) -> Vec<u8> {
        let n = n.min(self.rbuf.len() - self.rpos);
        let out = self.rbuf[self.rpos..self.rpos + n].to_vec();
        self.rpos += n;
        self.pos += n as i64;
        out
    }

    /// `fread`: greedy (plain files keep reading until `len` bytes or eof).
    /// `None` when the stream is not readable.
    pub fn read(&mut self, len: usize) -> Option<Vec<u8>> {
        if !self.readable {
            return None;
        }
        let mut out = Vec::new();
        while out.len() < len {
            if self.rpos >= self.rbuf.len() && self.fill() == 0 {
                break;
            }
            let more = self.take(len - out.len());
            out.extend_from_slice(&more);
        }
        Some(out)
    }

    /// Read to eof (`stream_get_contents` with no limit).
    pub fn read_all(&mut self) -> Option<Vec<u8>> {
        self.read(usize::MAX)
    }

    /// `fgets`: up to and including the next "\n", at most `max` bytes (when
    /// given); `None` at eof with nothing read.
    pub fn gets(&mut self, max: Option<usize>) -> Option<Vec<u8>> {
        if !self.readable {
            return None;
        }
        let mut out = Vec::new();
        loop {
            if let Some(m) = max {
                if out.len() >= m {
                    break;
                }
            }
            if self.rpos >= self.rbuf.len() && self.fill() == 0 {
                break;
            }
            let avail = &self.rbuf[self.rpos..];
            let mut n = match avail.iter().position(|&b| b == b'\n') {
                Some(i) => i + 1,
                None => avail.len(),
            };
            if let Some(m) = max {
                n = n.min(m - out.len());
            }
            let found_nl = avail[..n].last() == Some(&b'\n');
            let more = self.take(n);
            out.extend_from_slice(&more);
            if found_nl {
                break;
            }
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    /// Drop any read-ahead, re-syncing the descriptor to the logical position.
    fn sync(&mut self) {
        if self.rpos < self.rbuf.len() {
            unsafe {
                libc::lseek(self.fd(), self.pos as libc::off_t, libc::SEEK_SET);
            }
        }
        self.rbuf.clear();
        self.rpos = 0;
    }

    /// `fwrite`: `None` when not writable / the write fails.
    pub fn write(&mut self, data: &[u8]) -> Option<usize> {
        use std::io::Write;
        if !self.writable {
            return None;
        }
        self.sync();
        if data.is_empty() {
            return Some(0);
        }
        match self.file.write_all(data) {
            Ok(()) => {
                self.pos += data.len() as i64;
                Some(data.len())
            }
            Err(_) => None,
        }
    }

    /// `fseek`: true on success (php returns 0 / -1). Bug-compatible with
    /// php: the read buffer is discarded even when the seek fails, without
    /// re-syncing the descriptor, so a later read continues from wherever the
    /// descriptor was (the end of the buffered data) while `ftell` keeps the
    /// old position.
    pub fn seek(&mut self, offset: i64, whence: i32) -> bool {
        let (off, wh) = match whence {
            libc::SEEK_CUR => (self.pos + offset, libc::SEEK_SET),
            w => (offset, w),
        };
        self.rbuf.clear();
        self.rpos = 0;
        let r = unsafe { libc::lseek(self.fd(), off as libc::off_t, wh) };
        if r < 0 {
            return false;
        }
        self.pos = r as i64;
        self.eof = false;
        true
    }

    pub fn tell(&self) -> i64 {
        self.pos
    }

    pub fn eof(&self) -> bool {
        self.eof
    }

    pub fn rewind(&mut self) -> bool {
        self.seek(0, libc::SEEK_SET)
    }

    /// `ftruncate`: does not move the position.
    pub fn truncate(&mut self, size: i64) -> bool {
        if !self.writable {
            return false;
        }
        self.sync();
        unsafe { libc::ftruncate(self.fd(), size as libc::off_t) == 0 }
    }

    pub fn stat(&self) -> Option<StatInfo> {
        sys_fstat(self.fd())
    }

    /// Bytes buffered but not yet consumed (`unread_bytes`).
    pub fn unread(&self) -> usize {
        self.rbuf.len() - self.rpos
    }

    /// php's `php_stream_cast(PHP_STREAM_AS_FD)` before handing the
    /// descriptor to another process (`proc_open`): read-ahead is dropped and
    /// the descriptor moved back to the logical position.
    pub fn sync_for_cast(&mut self) {
        self.sync();
    }
}
