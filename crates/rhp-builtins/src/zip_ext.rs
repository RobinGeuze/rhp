//! Backing state for the `ZipArchive` class: an in-memory model of a zip
//! archive (read from disk on `open`, rewritten on `close` when changed), in
//! the spirit of libzip, which php's ext/zip wraps.
//!
//! Reading parses the central directory by hand (deflate/bzip2/stored data);
//! writing emits the same layout libzip does — local header + data per entry,
//! no data descriptors, no extra fields, "version made by" 6.3 with the entry's
//! OS byte, deflate at level 9 falling back to stored when it does not shrink.
//!
//! Open archives live in a thread-local table keyed by id; the PHP-level
//! `ZipArchive` prelude class (ext/zip) holds the id.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;

pub const ER_OK: i64 = 0;
pub const ER_NOENT: i64 = 9;
pub const ER_EXISTS: i64 = 10;
pub const ER_OPEN: i64 = 11;
pub const ER_WRITE: i64 = 6;
/// Opening a directory: libzip fails reading it.
/// Opening a directory: libzip reports ZIP_ER_OPNOTSUPP.
pub const ER_READ_DIR: i64 = 28;
/// `ZIP_ER_CHANGED`: reading an entry whose data source is not committed.
pub const ER_CHANGED: i64 = 15;
pub const ER_INVAL: i64 = 18;
pub const ER_NOZIP: i64 = 19;
pub const ER_INCONS: i64 = 21;
pub const ER_DELETED: i64 = 23;
pub const ER_RDONLY: i64 = 25;
pub const ER_COMPNOTSUPP: i64 = 16;

pub const FL_CREATE: i64 = 1;
pub const FL_EXCL: i64 = 2;
pub const FL_CHECKCONS: i64 = 4;
pub const FL_TRUNCATE: i64 = 8;
pub const FL_RDONLY: i64 = 16;

pub const OPSYS_UNIX: u8 = 3;

/// libzip's error strings (`zip_error_strerror`), indexed by `ER_*`.
pub const ERROR_STRINGS: &[&str] = &[
    "No error",
    "Multi-disk zip archives not supported",
    "Renaming temporary file failed",
    "Closing zip archive failed",
    "Seek error",
    "Read error",
    "Write error",
    "CRC error",
    "Containing zip archive was closed",
    "No such file",
    "File already exists",
    "Can't open file",
    "Failure to create temporary file",
    "Zlib error",
    "Malloc failure",
    "Entry has been changed",
    "Compression method not supported",
    "Premature end of file",
    "Invalid argument",
    "Not a zip archive",
    "Internal error",
    "Zip archive inconsistent",
    "Can't remove file",
    "Entry has been deleted",
    "Encryption method not supported",
    "Read-only archive",
    "No password provided",
    "Wrong password provided",
    "Operation not supported",
    "Resource still in use",
    "Tell error",
    "Compressed data invalid",
    "Operation cancelled",
    "Unexpected length of data",
    "Not allowed in torrentzip",
    "Possibly truncated or corrupted zip archive",
];

#[derive(Clone)]
enum Data {
    /// Unchanged entry from the archive on disk: compressed bytes as stored.
    Raw {
        method: u16,
        flags: u16,
        crc: u32,
        size: u64,
        comp: Vec<u8>,
    },
    /// New / replaced contents (compressed at close).
    New(Vec<u8>),
}

