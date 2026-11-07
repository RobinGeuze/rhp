//! The `phar` extension — enough of it to read and execute a native phar
//! (notably `composer.phar`): a native-format parser, SHA-256/512 signature
//! verification, a read-only `phar://` stream wrapper, and `Phar::mapPhar()`.
//!
//! Format (verified against composer.phar): `[stub] [manifest] [file data]
//! [signature]`, little-endian except the 2-byte API version. The stub is PHP
//! ending in `__HALT_COMPILER();`; the manifest begins just after. See
//! `docs/design/history/modules-and-streams.md` §3 for the exact field layout.
//!
//! Scope: READ-ONLY, native format only. Compressed entries use raw DEFLATE
//! (via `flate2`) or bzip2 (via the `bzip2` crate over C libbz2); tar/zip-based
//! phars are not yet supported. Writing, extraction, iteration, and the
//! compression APIs are deferred.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use flate2::read::DeflateDecoder;
use md5::Md5;
use rhp_values::ops::deref;
use rhp_values::{conv, Value};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

mod tar;

use crate::main::streams::{StatInfo, StreamWrapper, WrapperReg};
use crate::{Host, Thrown};

// --- little-endian readers ---------------------------------------------------

fn u32le(b: &[u8], pos: usize) -> Option<u32> {
    b.get(pos..pos + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

// --- parsed archive ----------------------------------------------------------

struct Entry {
    offset: usize, // absolute offset of the data block in the archive
    csize: usize,
    usize_: usize,
    flags: u32,
    timestamp: u32,
}

/// Where an archive's bytes are. php keeps only the manifest in memory and
/// reads an entry from the open file when it is used (`phar->fp`), so a
/// phar on disk is read the same way here: holding the whole archive (30 MB
/// for phpstan.phar) would count against `memory_limit` where php's doesn't.
enum Source {
    /// An archive read through a stream wrapper.
    Mem(Vec<u8>),
    /// A plain file, kept open; `len` is its size when it was opened.
    File { file: std::fs::File, len: usize },
}

/// Read size for scanning and hashing a file-backed archive.
const CHUNK: usize = 64 * 1024;

impl Source {
    /// Open a plain file as an archive source.
    fn open(path: &[u8]) -> Option<Source> {
        let file = std::fs::File::open(crate::zend::virtual_cwd::os_path(path)).ok()?;
        let meta = file.metadata().ok()?;
        if !meta.is_file() {
            return None;
        }
        Some(Source::File {
            file,
            len: usize::try_from(meta.len()).ok()?,
        })
    }

    fn len(&self) -> usize {
        match self {
            Source::Mem(d) => d.len(),
            Source::File { len, .. } => *len,
        }
    }

    /// The `len` bytes at `off`, or None past the end.
    fn read_at(&self, off: usize, len: usize) -> Option<std::borrow::Cow<'_, [u8]>> {
        use std::borrow::Cow;
        let end = off.checked_add(len)?;
        match self {
            Source::Mem(d) => d.get(off..end).map(Cow::Borrowed),
            Source::File { file, len: total } => {
                if end > *total {
                    return None;
                }
                use std::os::unix::fs::FileExt;
                let mut buf = vec![0u8; len];
                file.read_exact_at(&mut buf, off as u64).ok()?;
                Some(Cow::Owned(buf))
            }
        }
    }

    /// Feed the first `end` bytes to `f`, a chunk at a time.
    fn for_each_chunk(&self, end: usize, mut f: impl FnMut(&[u8])) -> Option<()> {
        match self {
            Source::Mem(d) => f(d.get(..end)?),
            Source::File { .. } => {
                let mut off = 0;
                while off < end {
                    let n = CHUNK.min(end - off);
                    f(&self.read_at(off, n)?);
                    off += n;
                }
            }
        }
        Some(())
    }

    /// Offset of the first `needle` (php scans the stub in chunks too).
    fn find(&self, needle: &[u8]) -> Option<usize> {
        match self {
            Source::Mem(d) => d.windows(needle.len()).position(|w| w == needle),
            Source::File { len, .. } => {
                let mut off = 0;
                while off < *len {
                    let n = (CHUNK + needle.len()).min(len - off);
                    let buf = self.read_at(off, n)?;
                    if let Some(i) = buf.windows(needle.len()).position(|w| w == needle) {
                        return Some(off + i);
                    }
                    if off + n >= *len {
                        return None;
                    }
                    off += CHUNK;
                }
                None
            }
        }
    }
}

/// A parsed, read-only native phar.
pub struct PharArchive {
    src: Source,
    alias: Vec<u8>,
    /// Filesystem path the archive was loaded from (for canonical `phar://` URLs).
    path: Vec<u8>,
    entries: HashMap<Vec<u8>, Entry>,
}

const COMPRESSED_GZ: u32 = 0x1000;
const COMPRESSED_BZ2: u32 = 0x2000;
const SIG_PRESENT: u32 = 0x1_0000;

impl PharArchive {
    /// Parse a native phar, verifying the signature. Only the manifest is
    /// read into memory.
    fn parse(src: Source, path: &[u8]) -> Result<PharArchive, String> {
        // Manifest begins just after `__HALT_COMPILER` + optional `(); ?>` + newline.
        let manifest_start = halt_offset(&src).ok_or_else(|| {
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
        let manifest_len = src
            .read_at(pos, 4)
            .and_then(|b| u32le(&b, 0))
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
        if manifest_len < 18 || src.len() < pos + manifest_len {
            return Err(corrupt("truncated manifest header"));
        }
        // File data begins after the manifest; `pos` indexes the
        // manifest's own bytes from here on.
        let data_start = pos + manifest_len;
        let manifest = src
            .read_at(pos, manifest_len)
            .ok_or_else(|| corrupt("truncated manifest header"))?;
        let data: &[u8] = &manifest;
        let mut pos = 0;
        let num_files = u32le(data, pos).ok_or("truncated file count")? as usize;
        if num_files == 0 {
            return Err(format!(
                "in phar \"{}\", manifest claims to have zero entries.  Phars must have at least 1 entry",
                String::from_utf8_lossy(path)
            ));
        }
        pos += 4;
        pos += 2; // API version (big-endian nibbles; not needed to read)
        let global_flags = u32le(data, pos).ok_or("truncated global flags")?;
        pos += 4;
        let alias_len = u32le(data, pos).ok_or("truncated alias length")? as usize;
        pos += 4;
        let alias = data
            .get(pos..pos + alias_len)
            .ok_or("truncated alias")?
            .to_vec();
        pos += alias_len;
        let meta_len = u32le(data, pos).ok_or("truncated global metadata length")? as usize;
        pos += 4 + meta_len; // skip global metadata (serialize())

        if global_flags & SIG_PRESENT != 0 {
            verify_signature(&src, path)?;
        }

        let mut entries = HashMap::with_capacity(num_files);
        let mut cursor = data_start;
        for _ in 0..num_files {
            let name_len = u32le(data, pos).ok_or("truncated entry name length")? as usize;
            pos += 4;
            let name = data
                .get(pos..pos + name_len)
                .ok_or("truncated entry name")?
                .to_vec();
            pos += name_len;
            let usize_ = u32le(data, pos).ok_or("truncated usize")? as usize;
            pos += 4;
            let timestamp = u32le(data, pos).ok_or("truncated mtime")?;
            pos += 4;
            let csize = u32le(data, pos).ok_or("truncated csize")? as usize;
            pos += 4;
            pos += 4; // crc32
            let flags = u32le(data, pos).ok_or("truncated entry flags")?;
            pos += 4;
            let em_len = u32le(data, pos).ok_or("truncated entry metadata length")? as usize;
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

        drop(manifest);
        Ok(PharArchive {
            src,
            alias,
            path: Vec::new(),
            entries,
        })
    }

    /// The raw (decompressed) bytes of an internal entry.
    fn read(&self, entry: &[u8]) -> Option<Vec<u8>> {
        let e = self.entries.get(entry)?;
        let raw = self.src.read_at(e.offset, e.csize)?;
        if e.flags & COMPRESSED_GZ != 0 {
            use std::io::Read;
            let mut out = Vec::with_capacity(e.usize_);
            DeflateDecoder::new(&raw[..]).read_to_end(&mut out).ok()?;
            Some(out)
        } else if e.flags & COMPRESSED_BZ2 != 0 {
            use std::io::Read;
            let mut out = Vec::with_capacity(e.usize_);
            bzip2::read::BzDecoder::new(&raw[..])
                .read_to_end(&mut out)
                .ok()?;
            Some(out)
        } else {
            Some(raw.into_owned())
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
fn halt_offset(src: &Source) -> Option<usize> {
    let tok = b"__HALT_COMPILER";
    let at = src.find(tok)?;
    // the token and what follows it: `(); ?>` and a newline, give or take blanks
    let data = src.read_at(at, (src.len() - at).min(256))?;
    let mut i = tok.len();
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
    Some(at + i)
}

/// Verify the trailing signature `[hash][type(4,LE)][GBMB]` over
/// `data[0 .. len-(hashlen+8)]`. Supports MD5, SHA-1, SHA-256 and SHA-512
/// (composer uses SHA-512), all verified byte-for-byte. OpenSSL signatures
/// (types 0x10/0x11/0x12: `[sig][siglen(4,LE)][type][GBMB]`, RSA over SHA-1 /
/// SHA-256 / SHA-512) are verified against `<phar path>.pubkey`, as php does.
fn verify_signature(src: &Source, path: &[u8]) -> Result<(), String> {
    let n = src.len();
    if n < 8 || src.read_at(n - 4, 4).as_deref() != Some(b"GBMB") {
        return Err("missing GBMB signature trailer".into());
    }
    let sig_type = src
        .read_at(n - 8, 4)
        .and_then(|b| u32le(&b, 0))
        .ok_or("truncated signature type")?;
    // Each arm hashes `data[0 .. n-(hashlen+8)]` (everything before the stored
    // hash, the 4-byte type, and the `GBMB` magic) and compares to the stored hash.
    let hashlen = match sig_type {
        0x0004 => 64,
        0x0003 => 32,
        0x0002 => 20,
        0x0001 => 16,
        0x0010..=0x0012 => return verify_openssl_signature(src, sig_type, path),
        other => return Err(format!("unknown phar signature type {other}")),
    };
    let body_len = n
        .checked_sub(hashlen + 8)
        .ok_or("truncated signature hash")?;
    let computed = match sig_type {
        0x0004 => digest_prefix::<Sha512>(src, body_len),
        0x0003 => digest_prefix::<Sha256>(src, body_len),
        0x0002 => digest_prefix::<Sha1>(src, body_len),
        _ => digest_prefix::<Md5>(src, body_len),
    }
    .ok_or("truncated signature hash")?;
    let stored = src
        .read_at(body_len, hashlen)
        .ok_or("truncated signature hash")?;
    if *stored != *computed {
        return Err("phar signature verification failed".into());
    }
    Ok(())
}

/// The `D` digest of the first `len` bytes of `src`.
fn digest_prefix<D: Digest>(src: &Source, len: usize) -> Option<Vec<u8>> {
    let mut d = D::new();
    src.for_each_chunk(len, |c| d.update(c))?;
    Some(d.finalize().to_vec())
}

/// The OpenSSL signature arms of [`verify_signature`] (php-src
/// `phar_verify_signature`): RSA verify of everything before the signature
/// with the PEM public key in `<path>.pubkey`.
fn verify_openssl_signature(src: &Source, sig_type: u32, path: &[u8]) -> Result<(), String> {
    let n = src.len();
    let fname = String::from_utf8_lossy(path).into_owned();
    let len_at = n
        .checked_sub(12)
        .ok_or_else(|| format!("phar \"{fname}\" openssl signature length could not be read"))?;
    let sig_len = src
        .read_at(len_at, 4)
        .and_then(|b| u32le(&b, 0))
        .ok_or_else(|| format!("phar \"{fname}\" openssl signature length could not be read"))?
        as usize;
    let sig_start = len_at
        .checked_sub(sig_len)
        .ok_or_else(|| format!("phar \"{fname}\" openssl signature could not be read"))?;
    let unreadable = || format!("phar \"{fname}\" openssl signature could not be read");
    let sig = src.read_at(sig_start, sig_len).ok_or_else(unreadable)?;
    let body = src.read_at(0, sig_start).ok_or_else(unreadable)?;
    let mut pk_path = path.to_vec();
    pk_path.extend_from_slice(b".pubkey");
    let fail =
        |why: &str| format!("phar \"{fname}\" openssl signature could not be verified: {why}");
    let pem = match std::fs::read(crate::zend::virtual_cwd::os_path(&pk_path)) {
        Ok(p) if !p.is_empty() => p,
        _ => return Err(fail("openssl public key could not be read")),
    };
    let algo = match sig_type {
        0x0012 => "sha512",
        0x0011 => "sha256",
        _ => "sha1",
    };
    match crate::ext::openssl::verify_with_pem(&pem, algo, &body, &sig) {
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

    fn map_phar(&self, path: &[u8], alias: Option<&[u8]>, src: Source) -> Result<(), String> {
        let mut arc = PharArchive::parse(src, path)?;
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
            let Some(st) = crate::main::streams::sys_stat(prefix, false) else {
                continue;
            };
            if st.is_dir {
                continue;
            }
            if !st.is_regular {
                return;
            }
            // phar_open_from_filename's open_basedir check (php's later
            // checks and its phar error are not modelled)
            if !crate::main::fopen_wrappers::check_queued(prefix, true) {
                return;
            }
            let Some(src) = Source::open(prefix) else {
                return;
            };
            let Ok(mut arc) = PharArchive::parse(src, prefix) else {
                return;
            };
            let real =
                crate::zend::virtual_cwd::realpath(prefix).unwrap_or_else(|| prefix.to_vec());
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
                && !crate::main::streams::sys_stat(rest, false).is_some_and(|s| s.is_regular)
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

/// The archive at `path`: a plain file stays on disk and is read as its
/// entries are used; anything else (a wrapper URL, a path open_basedir
/// governs) is read whole through the stream layer.
fn open_source(host: &mut dyn Host, path: &[u8]) -> Option<Source> {
    let plain = !path.windows(3).any(|w| w == b"://") && !crate::main::fopen_wrappers::active();
    if plain {
        if let Some(src) = Source::open(path) {
            return Some(src);
        }
    }
    host.stream_get_contents(path).map(Source::Mem)
}

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
    let src = open_source(host, &path)
        .ok_or_else(|| Thrown::error("PharException", "cannot read the running phar"))?;
    with_phar_wrapper(host, |pw| {
        pw.map_phar(&path, alias.as_deref(), src)
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
    let src = open_source(host, &filename)
        .ok_or_else(|| Thrown::error("PharException", "cannot read phar file"))?;
    with_phar_wrapper(host, |pw| {
        pw.map_phar(&filename, alias.as_deref(), src)
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

pub fn register(m: &mut crate::Registry) {
    crate::register_helpers(
        m,
        &[
            ("__phar_mapphar", fn_phar_mapphar, 0, Some(2)),
            ("__phar_loadphar", fn_phar_loadphar, 1, Some(2)),
            ("__phar_running", fn_phar_running, 0, Some(1)),
            ("__phardata_load", tar::fn_load, 1, Some(1)),
            ("__phardata_save", tar::fn_save, 3, Some(3)),
        ],
    );
}

pub fn wrappers() -> Vec<WrapperReg> {
    vec![WrapperReg {
        scheme: "phar",
        factory: || Rc::new(PharWrapper::new()),
    }]
}

/// The `Phar` class (read-only subset) + `PharException`. Static methods delegate
/// to the module's native `__phar_*` builtins (the GMP/DateTime idiom).
pub const PRELUDE_PHP: &str = include_str!("prelude.php");
