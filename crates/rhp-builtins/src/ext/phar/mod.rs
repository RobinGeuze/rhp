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
//! (via `flate2`) or bzip2 (via the `bzip2` crate over C libbz2); tar/zip-based
//! phars are not yet supported. Writing, extraction, iteration, and the
//! compression APIs are deferred.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::rc::Rc;

use flate2::read::DeflateDecoder;
use md5::Md5;
use rhp_values::{conv, Value};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

mod tar;

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
    fn parse(data: Vec<u8>, path: &[u8]) -> Result<PharArchive, String> {
        // Manifest begins just after `__HALT_COMPILER` + optional `(); ?>` + newline.
        let manifest_start = halt_offset(&data).ok_or_else(|| {
            format!(
                "internal corruption of phar \"{}\" (__HALT_COMPILER(); not found)",
                String::from_utf8_lossy(path)
            )
        })?;
        let mut pos = manifest_start;
        // phar_parse_pharfile's header checks, in its order and wording.
        let corrupt = |what: &str| {
            format!(
                "internal corruption of phar \"{}\" ({what})",
                String::from_utf8_lossy(path)
            )
        };
        let manifest_len = u32le(&data, pos)
            .ok_or_else(|| corrupt("truncated manifest at manifest length"))?
            as usize;
        if manifest_len > 1048576 * 100 {
            return Err(format!(
                "manifest cannot be larger than 100 MB in phar \"{}\"",
                String::from_utf8_lossy(path)
            ));
        }
        pos += 4;
        // MANIFEST_FIXED_LEN: count, API version, flags, alias length, metadata length.
        if manifest_len < 18 || data.len() < pos + manifest_len {
            return Err(corrupt("truncated manifest header"));
        }
        let data_start = pos + manifest_len; // file data begins here
        let num_files = u32le(&data, pos).ok_or("truncated file count")? as usize;
        if num_files == 0 {
            return Err(format!(
                "in phar \"{}\", manifest claims to have zero entries.  Phars must have at least 1 entry",
                String::from_utf8_lossy(path)
            ));
        }
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
            verify_signature(&data, path)?;
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

    /// php's `phar_set_inode`: the low 16 bits of zend's DJB string hash of
    /// archive path + entry name (capped at MAXPATHLEN bytes) — bug for bug:
    /// php `memcpy`s from the entry's `zend_string *` itself rather than its
    /// value, so the bytes hashed after the path are the string header
    /// (refcount=1, type_info=IS_STRING|NOT_COLLECTABLE, hash=0, len) followed
    /// by the name, truncated to the name's length.
    fn entry_inode(&self, entry: &[u8]) -> i64 {
        let mut hdr: Vec<u8> = Vec::with_capacity(24 + entry.len());
        hdr.extend_from_slice(&1u32.to_le_bytes());
        hdr.extend_from_slice(&0x16u32.to_le_bytes());
        hdr.extend_from_slice(&0u64.to_le_bytes());
        hdr.extend_from_slice(&(entry.len() as u64).to_le_bytes());
        hdr.extend_from_slice(entry);
        let mut tmp: Vec<u8> = self.path.clone();
        tmp.truncate(1024);
        let room = 1024 - tmp.len();
        tmp.extend_from_slice(&hdr[..entry.len().min(room)]);
        let mut h: u64 = 5381;
        for &b in &tmp {
            // zend hashes `char`, which is signed on x86-64 (unsigned on arm64).
            let v = if cfg!(target_arch = "x86_64") {
                b as i8 as i64 as u64
            } else {
                b as u64
            };
            h = h.wrapping_mul(33).wrapping_add(v);
        }
        (h & 0xffff) as i64
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
                ino: self.entry_inode(entry),
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
/// (composer uses SHA-512), all verified byte-for-byte. OpenSSL signatures
/// (types 0x10/0x11/0x12: `[sig][siglen(4,LE)][type][GBMB]`, RSA over SHA-1 /
/// SHA-256 / SHA-512) are verified against `<phar path>.pubkey`, as php does.
fn verify_signature(data: &[u8], path: &[u8]) -> Result<(), String> {
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
        0x0010..=0x0012 => return verify_openssl_signature(data, sig_type, path),
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

/// The OpenSSL signature arms of [`verify_signature`] (php-src
/// `phar_verify_signature`): RSA verify of everything before the signature
/// with the PEM public key in `<path>.pubkey`.
fn verify_openssl_signature(data: &[u8], sig_type: u32, path: &[u8]) -> Result<(), String> {
    let n = data.len();
    let fname = String::from_utf8_lossy(path).into_owned();
    let len_at = n
        .checked_sub(12)
        .ok_or_else(|| format!("phar \"{fname}\" openssl signature length could not be read"))?;
    let sig_len = u32le(data, len_at)
        .ok_or_else(|| format!("phar \"{fname}\" openssl signature length could not be read"))?
        as usize;
    let sig_start = len_at
        .checked_sub(sig_len)
        .ok_or_else(|| format!("phar \"{fname}\" openssl signature could not be read"))?;
    let sig = &data[sig_start..len_at];
    let body = &data[..sig_start];
    let mut pk_path = path.to_vec();
    pk_path.extend_from_slice(b".pubkey");
    let fail =
        |why: &str| format!("phar \"{fname}\" openssl signature could not be verified: {why}");
    let pem = match std::fs::read(String::from_utf8_lossy(&pk_path).as_ref()) {
        Ok(p) if !p.is_empty() => p,
        _ => return Err(fail("openssl public key could not be read")),
    };
    let algo = match sig_type {
        0x0012 => "sha512",
        0x0011 => "sha256",
        _ => "sha1",
    };
    match crate::ext::openssl::verify_with_pem(&pem, algo, body, sig) {
        Some(true) => Ok(()),
        Some(false) => Err(fail("broken openssl signature")),
        None => Err(fail("openssl signature could not be processed")),
    }
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
        let mut arc = PharArchive::parse(bytes, path)?;
        // A stored (implicit) alias is enforced over an explicit one.
        if let Some(a) = alias {
            if !arc.alias.is_empty() && arc.alias != a {
                return Err(format!(
                    "cannot load phar \"{}\" with implicit alias \"{}\" under different alias \"{}\"",
                    String::from_utf8_lossy(path),
                    String::from_utf8_lossy(&arc.alias),
                    String::from_utf8_lossy(a)
                ));
            }
        }
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
        if !self.has_archive_for(rest) {
            self.open_from_path(rest);
        }
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

impl PharWrapper {
    /// The length of the registered archive key that `rest` (a `phar://` URL
    /// without the scheme) lies in, opening the archive by path if needed.
    fn archive_prefix_len(&self, rest: &[u8]) -> Option<usize> {
        if !self.has_archive_for(rest) {
            self.open_from_path(rest);
        }
        self.archives
            .borrow()
            .keys()
            .filter(|key| {
                rest == key.as_slice()
                    || (rest.len() > key.len() && rest.starts_with(key) && rest[key.len()] == b'/')
            })
            .map(|k| k.len())
            .max()
    }

    fn has_archive_for(&self, rest: &[u8]) -> bool {
        self.archives.borrow().keys().any(|key| {
            rest == key.as_slice()
                || (rest.len() > key.len() && rest.starts_with(key) && rest[key.len()] == b'/')
        })
    }

    /// php opens a phar on first access by filesystem path
    /// (`phar:///path/to/x.phar/entry`) without any `Phar::loadPhar`: find the
    /// first path prefix that is a regular file and load it as a phar.
    fn open_from_path(&self, rest: &[u8]) {
        let mut cuts: Vec<usize> = rest
            .iter()
            .enumerate()
            .filter(|&(i, &b)| b == b'/' && i > 0)
            .map(|(i, _)| i)
            .collect();
        cuts.push(rest.len());
        for cut in cuts {
            let prefix = &rest[..cut];
            let Some(st) = crate::streams::sys_stat(prefix, false) else {
                continue;
            };
            if st.is_dir {
                continue;
            }
            if !st.is_regular {
                return;
            }
            let Ok(bytes) = std::fs::read(std::ffi::OsStr::from_bytes(prefix)) else {
                return;
            };
            let Ok(mut arc) = PharArchive::parse(bytes, prefix) else {
                return;
            };
            let real = std::fs::canonicalize(std::ffi::OsStr::from_bytes(prefix))
                .map(|p| p.into_os_string().into_vec())
                .unwrap_or_else(|_| prefix.to_vec());
            arc.path = real.clone();
            let arc = Rc::new(arc);
            self.install(&real, None, arc.clone());
            self.install(prefix, None, arc);
            return;
        }
    }

    /// `phar://<alias>` with nothing after the alias: php treats a bare name
    /// as a filesystem path to the archive, so it is the archive root only
    /// when it names the phar file itself; a bare *alias* cannot be stat'd or
    /// opened (only `phar://<alias>/` is the root).
    fn is_bare_archive(&self, url: &[u8]) -> bool {
        url.strip_prefix(b"phar://").is_some_and(|rest| {
            self.archives.borrow().contains_key(rest)
                && !crate::streams::sys_stat(rest, false).is_some_and(|s| s.is_regular)
        })
    }
}

impl StreamWrapper for PharWrapper {
    fn exists(&self, url: &[u8]) -> bool {
        if self.is_bare_archive(url) {
            return false;
        }
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
        if self.is_bare_archive(url) {
            return None;
        }
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
        if self.is_bare_archive(url) {
            return None;
        }
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
    // php's phar_open_executed_filename: the file currently executing.
    let path = host.executing_file().ok_or_else(|| {
        Thrown::error(
            "PharException",
            "cannot initialize a phar outside of PHP execution",
        )
    })?;
    let bytes = host
        .stream_get_contents(&path)
        .ok_or_else(|| Thrown::error("PharException", "cannot read the running phar"))?;
    with_phar_wrapper(host, |pw| {
        pw.map_phar(&path, alias.as_deref(), bytes)
            .map_err(|e| Thrown::error("PharException", &e))
    })?;
    Ok(Value::Bool(true))
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

/// `Phar::running($retphar)`: when the currently executing file is inside a
/// phar (`phar://<archive>/<entry>`), the `phar://<archive>` URL (or, with
/// `$retphar = false`, the archive path); otherwise "".
fn fn_phar_running(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let retphar = args
        .first()
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(true);
    let Some(fname) = host.executing_file() else {
        return Ok(Value::str(""));
    };
    if fname.len() < 7 || !fname[..7].eq_ignore_ascii_case(b"phar://") {
        return Ok(Value::str(""));
    }
    let rest = &fname[7..];
    let arch_len = with_phar_wrapper(host, |pw| Ok(pw.archive_prefix_len(rest)))?;
    Ok(match arch_len {
        Some(n) if retphar => Value::str(fname[..n + 7].to_vec()),
        Some(n) => Value::str(rest[..n].to_vec()),
        None => Value::str(""),
    })
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
    put(m, "__phardata_load", tar::fn_load, 1, Some(1));
    put(m, "__phardata_save", tar::fn_save, 3, Some(3));
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
    public static function canCompress($type = 0) { return $type === self::GZ || $type === self::BZ2 || $type === 0; }
    public static function isValidPharFilename($filename, $executable = true) { return true; }
    public static function interceptFileFuncs() {}
    public static function createDefaultStub($index = null, $webindex = null) { return ""; }
}
class PharFileInfo extends SplFileInfo {
    private $__c = ''; private $__d = false; private $__m = 0; private $__mode = 0666; private $__arc = '';
    public function __construct(string $filename) { parent::__construct($filename); }
    public static function __make(string $path, string $arc, array $e): PharFileInfo {
        $f = new PharFileInfo($path);
        $f->__c = $e['content']; $f->__d = $e['dir']; $f->__m = $e['mtime']; $f->__mode = $e['mode']; $f->__arc = $arc;
        return $f;
    }
    public function getContent(): string {
        if ($this->__d) {
            throw new BadMethodCallException('phar error: Cannot retrieve contents, "' . substr($this->__path, strlen('phar://' . $this->__arc) + 1) . '" in phar "' . $this->__arc . '" is a directory');
        }
        return $this->__c;
    }
    public function isDir(): bool { return $this->__d; }
    public function isFile(): bool { return !$this->__d; }
    public function isLink(): bool { return false; }
    public function isReadable(): bool { return true; }
    public function isWritable(): bool { return true; }
    public function getSize(): int|false { return strlen($this->__c); }
    public function getMTime(): int|false { return $this->__m; }
    public function getCTime(): int|false { return $this->__m; }
    public function getATime(): int|false { return $this->__m; }
    public function getPerms(): int|false { return ($this->__d ? 040000 : 0100000) | $this->__mode; }
    public function getType(): string|false { return $this->__d ? 'dir' : 'file'; }
    public function getCompressedSize(): int { return strlen($this->__c); }
    public function getCRC32(): int { return crc32($this->__c); }
    public function isCRCChecked(): bool { return true; }
    public function isCompressed(int $compression = 9021976): bool { return false; }
    public function hasMetadata(): bool { return false; }
    public function getMetadata(array $unserializeOptions = []): mixed { return null; }
    public function getRealPath(): string|false { return false; }
}
class PharData extends RecursiveDirectoryIterator implements Countable, ArrayAccess {
    private $__file; private $__ents = []; private $__comp = 0; private $__fmt = 2;
    public function __construct(string $filename, int $flags = FilesystemIterator::SKIP_DOTS | FilesystemIterator::UNIX_PATHS, ?string $alias = null, int $format = 0) {
        $this->__flags = $flags;
        $dir = realpath(dirname($filename));
        $this->__file = ($dir === false ? dirname($filename) : $dir) . '/' . basename($filename);
        $lower = strtolower($this->__file);
        if (is_file($filename) && filesize($filename) > 0) {
            if (str_ends_with($lower, '.zip') || $format === Phar::ZIP) {
                throw new UnexpectedValueException('zip-based phar "' . $this->__file . '" is not supported');
            }
            $r = __phardata_load($filename);
            if ($r === false) {
                throw new UnexpectedValueException('internal corruption of phar "' . $this->__file . '" (__HALT_COMPILER(); not found)');
            }
            $this->__comp = $r['comp'];
            foreach ($r['entries'] as $e) { $this->__ents[$e['name']] = $e; }
        } else {
            if (str_ends_with($lower, '.zip') || $format === Phar::ZIP) {
                throw new UnexpectedValueException('zip-based phar "' . $this->__file . '" is not supported');
            }
            if (preg_match('/\.(tar\.gz|tgz)$/', $lower)) { $this->__comp = Phar::GZ; }
            elseif (preg_match('/\.(tar\.bz2|tbz2?)$/', $lower)) { $this->__comp = Phar::BZ2; }
        }
        $this->__dir = 'phar://' . $this->__file;
        $this->__listTop();
    }
    private function __listTop(): void {
        $top = [];
        foreach ($this->__ents as $name => $e) {
            $first = explode('/', $name, 2)[0];
            $top[$first] = true;
        }
        $this->__entries = array_keys($top);
        sort($this->__entries, SORT_STRING);
        $this->__pos = 0;
    }
    private function __flush(): void {
        $r = __phardata_save($this->__file, array_values($this->__ents), $this->__comp);
        if ($r !== true) { throw new PharException($r); }
        $this->__listTop();
    }
    private function __entry(string $name): ?array {
        $name = trim(str_replace('\\', '/', $name), '/');
        if (isset($this->__ents[$name])) { return $this->__ents[$name]; }
        $pfx = $name . '/';
        foreach ($this->__ents as $n => $e) {
            if (str_starts_with($n, $pfx)) { return ['name' => $name, 'dir' => true, 'content' => '', 'mtime' => 0, 'mode' => 0777]; }
        }
        return null;
    }
    private function __info(string $name): PharFileInfo {
        $e = $this->__entry($name);
        return PharFileInfo::__make($this->__dir . '/' . $e['name'], $this->__file, $e);
    }
    public function count(): int { return count($this->__ents); }
    public function offsetExists($localName): bool { return $this->__entry((string) $localName) !== null; }
    public function offsetGet($localName): PharFileInfo {
        if ($this->__entry((string) $localName) === null) {
            throw new BadMethodCallException('Entry ' . $localName . ' does not exist');
        }
        return $this->__info((string) $localName);
    }
    public function offsetSet($localName, $value): void { $this->addFromString((string) $localName, is_resource($value) ? stream_get_contents($value) : (string) $value); }
    public function offsetUnset($localName): void { $this->delete((string) $localName); }
    public function delete(string $localName): true {
        $n = trim($localName, '/');
        if (!isset($this->__ents[$n])) { throw new BadMethodCallException('Entry ' . $localName . ' does not exist and cannot be deleted'); }
        unset($this->__ents[$n]);
        $this->__flush();
        return true;
    }
    private function __add(string $local, string $content, int $mode): void {
        $local = ltrim(str_replace('\\', '/', $local), '/');
        unset($this->__ents[$local]);
        $this->__ents[$local] = ['name' => $local, 'dir' => false, 'content' => $content, 'mtime' => time(), 'mode' => $mode];
    }
    public function addFromString(string $localName, string $contents): void { $this->__add($localName, $contents, 0666 & ~umask()); $this->__flush(); }
    public function addFile(string $filename, ?string $localName = null): void {
        $c = @file_get_contents($filename);
        if ($c === false) { throw new RuntimeException('phar error: unable to open file "' . $filename . '" to add to phar archive'); }
        $this->__add($localName ?? $filename, $c, @fileperms($filename) & 0777);
        $this->__flush();
    }
    public function addEmptyDir(string $directory): void {
        $d = trim(str_replace('\\', '/', $directory), '/');
        if (!isset($this->__ents[$d])) {
            $this->__ents[$d] = ['name' => $d, 'dir' => true, 'content' => '', 'mtime' => time(), 'mode' => 0777];
        }
        $this->__flush();
    }
    public function buildFromIterator(Traversable $iterator, ?string $baseDirectory = null): array {
        $map = [];
        foreach ($iterator as $key => $value) {
            if ($value instanceof SplFileInfo) {
                if ($value->isDir()) { continue; }
                $fname = $value->getPathname();
            } elseif (is_string($value)) {
                $fname = $value;
            } else {
                throw new UnexpectedValueException('Iterator ' . get_class($iterator) . ' returns an SplFileInfo object, so base directory must be specified');
            }
            if ($baseDirectory !== null && $baseDirectory !== '') {
                if (!str_starts_with($fname, $baseDirectory)) {
                    throw new UnexpectedValueException('Iterator ' . get_class($iterator) . ' returned a path "' . $fname . '" that is not in the base directory "' . $baseDirectory . '"');
                }
                $local = ltrim(substr($fname, strlen($baseDirectory)), '/\\');
            } else {
                $local = is_string($key) ? $key : $fname;
            }
            $c = @file_get_contents($fname);
            if ($c === false) { throw new UnexpectedValueException('Iterator ' . get_class($iterator) . ' returned a file that could not be opened "' . $fname . '"'); }
            $this->__add($local, $c, @fileperms($fname) & 0777);
            $map[$local] = $fname;
        }
        if ($map) { $this->__flush(); }
        return $map;
    }
    public function buildFromDirectory(string $directory, string $pattern = ''): array {
        $it = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($directory, FilesystemIterator::SKIP_DOTS));
        $files = [];
        foreach ($it as $f) {
            if ($pattern !== '' && !preg_match($pattern, $f->getPathname())) { continue; }
            $files[$f->getPathname()] = $f;
        }
        ksort($files);
        return $this->buildFromIterator(new ArrayIterator($files), $directory);
    }
    public function compress(int $compression, ?string $extension = null): ?PharData {
        $base = basename($this->__file);
        foreach (['.phar.tar.gz', '.phar.tar.bz2', '.phar.tar', '.phar.zip', '.phar.gz', '.phar.bz2', '.tar.gz', '.tar.bz2', '.tgz', '.tbz', '.tar', '.zip', '.phar'] as $ext) {
            if (strlen($base) > strlen($ext) && str_ends_with($base, $ext)) { $base = substr($base, 0, -strlen($ext)); break; }
        }
        $ext = $extension ?? ($compression === Phar::GZ ? 'tar.gz' : ($compression === Phar::BZ2 ? 'tar.bz2' : 'tar'));
        $new = dirname($this->__file) . '/' . $base . '.' . ltrim($ext, '.');
        if (file_exists($new)) {
            throw new BadMethodCallException('Unable to add newly converted phar "' . $new . '" to the list of phars, a phar with that name already exists');
        }
        $r = __phardata_save($new, array_values($this->__ents), $compression);
        if ($r !== true) { throw new BadMethodCallException($r); }
        return new PharData($new);
    }
    public function decompress(?string $extension = null): ?PharData { return $this->compress(Phar::NONE, $extension); }
    public function extractTo(string $directory, array|string|null $files = null, bool $overwrite = false): bool {
        if ($directory === '') { throw new InvalidArgumentException('Invalid argument, extraction path must be non-zero length'); }
        if (is_string($files)) { $files = [$files]; }
        if (!is_dir($directory) && !@mkdir($directory, 0777, true)) {
            throw new RuntimeException('Unable to create path "' . $directory . '" for extraction');
        }
        foreach ($this->__ents as $name => $e) {
            if ($files !== null) {
                $hit = false;
                foreach ($files as $f) { $f = trim($f, '/'); if ($f === $name || str_starts_with($name, $f . '/')) { $hit = true; break; } }
                if (!$hit) { continue; }
            }
            $parts = [];
            foreach (explode('/', $name) as $p) {
                if ($p === '' || $p === '.') { continue; }
                if ($p === '..') { array_pop($parts); continue; }
                $parts[] = $p;
            }
            if (!$parts) { continue; }
            $full = rtrim($directory, '/') . '/' . implode('/', $parts);
            if (!$overwrite && file_exists($full)) {
                throw new PharException('Extraction from phar "' . $this->__file . '" failed: Cannot extract "' . $name . '" to "' . $full . '", path already exists');
            }
            // phar_extract_file creates the entry's *parent* directory (with
            // the entry's mode for a directory entry), never the entry itself.
            $parent = dirname($full);
            if ($e['dir']) {
                if (!file_exists($parent) && !@mkdir($parent, $e['mode'], true)) {
                    throw new PharException('Extraction from phar "' . $this->__file . '" failed: Cannot extract "' . $name . '", could not create directory "' . $parent . '"');
                }
                continue;
            }
            if (!file_exists($parent) && !@mkdir($parent, 0777, true)) {
                throw new PharException('Extraction from phar "' . $this->__file . '" failed: Cannot extract "' . $name . '", could not create directory "' . $parent . '"');
            }
            if (@file_put_contents($full, $e['content']) === false) {
                throw new PharException('Extraction from phar "' . $this->__file . '" failed: Cannot extract "' . $name . '", could not open for writing "' . $full . '"');
            }
            @chmod($full, $e['mode']);
        }
        return true;
    }
    public function isFileFormat(int $format): bool { return $format === Phar::TAR; }
    public function getPath(): string { return $this->__file; }
    public function getPathname(): string { return $this->__dir . '/' . $this->getFilename(); }
    public function isCompressed(): int|false { return $this->__comp === 0 ? false : $this->__comp; }
    public function isWritable(): bool { return true; }
    public function isDir(): bool { $e = $this->__entry($this->getFilename()); return $e !== null && $e['dir']; }
    public function isFile(): bool { $e = $this->__entry($this->getFilename()); return $e !== null && !$e['dir']; }
    public function hasChildren(bool $allowLinks = false): bool { return $this->isDir(); }
    public function current(): mixed {
        if ($this->__flags & self::CURRENT_AS_PATHNAME) { return $this->getPathname(); }
        if ($this->__flags & self::CURRENT_AS_SELF) { return $this; }
        return $this->__info($this->getFilename());
    }
    public function valid(): bool { return $this->__pos < count($this->__entries); }
    public function getMetadata(array $unserializeOptions = []): mixed { return null; }
    public function hasMetadata(): bool { return false; }
    public function setMetadata(mixed $metadata): void {}
    public function getSignature(): array|false { return false; }
    public function getStub(): string { return ''; }
    public function getVersion(): string { return '1.1.1'; }
    public function getAlias(): ?string { return null; }
    public function startBuffering(): void {}
    public function stopBuffering(): void {}
    public function isBuffering(): bool { return false; }
    public static function canCompress(int $compression = 0): bool { return Phar::canCompress($compression); }
    public static function canWrite(): bool { return true; }
    public static function isValidPharFilename(string $filename, bool $executable = true): bool { return true; }
}
"#;