#[derive(Clone)]
pub struct Entry {
    pub name: Vec<u8>,
    data: Data,
    /// Unix mtime.
    pub mtime: i64,
    pub opsys: u8,
    pub attr: u32,
    pub comment: Vec<u8>,
    /// `setCompressionName` override (`CM_*`; -1 default).
    pub method: i32,
    pub deleted: bool,
    /// Whether the entry is pending (added/replaced since open).
    pub pending: bool,
    /// Pending data from a file source (`addFile`/`replaceFile`): libzip
    /// can't read it back (ZIP_ER_CHANGED) and stats a compressed size of 0.
    pub from_file: bool,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.name.last() == Some(&b'/')
    }
    pub fn size(&self) -> u64 {
        match &self.data {
            Data::Raw { size, .. } => *size,
            Data::New(d) => d.len() as u64,
        }
    }
    /// (crc, comp_size, comp_method) as php's `statIndex` reports them.
    pub fn stat_bits(&self) -> (u32, u64, u16) {
        match &self.data {
            Data::Raw {
                method, crc, comp, ..
            } => (*crc, comp.len() as u64, *method),
            Data::New(_) if self.from_file => (0, 0, 0),
            Data::New(d) => (0, d.len() as u64, 0),
        }
    }
    /// Uncompressed contents.
    pub fn contents(&self) -> Option<Vec<u8>> {
        if self.from_file {
            return None;
        }
        match &self.data {
            Data::New(d) => Some(d.clone()),
            Data::Raw {
                method, comp, size, ..
            } => match method {
                0 => Some(comp.clone()),
                8 => {
                    let mut out = Vec::with_capacity(*size as usize);
                    flate2::read::DeflateDecoder::new(&comp[..])
                        .read_to_end(&mut out)
                        .ok()?;
                    Some(out)
                }
                12 => {
                    let mut out = Vec::with_capacity(*size as usize);
                    bzip2::read::BzDecoder::new(&comp[..])
                        .read_to_end(&mut out)
                        .ok()?;
                    Some(out)
                }
                _ => None,
            },
        }
    }
}

pub struct Archive {
    pub path: Vec<u8>,
    pub entries: Vec<Entry>,
    pub comment: Vec<u8>,
    original_comment: Vec<u8>,
    original: Vec<Entry>,
    pub changed: bool,
    /// The file existed when opened (so an emptied archive is removed on close).
    existed: bool,
    /// The file was zero bytes when opened (libzip removes it if still empty).
    was_empty_file: bool,
    pub rdonly: bool,
    /// Last libzip error code on this archive (`$zip->status`).
    pub status: i64,
}

fn u16le(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}
fn u32le(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}
fn u64le(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?))
}

/// DOS date/time (local time) → unix time, as libzip's `_zip_d2u_time`.
pub fn dos_to_unix(time: u16, date: u16) -> i64 {
    // SAFETY: tm is plain data; mktime only reads/normalizes it.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_isdst = -1;
        tm.tm_year = ((date >> 9) & 127) as i32 + 1980 - 1900;
        tm.tm_mon = ((date >> 5) & 15) as i32 - 1;
        tm.tm_mday = (date & 31) as i32;
        tm.tm_hour = ((time >> 11) & 31) as i32;
        tm.tm_min = ((time >> 5) & 63) as i32;
        tm.tm_sec = ((time << 1) & 62) as i32;
        libc::mktime(&mut tm) as i64
    }
}

