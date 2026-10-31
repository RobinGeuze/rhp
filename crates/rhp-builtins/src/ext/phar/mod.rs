//! The `phar` extension — enough of it to read and execute a native phar
//! (notably `composer.phar`): a native-format parser, SHA-256/512 signature
//! verification, a read-only `phar://` stream wrapper, and `Phar::mapPhar()`.
//!
//! Format (verified against composer.phar): `[stub] [manifest] [file data]
//! [signature]`, little-endian except the 2-byte API version. The stub is PHP
//! ending in `__HALT_COMPILER();`; the manifest begins just after. See
//! `docs/design/modules-and-streams.md` §3 for the exact field layout.
//!
//! Scope: READ-ONLY, native format only. Compressed entries use raw DEFLATE
//! (via `flate2`) or bzip2 (via the pure-Rust `bzip2` backend); tar/zip-based
//! phars are not yet supported. Writing, extraction, iteration, and the
//! compression APIs are deferred.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use flate2::read::DeflateDecoder;
use md5::Md5;
use rhp_values::{conv, Value};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

use crate::streams::{StatInfo, StreamWrapper, WrapperReg};
use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

// --- little-endian readers ---------------------------------------------------

fn u32le(b: &[u8], pos: usize) -> Option<u32> {
    b.get(pos..pos + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

// --- parsed archive ----------------------------------------------------------

struct Entry {
    offset: usize, // absolute offset of the data block in `data`
    csize: usize,
    usize_: usize,
    flags: u32,
    timestamp: u32,
}

/// A parsed, read-only native phar.
pub struct PharArchive {
    data: Vec<u8>,
    alias: Vec<u8>,
    /// Filesystem path the archive was loaded from (for canonical `phar://` URLs).
    path: Vec<u8>,
    entries: HashMap<Vec<u8>, Entry>,
}

const COMPRESSED_GZ: u32 = 0x1000;
const COMPRESSED_BZ2: u32 = 0x2000;
const SIG_PRESENT: u32 = 0x1_0000;

impl PharArchive {
    /// Parse a native phar from its full file bytes, verifying the signature.
    fn parse(data: Vec<u8>) -> Result<PharArchive, String> {
        // Manifest begins just after `__HALT_COMPILER` + optional `(); ?>` + newline.
        let manifest_start = halt_offset(&data).ok_or("no __HALT_COMPILER in stub")?;
        let mut pos = manifest_start;
        let manifest_len = u32le(&data, pos).ok_or("truncated manifest length")? as usize;
        pos += 4;
        let data_start = pos + manifest_len; // file data begins here
        let num_files = u32le(&data, pos).ok_or("truncated file count")? as usize;
        pos += 4;
        pos += 2; // API version (big-endian nibbles; not needed to read)
        let global_flags = u32le(&data, pos).ok_or("truncated global flags")?;
        pos += 4;
        let alias_len = u32le(&data, pos).ok_or("truncated alias length")? as usize;
        pos += 4;
        let alias = data
            .get(pos..pos + alias_len)
            .ok_or("truncated alias")?
            .to_vec();
        pos += alias_len;
        let meta_len = u32le(&data, pos).ok_or("truncated global metadata length")? as usize;
        pos += 4 + meta_len; // skip global metadata (serialize())

        if global_flags & SIG_PRESENT != 0 {
            verify_signature(&data)?;
        }

        let mut entries = HashMap::with_capacity(num_files);
        let mut cursor = data_start;
        for _ in 0..num_files {
            let name_len = u32le(&data, pos).ok_or("truncated entry name length")? as usize;
            pos += 4;
            let name = data
                .get(pos..pos + name_len)
                .ok_or("truncated entry name")?
                .to_vec();
            pos += name_len;
            let usize_ = u32le(&data, pos).ok_or("truncated usize")? as usize;
            pos += 4;
            let timestamp = u32le(&data, pos).ok_or("truncated mtime")?;
            pos += 4;
            let csize = u32le(&data, pos).ok_or("truncated csize")? as usize;
            pos += 4;
            pos += 4; // crc32
            let flags = u32le(&data, pos).ok_or("truncated entry flags")?;
            pos += 4;
            let em_len = u32le(&data, pos).ok_or("truncated entry metadata length")? as usize;
            pos += 4 + em_len; // skip per-file metadata

            // Directories (trailing '/') carry no data block.
            if !name.ends_with(b"/") {
                entries.insert(
                    name,
                    Entry {
                        offset: cursor,
                        csize,
                        usize_,
                        flags,
                        timestamp,
                    },
                );
                cursor += csize;
            }
        }

        Ok(PharArchive {
            data,
            alias,
            path: Vec::new(),
            entries,
        })
    }

    /// The raw (decompressed) bytes of an internal entry.
    fn read(&self, entry: &[u8]) -> Option<Vec<u8>> {
        let e = self.entries.get(entry)?;
        let raw = self.data.get(e.offset..e.offset + e.csize)?;
        if e.flags & COMPRESSED_GZ != 0 {
            use std::io::Read;
            let mut out = Vec::with_capacity(e.usize_);
            DeflateDecoder::new(raw).read_to_end(&mut out).ok()?;
            Some(out)
        } else if e.flags & COMPRESSED_BZ2 != 0 {
            use std::io::Read;
            let mut out = Vec::with_capacity(e.usize_);
            bzip2::read::BzDecoder::new(raw)
                .read_to_end(&mut out)
                .ok()?;
            Some(out)
        } else {
            Some(raw.to_vec())
        }
    }

    /// php `phar_dostat` for a file entry (archive opened read-only, so the
    /// write bits are masked off).
    #[allow(clippy::unnecessary_cast)]
    fn entry_stat(&self, entry: &[u8]) -> Option<StatInfo> {
        self.entries.get(entry).map(|e| {
            let perms = e.flags & 0o777;
            let mode = (perms & 0o555) | libc::S_IFREG as u32;
            StatInfo {
                is_regular: true,
                size: e.usize_ as u64,
                mode,
                mtime: e.timestamp as i64,
                atime: e.timestamp as i64,
                ctime: e.timestamp as i64,
                nlink: 1,
                rdev: -1,
                dev: 0xc,
                blksize: -1,
                blocks: -1,
                ..StatInfo::default()
            }
        })
    }

    /// php `phar_dostat` for a directory with no manifest entry of its own.
    #[allow(clippy::unnecessary_cast)]
    fn dir_stat(&self) -> StatInfo {
        let max = self
            .entries
            .values()
            .map(|e| e.timestamp)
            .max()
            .unwrap_or(0) as i64;
        StatInfo {
            is_dir: true,
            mode: 0o555 | libc::S_IFDIR as u32,
            mtime: max,
            atime: max,
            ctime: max,
            nlink: 1,
            rdev: -1,
            dev: 0xc,
            blksize: -1,
            blocks: -1,
            ..StatInfo::default()
        }
    }

    /// Immediate children of directory `prefix`, sorted (php sorts phar dir
    /// streams by binary name compare).
    fn list_dir(&self, prefix: &[u8]) -> Option<Vec<Vec<u8>>> {
        if !prefix.is_empty() && !self.has_dir(prefix) {
            return None;
        }
        let mut p = prefix.to_vec();
        if !p.is_empty() {
            p.push(b'/');
        }
        let mut names: Vec<Vec<u8>> = Vec::new();
        for k in self.entries.keys() {
            if let Some(rest) = k.strip_prefix(&p[..]) {
                let child = match rest.iter().position(|&b| b == b'/') {
                    Some(i) => &rest[..i],
                    None => rest,
                };
                if !child.is_empty() && !names.iter().any(|n| n == child) {
                    names.push(child.to_vec());
                }
            }
        }
        names.sort();
        Some(names)
    }

    fn has_dir(&self, prefix: &[u8]) -> bool {
        let mut p = prefix.to_vec();
        if !p.is_empty() && !p.ends_with(b"/") {
            p.push(b'/');
        }
        self.entries.keys().any(|k| k.starts_with(&p))
    }
}

/// Offset of the manifest: just after the `__HALT_COMPILER` token and its
/// optional `(); ?>` plus one trailing newline.
fn halt_offset(data: &[u8]) -> Option<usize> {
    let tok = b"__HALT_COMPILER";
    let at = data.windows(tok.len()).position(|w| w == tok)?;
    let mut i = at + tok.len();
    // skip "(" ")" ";" and surrounding whitespace
    while i < data.len() && matches!(data[i], b' ' | b'\t' | b'(' | b')') {
        i += 1;
    }
    if i < data.len() && data[i] == b';' {
        i += 1;
    }
    while i < data.len() && matches!(data[i], b' ' | b'\t') {
        i += 1;
    }
    if data.get(i..i + 2) == Some(b"?>") {
        i += 2;
    }
    // one trailing newline (\r\n, \n, or \r)
    if data.get(i) == Some(&b'\r') {
        i += 1;
    }
    if data.get(i) == Some(&b'\n') {
        i += 1;
    }
    Some(i)
}

/// Verify the trailing signature `[hash][type(4,LE)][GBMB]` over
/// `data[0 .. len-(hashlen+8)]`. Supports MD5, SHA-1, SHA-256 and SHA-512
/// (composer uses SHA-512), all verified byte-for-byte. OpenSSL (type 0x10)
/// signatures carry a variable-length RSA signature and need the archive's
/// public key (`<phar>.pubkey`); they remain accepted without verification.
fn verify_signature(data: &[u8]) -> Result<(), String> {
    let n = data.len();
    if n < 8 || &data[n - 4..] != b"GBMB" {
        return Err("missing GBMB signature trailer".into());
    }
    let sig_type = u32le(data, n - 8).ok_or("truncated signature type")?;
    // Each arm hashes `data[0 .. n-(hashlen+8)]` (everything before the stored
    // hash, the 4-byte type, and the `GBMB` magic) and compares to the stored hash.
    let (hashlen, computed): (usize, Option<Vec<u8>>) = match sig_type {
        0x0004 => {
            let body = &data[..n - 72];
            (64, Some(Sha512::digest(body).to_vec()))
        }
        0x0003 => {
            let body = &data[..n - 40];
            (32, Some(Sha256::digest(body).to_vec()))
        }
        0x0002 => {
            let body = &data[..n - 28];
            (20, Some(Sha1::digest(body).to_vec()))
        }
        0x0001 => {
            let body = &data[..n - 24];
            (16, Some(Md5::digest(body).to_vec()))
        }
        0x0010 => return Ok(()), // OpenSSL: variable length; accept (needs pubkey + RSA)
        other => return Err(format!("unknown phar signature type {other}")),
    };
    if let Some(expected) = computed {
        let stored = data
            .get(n - 8 - hashlen..n - 8)
            .ok_or("truncated signature hash")?;
        if stored != expected.as_slice() {
            return Err("phar signature verification failed".into());
        }
    }
    Ok(())
}

// --- the phar:// wrapper -----------------------------------------------------

/// `phar://` wrapper. Holds the archives opened via `Phar::mapPhar` /
/// `Phar::loadPhar`, keyed by alias and by filesystem path.
pub struct PharWrapper {
    archives: RefCell<HashMap<Vec<u8>, Rc<PharArchive>>>,
}

impl PharWrapper {
    fn new() -> PharWrapper {
        PharWrapper {
            archives: RefCell::new(HashMap::new()),
        }
    }

    /// Register a parsed archive under its filesystem path, its manifest alias,
    /// and an explicit alias (from `mapPhar`), so all `phar://` forms resolve.
    fn install(&self, path: &[u8], explicit_alias: Option<&[u8]>, arc: Rc<PharArchive>) {
        let mut m = self.archives.borrow_mut();
        m.insert(path.to_vec(), arc.clone());
        if !arc.alias.is_empty() {
            m.insert(arc.alias.clone(), arc.clone());
        }
        if let Some(a) = explicit_alias {
            if !a.is_empty() {
                m.insert(a.to_vec(), arc.clone());
            }
        }
    }

    fn map_phar(&self, path: &[u8], alias: Option<&[u8]>, bytes: Vec<u8>) -> Result<(), String> {
        let mut arc = PharArchive::parse(bytes)?;
        arc.path = path.to_vec();
        self.install(path, alias, Rc::new(arc));
        Ok(())
    }

    /// The canonical `phar://<real-path>/<entry>` URL for `url` (php reports the
    /// archive's real path in `__FILE__`, even when accessed via its alias).
    fn canonical_url(&self, url: &[u8]) -> Option<Vec<u8>> {
        let (arc, entry) = self.resolve(url)?;
        let mut out = b"phar://".to_vec();
        out.extend_from_slice(&arc.path);
        if !entry.is_empty() {
            out.push(b'/');
            out.extend_from_slice(&entry);
        }
        Some(out)
    }

    /// Resolve `phar://<archive>/<entry>` to the archive and the normalized
    /// entry path, by longest registered-key prefix match.
    fn resolve(&self, url: &[u8]) -> Option<(Rc<PharArchive>, Vec<u8>)> {
        let rest = url.strip_prefix(b"phar://")?;
        let m = self.archives.borrow();
        let mut best: Option<(&Vec<u8>, usize)> = None;
        for key in m.keys() {
            let matches = rest == key.as_slice()
                || (rest.len() > key.len() && rest.starts_with(key) && rest[key.len()] == b'/');
            if matches && best.map(|(_, l)| key.len() > l).unwrap_or(true) {
                best = Some((key, key.len()));
            }
        }
        let (key, klen) = best?;
        let arc = m.get(key)?.clone();
        let entry = if rest.len() > klen {
            &rest[klen + 1..]
        } else {
            b""
        };
        Some((arc, normalize(entry)))
    }
}

/// Collapse `.` and `..` segments in a phar-internal entry path.
fn normalize(entry: &[u8]) -> Vec<u8> {
    let mut out: Vec<&[u8]> = Vec::new();
    for seg in entry.split(|&b| b == b'/') {
        match seg {
            b"" | b"." => {}
            b".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.join(&b"/"[..])
}

impl StreamWrapper for PharWrapper {
    fn exists(&self, url: &[u8]) -> bool {
        match self.resolve(url) {
            Some((arc, entry)) => arc.entries.contains_key(&entry) || arc.has_dir(&entry),
            None => false,
        }
    }
    fn get_contents(&self, url: &[u8]) -> Option<Vec<u8>> {
        let (arc, entry) = self.resolve(url)?;
        arc.read(&entry)
    }
    fn stat(&self, url: &[u8]) -> Option<StatInfo> {
        let (arc, entry) = self.resolve(url)?;
        if let Some(s) = arc.entry_stat(&entry) {
            Some(s)
        } else if entry.is_empty() || arc.has_dir(&entry) {
            Some(arc.dir_stat())
        } else {
            None
        }
    }
    fn opendir(&self, url: &[u8]) -> Option<Vec<Vec<u8>>> {
        let (arc, entry) = self.resolve(url)?;
        arc.list_dir(&entry)
    }
    fn canonical(&self, url: &[u8]) -> Option<Vec<u8>> {
        self.canonical_url(url)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Run `f` with the concrete `PharWrapper` behind the registered `phar://` entry.
fn with_phar_wrapper<R>(
    host: &dyn Host,
    f: impl FnOnce(&PharWrapper) -> Result<R, Thrown>,
) -> Result<R, Thrown> {
    let w = host
        .stream_wrapper(b"phar")
        .ok_or_else(|| Thrown::error("PharException", "phar:// wrapper is not registered"))?;
    let pw = w
        .as_any()
        .downcast_ref::<PharWrapper>()
        .ok_or_else(|| Thrown::error("PharException", "phar:// wrapper has the wrong type"))?;
    f(pw)
}

// --- native builtins behind the Phar class ----------------------------------

/// `Phar::mapPhar($alias)`: parse the currently-running phar (the top-level
/// script) and register `$alias` → the archive.
fn fn_phar_mapphar(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let alias = args
        .first()
        .map(|v| conv::to_string_bytes(&deref(v)))
        .filter(|a| !a.is_empty());
    let path = host
        .running_main_path()
        .ok_or_else(|| Thrown::error("PharException", "cannot determine the running phar"))?;
    let bytes = host
        .stream_get_contents(&path)
        .ok_or_else(|| Thrown::error("PharException", "cannot read the running phar"))?;
    with_phar_wrapper(host, |pw| {
        pw.map_phar(&path, alias.as_deref(), bytes)
            .map_err(|e| Thrown::error("PharException", &e))
    })?;
    Ok(Value::Null)
}

/// `Phar::loadPhar($filename, $alias)`: load an external phar by path.
fn fn_phar_loadphar(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let filename = conv::to_string_bytes(&deref(&args[0]));
    let alias = args
        .get(1)
        .map(|v| conv::to_string_bytes(&deref(v)))
        .filter(|a| !a.is_empty());
    let bytes = host
        .stream_get_contents(&filename)
        .ok_or_else(|| Thrown::error("PharException", "cannot read phar file"))?;
    with_phar_wrapper(host, |pw| {
        pw.map_phar(&filename, alias.as_deref(), bytes)
            .map_err(|e| Thrown::error("PharException", &e))
    })?;
    Ok(Value::Bool(true))
}

/// `Phar::running($retphar)`: the URL (or path) of the running phar. Used only by
/// composer's self-update; returns "" when not running inside a phar.
fn fn_phar_running(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let retphar = args
        .first()
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(true);
    match host.running_main_path() {
        Some(p) => {
            if retphar {
                let mut s = b"phar://".to_vec();
                s.extend_from_slice(&p);
                Ok(Value::str(s))
            } else {
                Ok(Value::str(p))
            }
        }
        None => Ok(Value::str("")),
    }
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    }
    put(m, "__phar_mapphar", fn_phar_mapphar, 0, Some(2));
    put(m, "__phar_loadphar", fn_phar_loadphar, 1, Some(2));
    put(m, "__phar_running", fn_phar_running, 0, Some(1));
}

pub fn wrappers() -> Vec<WrapperReg> {
    vec![WrapperReg {
        scheme: "phar",
        factory: || Rc::new(PharWrapper::new()),
    }]
}

/// The `Phar` class (read-only subset) + `PharException`. Static methods delegate
/// to the module's native `__phar_*` builtins (the GMP/DateTime idiom).
pub const PRELUDE_PHP: &str = r#"
class PharException extends Exception {}
class Phar {
    const NONE = 0;
    const COMPRESSED = 61440;
    const GZ = 4096;
    const BZ2 = 8192;
    const SAME = 0;
    const PHAR = 1;
    const TAR = 2;
    const ZIP = 3;
    const MD5 = 1;
    const SHA1 = 2;
    const SHA256 = 3;
    const SHA512 = 4;
    const OPENSSL = 16;
    public static function mapPhar($alias = null, $dataoffset = 0) { return __phar_mapphar($alias); }
    public static function loadPhar($filename, $alias = null) { return __phar_loadphar($filename, $alias); }
    public static function running($retphar = true) { return __phar_running($retphar); }
    public static function canCompress($type = 0) { return false; }
    public static function isValidPharFilename($filename, $executable = true) { return true; }
    public static function interceptFileFuncs() {}
    public static function createDefaultStub($index = null, $webindex = null) { return ""; }
}
"#;
