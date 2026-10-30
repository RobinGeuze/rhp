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

/// Minimal stat for a stream URL (what `is_file`/`is_dir`/`filesize` need).
pub struct StatInfo {
    pub is_dir: bool,
    /// A *regular* file (php `is_file`): false for directories and for special
    /// files such as `/dev/null` (character devices), sockets, fifos.
    pub is_regular: bool,
    pub size: u64,
    pub mtime: i64,
}

/// A native stream wrapper, keyed by scheme (`file`, `phar`, …).
pub trait StreamWrapper: Any {
    /// Does the URL name an existing file or directory?
    fn exists(&self, url: &[u8]) -> bool;
    /// Whole-file read; `None` if missing / not readable.
    fn get_contents(&self, url: &[u8]) -> Option<Vec<u8>>;
    /// Stat the URL; `None` if missing.
    fn stat(&self, url: &[u8]) -> Option<StatInfo>;
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
fn file_path(url: &[u8]) -> &[u8] {
    url.strip_prefix(b"file://").unwrap_or(url)
}

/// The default local-filesystem wrapper.
pub struct FileWrapper;

impl StreamWrapper for FileWrapper {
    fn exists(&self, url: &[u8]) -> bool {
        Path::new(&*String::from_utf8_lossy(file_path(url))).exists()
    }
    fn get_contents(&self, url: &[u8]) -> Option<Vec<u8>> {
        std::fs::read(&*String::from_utf8_lossy(file_path(url))).ok()
    }
    fn stat(&self, url: &[u8]) -> Option<StatInfo> {
        let md = std::fs::metadata(&*String::from_utf8_lossy(file_path(url))).ok()?;
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Some(StatInfo {
            is_dir: md.is_dir(),
            is_regular: md.is_file(),
            size: md.len(),
            mtime,
        })
    }
    fn put_contents(&self, url: &[u8], data: &[u8]) -> Option<usize> {
        std::fs::write(&*String::from_utf8_lossy(file_path(url)), data)
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

// --- defaults for hosts without a registry (the oracle) ----------------------
// These implement only `file://` via std::fs, so the tree-walker keeps working
// on plain local paths; any non-file scheme is unsupported there.

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