/// unix time → DOS (time, date) in local time, as libzip's `_zip_u2d_time`.
pub fn unix_to_dos(t: i64) -> (u16, u16) {
    // SAFETY: localtime_r fills the provided tm.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let tt = t as libc::time_t;
        if libc::localtime_r(&tt, &mut tm).is_null() {
            return (0, 0x21);
        }
        let year = (tm.tm_year + 1900).max(1980);
        let date = (((year - 1980) << 9) | ((tm.tm_mon + 1) << 5) | tm.tm_mday) as u16;
        let time = ((tm.tm_hour << 11) | (tm.tm_min << 5) | (tm.tm_sec >> 1)) as u16;
        (time, date)
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse an archive's central directory. `Err(code)` on a malformed file.
fn parse(bytes: &[u8]) -> Result<(Vec<Entry>, Vec<u8>), i64> {
    // Locate the end-of-central-directory record (scan back over the comment).
    if bytes.len() < 22 {
        return Err(ER_NOZIP);
    }
    let min = bytes.len().saturating_sub(22 + 65535);
    let mut eocd = None;
    let mut i = bytes.len() - 22;
    loop {
        if bytes[i..i + 4] == [0x50, 0x4b, 0x05, 0x06] {
            eocd = Some(i);
            break;
        }
        if i == min {
            break;
        }
        i -= 1;
    }
    let e = eocd.ok_or(ER_NOZIP)?;
    let mut count = u16le(bytes, e + 10).ok_or(ER_NOZIP)? as u64;
    let mut cd_size = u32le(bytes, e + 12).ok_or(ER_NOZIP)? as u64;
    let mut cd_off = u32le(bytes, e + 16).ok_or(ER_NOZIP)? as u64;
    let clen = u16le(bytes, e + 20).ok_or(ER_NOZIP)? as usize;
    let comment = bytes.get(e + 22..e + 22 + clen).unwrap_or(&[]).to_vec();
    // zip64 end-of-central-directory locator just before the EOCD.
    if e >= 20 && bytes[e - 20..e - 16] == [0x50, 0x4b, 0x06, 0x07] {
        let z = u64le(bytes, e - 12).ok_or(ER_NOZIP)? as usize;
        if bytes.get(z..z + 4) == Some(&[0x50, 0x4b, 0x06, 0x06]) {
            count = u64le(bytes, z + 32).ok_or(ER_NOZIP)?;
            cd_size = u64le(bytes, z + 40).ok_or(ER_NOZIP)?;
            cd_off = u64le(bytes, z + 48).ok_or(ER_NOZIP)?;
        }
    }
    let _ = cd_size;
    let mut p = cd_off as usize;
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if u32le(bytes, p) != Some(0x0201_4b50) {
            return Err(ER_INCONS);
        }
        let made_by = u16le(bytes, p + 4).ok_or(ER_INCONS)?;
        let flags = u16le(bytes, p + 8).ok_or(ER_INCONS)?;
        let method = u16le(bytes, p + 10).ok_or(ER_INCONS)?;
        let dtime = u16le(bytes, p + 12).ok_or(ER_INCONS)?;
        let ddate = u16le(bytes, p + 14).ok_or(ER_INCONS)?;
        let crc = u32le(bytes, p + 16).ok_or(ER_INCONS)?;
        let mut csize = u32le(bytes, p + 20).ok_or(ER_INCONS)? as u64;
        let mut size = u32le(bytes, p + 24).ok_or(ER_INCONS)? as u64;
        let nlen = u16le(bytes, p + 28).ok_or(ER_INCONS)? as usize;
        let xlen = u16le(bytes, p + 30).ok_or(ER_INCONS)? as usize;
        let cmlen = u16le(bytes, p + 32).ok_or(ER_INCONS)? as usize;
        let attr = u32le(bytes, p + 38).ok_or(ER_INCONS)?;
        let mut loff = u32le(bytes, p + 42).ok_or(ER_INCONS)? as u64;
        let name = bytes.get(p + 46..p + 46 + nlen).ok_or(ER_INCONS)?.to_vec();
        let extra = bytes
            .get(p + 46 + nlen..p + 46 + nlen + xlen)
            .ok_or(ER_INCONS)?;
        let ecomment = bytes
            .get(p + 46 + nlen + xlen..p + 46 + nlen + xlen + cmlen)
            .ok_or(ER_INCONS)?
            .to_vec();
        // zip64 extended information extra field.
        let mut x = 0;
        while x + 4 <= extra.len() {
            let id = u16le(extra, x).unwrap_or(0);
            let len = u16le(extra, x + 2).unwrap_or(0) as usize;
            if id == 1 {
                let mut q = x + 4;
                if size == 0xFFFF_FFFF {
                    size = u64le(extra, q).unwrap_or(size);
                    q += 8;
                }
                if csize == 0xFFFF_FFFF {
                    csize = u64le(extra, q).unwrap_or(csize);
                    q += 8;
                }
                if loff == 0xFFFF_FFFF {
                    loff = u64le(extra, q).unwrap_or(loff);
                }
            }
            x += 4 + len;
        }
        p += 46 + nlen + xlen + cmlen;
        // Local header → data offset.
        let l = loff as usize;
        if u32le(bytes, l) != Some(0x0403_4b50) {
            return Err(ER_INCONS);
        }
        let lnlen = u16le(bytes, l + 26).ok_or(ER_INCONS)? as usize;
        let lxlen = u16le(bytes, l + 28).ok_or(ER_INCONS)? as usize;
        let ds = l + 30 + lnlen + lxlen;
        let comp = bytes
            .get(ds..ds + csize as usize)
            .ok_or(ER_INCONS)?
            .to_vec();
        entries.push(Entry {
            name,
            data: Data::Raw {
                method,
                flags,
                crc,
                size,
                comp,
            },
            mtime: dos_to_unix(dtime, ddate),
            opsys: (made_by >> 8) as u8,
            attr,
            comment: ecomment,
            method: -1,
            deleted: false,
            pending: false,
            from_file: false,
        });
    }
    Ok((entries, comment))
}

impl Archive {
    /// libzip `zip_open` semantics. `bytes` is the file's contents when it exists.
    pub fn open(path: Vec<u8>, bytes: Option<Vec<u8>>, flags: i64) -> Result<Archive, i64> {
        let existed = bytes.is_some();
        let was_empty_file = bytes.as_ref().is_some_and(|b| b.is_empty());
        let (entries, comment) = match bytes {
            None => {
                if flags & FL_CREATE == 0 {
                    return Err(ER_NOENT);
                }
                (Vec::new(), Vec::new())
            }
            Some(b) => {
                if flags & FL_CREATE != 0 && flags & FL_EXCL != 0 {
                    return Err(ER_EXISTS);
                }
                if flags & FL_TRUNCATE != 0 || b.is_empty() {
                    (Vec::new(), Vec::new())
                } else {
                    parse(&b)?
                }
            }
        };
        Ok(Archive {
            path,
            original: entries.clone(),
            original_comment: comment.clone(),
            entries,
            comment,
            changed: flags & FL_TRUNCATE != 0 && existed,
            existed,
            was_empty_file,
            rdonly: flags & FL_RDONLY != 0,
            status: 0,
        })
    }

    /// `zip_name_locate`: index of a live entry by name.
    pub fn locate(&self, name: &[u8], nocase: bool, nodir: bool) -> Option<usize> {
        self.entries.iter().position(|e| {
            if e.deleted {
                return false;
            }
            let n: &[u8] = if nodir {
                match e.name.iter().rposition(|&c| c == b'/') {
                    Some(i) => &e.name[i + 1..],
                    None => &e.name,
                }
            } else {
                &e.name
            };
            if nocase {
                n.eq_ignore_ascii_case(name)
            } else {
                n == name
            }
        })
    }

    pub fn live(&self, i: i64) -> Option<&Entry> {
        if i < 0 {
            return None;
        }
        self.entries.get(i as usize).filter(|e| !e.deleted)
    }

    pub fn live_mut(&mut self, i: i64) -> Option<&mut Entry> {
        if i < 0 {
            return None;
        }
        self.entries.get_mut(i as usize).filter(|e| !e.deleted)
    }

    /// Add (or with `overwrite`, replace) an entry. Returns its index, or an
    /// error code (`ER_EXISTS` when the name is taken and not overwriting).
    pub fn add(
        &mut self,
        name: Vec<u8>,
        data: Vec<u8>,
        mtime: i64,
        attr: u32,
        overwrite: bool,
    ) -> Result<usize, i64> {
        if self.rdonly {
            return Err(ER_RDONLY);
        }
        if name.is_empty() {
            return Err(ER_INVAL);
        }
        let entry = Entry {
            name: name.clone(),
            data: Data::New(data),
            mtime,
            opsys: OPSYS_UNIX,
            attr,
            comment: Vec::new(),
            method: -1,
            deleted: false,
            pending: true,
            from_file: false,
        };
        self.changed = true;
        if let Some(i) = self.locate(&name, false, false) {
            if !overwrite {
                return Err(ER_EXISTS);
            }
            self.entries[i] = entry;
            return Ok(i);
        }
        self.entries.push(entry);
        Ok(self.entries.len() - 1)
    }

    pub fn replace(&mut self, i: i64, data: Vec<u8>, mtime: i64) -> bool {
        match self.live_mut(i) {
            Some(e) => {
                e.data = Data::New(data);
                e.mtime = mtime;
                e.pending = true;
                e.from_file = true;
                self.changed = true;
                true
            }
            None => false,
        }
    }

    pub fn delete(&mut self, i: i64) -> bool {
        match self.live_mut(i) {
            Some(e) => {
                e.deleted = true;
                self.changed = true;
                true
            }
            None => false,
        }
    }

    pub fn rename(&mut self, i: i64, name: Vec<u8>) -> Result<(), i64> {
        if name.is_empty() {
            return Err(ER_INVAL);
        }
        if let Some(j) = self.locate(&name, false, false) {
            if j as i64 != i {
                return Err(ER_EXISTS);
            }
        }
        match self.live_mut(i) {
            Some(e) => {
                e.name = name;
                self.changed = true;
                Ok(())
            }
            None => Err(ER_INVAL),
        }
    }

    pub fn touch(&mut self) {
        self.changed = true;
    }

    /// Revert one entry to its on-disk state (`unchangeIndex`).
    pub fn unchange(&mut self, i: i64) -> bool {
        if i < 0 || i as usize >= self.entries.len() {
            return false;
        }
        let i = i as usize;
        if i < self.original.len() {
            self.entries[i] = self.original[i].clone();
        } else {
            self.entries[i].deleted = true;
        }
        true
    }

    pub fn unchange_all(&mut self) {
        self.entries = self.original.clone();
        self.comment = self.original_comment.clone();
        self.changed = false;
    }

    pub fn unchange_archive(&mut self) {
        self.comment = self.original_comment.clone();
    }

    /// Serialize the live entries (libzip's on-disk layout).
    pub fn serialize(&self) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        let mut central: Vec<u8> = Vec::new();
        let mut n = 0u64;
        for e in self.entries.iter().filter(|e| !e.deleted) {
            n += 1;
            let (method, mut flags, crc, size, comp): (u16, u16, u32, u64, Vec<u8>) = match &e.data
            {
                Data::Raw {
                    method,
                    flags,
                    crc,
                    size,
                    comp,
                } => (*method, flags & !0x0008, *crc, *size, comp.clone()),
                Data::New(d) => {
                    let crc = crate::ext::zlib::zraw::crc32(d);
                    let want_store = e.method == 0 || d.is_empty() || e.is_dir();
                    let deflated = if want_store {
                        None
                    } else {
                        crate::ext::zlib::raw_deflate(d, 9)
                    };
                    match deflated {
                        Some(c) if c.len() < d.len() || e.method == 8 => {
                            (8, 0x0002, crc, d.len() as u64, c)
                        }
                        _ => (0, 0, crc, d.len() as u64, d.clone()),
                    }
                }
            };
            if !e.name.is_ascii() && std::str::from_utf8(&e.name).is_ok() {
                flags |= 0x0800;
            }
            let (dt, dd) = unix_to_dos(e.mtime);
            let off = out.len() as u32;
            // local file header
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&flags.to_le_bytes());
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&dt.to_le_bytes());
            out.extend_from_slice(&dd.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            out.extend_from_slice(&(size as u32).to_le_bytes());
            out.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&e.name);
            out.extend_from_slice(&comp);
            // central directory header
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&(((e.opsys as u16) << 8) | 63).to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&flags.to_le_bytes());
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&dt.to_le_bytes());
            central.extend_from_slice(&dd.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            central.extend_from_slice(&(size as u32).to_le_bytes());
            central.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&(e.comment.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes()); // disk number
            central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            central.extend_from_slice(&e.attr.to_le_bytes());
            central.extend_from_slice(&off.to_le_bytes());
            central.extend_from_slice(&e.name);
            central.extend_from_slice(&e.comment);
        }
        let cd_off = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(n as u16).to_le_bytes());
        out.extend_from_slice(&(n as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&(self.comment.len() as u16).to_le_bytes());
        out.extend_from_slice(&self.comment);
        out
    }

    /// Write the archive back if it changed (libzip `zip_close`). An archive
    /// left with no entries is not written; an existing file is removed.
    pub fn commit(&self) -> Result<(), i64> {
        let path = String::from_utf8_lossy(&self.path).into_owned();
        if !self.changed {
            if self.was_empty_file && self.entries.is_empty() {
                let _ = std::fs::remove_file(&path);
            }
            return Ok(());
        }
        if self.entries.iter().all(|e| e.deleted) && self.comment.is_empty() {
            if self.existed && std::fs::remove_file(&path).is_err() {
                return Err(22); // ER_REMOVE
            }
            return Ok(());
        }
        std::fs::write(&path, self.serialize()).map_err(|_| ER_OPEN)
    }
}

/// php's `php_zip_make_relative_path` + CWD_EXPAND normalization: drop `.`/`..`
/// and leading slashes so an entry can never escape the destination.
fn clean_entry_path(name: &[u8]) -> Vec<u8> {
    let mut parts: Vec<&[u8]> = Vec::new();
    // IS_SLASH is only '/' on Unix: a backslash is an ordinary name byte.
    for seg in name.split(|&c| c == b'/') {
        match seg {
            b"" | b"." => {}
            b".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join(&b'/')
}

/// `ZipArchive::extractTo` for one entry. Returns false on failure.
pub fn extract_entry(e: &Entry, dest: &str) -> bool {
    let rel = clean_entry_path(&e.name);
    if rel.is_empty() {
        return false;
    }
    let rel = String::from_utf8_lossy(&rel).into_owned();
    let full = format!("{dest}/{rel}");
    if e.is_dir() {
        return std::fs::create_dir_all(&full).is_ok() || std::path::Path::new(&full).is_dir();
    }
    if let Some(parent) = std::path::Path::new(&full).parent() {
        if !parent.is_dir() && std::fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    match e.contents() {
        Some(data) => std::fs::write(&full, data).is_ok(),
        None => false,
    }
}

thread_local! {
    /// Open archives keyed by the owning ZipArchive's spl_object_id.
    static ARCHIVES: RefCell<HashMap<i64, Archive>> = RefCell::new(HashMap::new());
}

/// Drop every open archive (a new request: object ids restart).
pub(crate) fn reset_state() {
    ARCHIVES.with(|t| t.borrow_mut().clear());
}

pub fn insert(id: i64, a: Archive) {
    ARCHIVES.with(|t| t.borrow_mut().insert(id, a));
}

pub fn with<R>(id: i64, f: impl FnOnce(&mut Archive) -> R) -> Option<R> {
    ARCHIVES.with(|t| t.borrow_mut().get_mut(&id).map(f))
}

pub fn remove(id: i64) -> Option<Archive> {
    ARCHIVES.with(|t| t.borrow_mut().remove(&id))
}

/// `free_obj` for a `ZipArchive`: libzip's `zip_close` runs when the object is
/// freed (also at shutdown), writing an archive that was never closed.
pub fn object_freed(id: u32) {
    let a = ARCHIVES
        .try_with(|t| {
            t.try_borrow_mut()
                .ok()
                .and_then(|mut t| t.remove(&(id as i64)))
        })
        .ok()
        .flatten();
    if let Some(a) = a {
        let _ = a.commit();
    }
}
