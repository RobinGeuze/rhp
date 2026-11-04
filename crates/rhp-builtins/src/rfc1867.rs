//! `multipart/form-data` request bodies (php's `main/rfc1867.c`) and the
//! upload builtins: `is_uploaded_file`, `move_uploaded_file` and
//! `request_parse_body` (`ext/standard/http.c`).
//!
//! [`parse`] is a line-by-line port of `rfc1867_post_handler`, including its
//! 5 KiB `multipart_buffer` (how a line longer than the buffer or a split
//! boundary is treated depends on it). It fills `$_POST` and `$_FILES`,
//! writes each uploaded file to `upload_tmp_dir` (else the system temp
//! directory) as `php` + random, and returns what php reports through
//! `php_error_docref` as [`Diag`]s for the caller to raise: during request
//! startup they are "PHP Request Startup: …" warnings, under
//! `request_parse_body()` exceptions.
//!
//! The request's uploads are tracked in its [`BodyState`]
//! (`SG(rfc1867_uploaded_files)`): the files left there are deleted when the
//! request ends.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;

use rhp_values::ops::deref;
use rhp_values::{ArrayKey, PhpArray, Thrown, Value};

use crate::query_fns::{post_form_vars, register_variable_value};
use crate::Host;

/// `FILLUNIT`: the multipart buffer's minimum size.
const FILLUNIT: usize = 1024 * 5;

/// `SAPI_POST_BLOCK_SIZE`: the block `sapi_read_standard_form_data` reads
/// the body in.
pub const SAPI_POST_BLOCK_SIZE: usize = 0x40000;

/// The warning php raises once more than `post_max_size` bytes of a body
/// were actually read, whatever its `Content-Length` said
/// (GHSA-2cv3-554h-jpmh).
pub fn actual_length_exceeds(post_max_size: i64) -> String {
    format!("Actual POST length does not match Content-Length, and exceeds {post_max_size} bytes")
}

/// `UPLOAD_ERR_*`.
const UPLOAD_ERR_INI_SIZE: i64 = 1;
const UPLOAD_ERR_FORM_SIZE: i64 = 2;
const UPLOAD_ERR_PARTIAL: i64 = 3;
const UPLOAD_ERR_NO_FILE: i64 = 4;
const UPLOAD_ERR_CANT_WRITE: i64 = 7;

/// The request body's input state (php's `SG(read_post_bytes)`,
/// `SG(request_info).request_body` and `SG(rfc1867_uploaded_files)`).
/// The read position and buffer are interior-mutable: opening
/// `php://input` reads the body, from a `&self` stream lookup.
#[derive(Default)]
pub struct BodyState {
    /// Bytes of the body already read from the SAPI (by a POST reader, the
    /// multipart parser or `php://input`).
    consumed: Cell<usize>,
    /// `SG(request_info).request_body`: what a reader buffered of the body
    /// (`php://input` reads it).
    buffered: RefCell<Option<Vec<u8>>>,
    /// This request's uploaded files (`is_uploaded_file`), once a multipart
    /// body was parsed.
    uploaded: Option<Vec<Vec<u8>>>,
    /// Uploads of an earlier parse that a later one replaced (php leaks
    /// them; rhp still deletes them at request end).
    orphans: Vec<Vec<u8>>,
}

impl BodyState {
    /// What `php://input` reads: the buffered body, else (creating the
    /// buffer, as php's opener does) whatever the SAPI has left.
    pub fn input(&self, body: &[u8]) -> Vec<u8> {
        if self.buffered.borrow().is_none() {
            self.buffer_rest(body);
        }
        self.buffered.borrow().clone().unwrap_or_default()
    }

    /// The body bytes the SAPI has not delivered yet.
    pub fn remaining<'a>(&self, body: &'a [u8]) -> &'a [u8] {
        body.get(self.consumed.get()..).unwrap_or_default()
    }

    /// `sapi_read_standard_form_data` (within `post_max_size`): the rest of
    /// the body becomes `request_body`.
    pub fn buffer_rest(&self, body: &[u8]) {
        *self.buffered.borrow_mut() = Some(self.remaining(body).to_vec());
        self.consumed.set(body.len());
    }

    /// `SG(read_post_bytes)`: body bytes read from the SAPI so far.
    pub fn read_bytes(&self) -> usize {
        self.consumed.get()
    }

    /// `sapi_read_standard_form_data`'s read loop (the declared length is
    /// within `post_max_size`): the rest of the body is read in
    /// [`SAPI_POST_BLOCK_SIZE`] blocks into a new `request_body`, stopping
    /// after the block that takes the bytes read past `post_max_size` (a
    /// body longer than its `Content-Length`). True when it stopped so: the
    /// caller raises [`actual_length_exceeds`] and still parses what was
    /// read.
    pub fn read_standard(&self, body: &[u8], post_max_size: i64) -> bool {
        let rest = self.remaining(body);
        let mut off = 0;
        let mut over = false;
        loop {
            let n = SAPI_POST_BLOCK_SIZE.min(rest.len() - off);
            off += n;
            self.consumed.set(self.consumed.get() + n);
            if post_max_size > 0 && self.consumed.get() as i64 > post_max_size {
                over = true;
                break;
            }
            if n < SAPI_POST_BLOCK_SIZE {
                break;
            }
        }
        *self.buffered.borrow_mut() = Some(rest[..off].to_vec());
        over
    }

    /// The buffered body (`request_body`), if a reader made one.
    pub fn buffered(&self) -> Option<Vec<u8>> {
        self.buffered.borrow().clone()
    }

    /// Record a multipart parse's outcome.
    pub fn apply(&mut self, out: &mut Outcome) {
        self.consumed.set(self.consumed.get() + out.consumed);
        if let Some(list) = out.uploaded.take() {
            if let Some(old) = self.uploaded.replace(list) {
                self.orphans.extend(old);
            }
        }
    }

    /// `destroy_uploaded_files_hash`: delete the uploads still in place.
    pub fn cleanup(&mut self) {
        let files = self.uploaded.take().into_iter().flatten();
        for f in files.chain(std::mem::take(&mut self.orphans)) {
            let _ = std::fs::remove_file(os_path(&f));
        }
    }

    fn is_uploaded(&self, path: &[u8]) -> bool {
        self.uploaded
            .as_ref()
            .is_some_and(|l| l.iter().any(|p| p == path))
    }

    fn forget(&mut self, path: &[u8]) {
        if let Some(l) = self.uploaded.as_mut() {
            l.retain(|p| p != path);
        }
    }
}

impl Drop for BodyState {
    fn drop(&mut self) {
        self.cleanup();
    }
}

use crate::zend::virtual_cwd::os_path;

/// `request_parse_body()`'s `$options` (`REQUEST_PARSE_BODY_OPTION_GET`):
/// a set field overrides the ini directive of the same name.
#[derive(Default, Clone, Copy)]
pub struct BodyOptions {
    pub max_file_uploads: Option<i64>,
    pub max_input_vars: Option<i64>,
    pub max_multipart_body_parts: Option<i64>,
    pub post_max_size: Option<i64>,
    pub upload_max_filesize: Option<i64>,
}

/// The limits one multipart parse runs under.
pub struct Limits {
    pub max_file_uploads: i64,
    pub max_input_vars: i64,
    pub max_parts: i64,
    pub post_max_size: i64,
    pub upload_max_filesize: i64,
    pub file_uploads: bool,
    pub upload_tmp_dir: Vec<u8>,
}

impl Limits {
    /// The ini settings, overridden by `request_parse_body()` options.
    pub fn new(host: &dyn Host, o: &BodyOptions) -> Limits {
        let ini = |n: &[u8]| host.ini_get(n).unwrap_or_default();
        let q = |n: &[u8]| crate::ini_parse_quantity(&ini(n));
        let fu = ini(b"file_uploads");
        Limits {
            max_file_uploads: o.max_file_uploads.unwrap_or_else(|| q(b"max_file_uploads")),
            max_input_vars: o.max_input_vars.unwrap_or_else(|| q(b"max_input_vars")),
            max_parts: o
                .max_multipart_body_parts
                .unwrap_or_else(|| q(b"max_multipart_body_parts")),
            post_max_size: o.post_max_size.unwrap_or_else(|| q(b"post_max_size")),
            upload_max_filesize: o
                .upload_max_filesize
                .unwrap_or_else(|| q(b"upload_max_filesize")),
            file_uploads: crate::ini_parse_bool(&fu),
            upload_tmp_dir: ini(b"upload_tmp_dir"),
        }
    }
}

/// A diagnostic the parser raised.
pub enum Diag {
    /// `EMIT_WARNING_OR_ERROR`: a warning, or under `request_parse_body()`
    /// a `RequestParseBodyException`.
    Emit(String),
    /// `php_error_docref(NULL, E_NOTICE, …)` (never an exception).
    Notice(String),
    /// `sapi_module.sapi_error(E_WARNING, …)`: a plain warning.
    Sapi(String),
}

/// What one [`parse`] did.
#[derive(Default)]
pub struct Outcome {
    /// Body bytes read from the SAPI.
    pub consumed: usize,
    /// The uploads (`SG(rfc1867_uploaded_files)`), when the parse got far
    /// enough to install the list.
    pub uploaded: Option<Vec<Vec<u8>>>,
    pub diags: Vec<Diag>,
}

/// `SG(request_info).content_type_dup` as `sapi_read_post_data` makes it:
/// the media type lowercased, the parameters as sent.
pub fn content_type_dup(ct: &[u8]) -> Vec<u8> {
    let mut out = ct.to_vec();
    for c in out.iter_mut() {
        if matches!(*c, b';' | b',' | b' ') {
            break;
        }
        *c = c.to_ascii_lowercase();
    }
    out
}

/// The media type `sapi_read_post_data` looks a POST handler up by.
pub fn media_type(ct: &[u8]) -> Vec<u8> {
    let end = ct
        .iter()
        .position(|&b| matches!(b, b';' | b',' | b' '))
        .unwrap_or(ct.len());
    ct[..end].to_ascii_lowercase()
}

fn c_isspace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// A C string's bytes: up to the first NUL.
fn cstr(s: &[u8]) -> &[u8] {
    match s.iter().position(|&b| b == 0) {
        Some(p) => &s[..p],
        None => s,
    }
}

/// `multipart_buffer`, reading from the body.
struct MBuf<'a> {
    src: &'a [u8],
    src_pos: usize,
    buffer: Vec<u8>,
    begin: usize,
    bytes: usize,
    bufsize: usize,
    boundary: Vec<u8>,
    boundary_next: Vec<u8>,
    /// `SG(read_post_bytes)` before this parse started reading.
    read_before: usize,
    /// `post_max_size` (<= 0: no limit).
    post_max_size: i64,
}

impl<'a> MBuf<'a> {
    fn new(src: &'a [u8], boundary: &[u8]) -> MBuf<'a> {
        let bufsize = (boundary.len() + 6).max(FILLUNIT);
        let mut b = b"--".to_vec();
        b.extend_from_slice(boundary);
        let mut bn = b"\n--".to_vec();
        bn.extend_from_slice(boundary);
        MBuf {
            src,
            src_pos: 0,
            buffer: vec![0; bufsize + 1],
            begin: 0,
            bytes: 0,
            bufsize,
            boundary: b,
            boundary_next: bn,
            read_before: 0,
            post_max_size: 0,
        }
    }

    /// `SG(read_post_bytes)`: everything read from the SAPI so far.
    fn read_post_bytes(&self) -> usize {
        self.read_before + self.src_pos
    }

    /// More than `post_max_size` bytes were read (the body was longer than
    /// its `Content-Length`).
    fn over_limit(&self) -> bool {
        self.post_max_size > 0 && self.read_post_bytes() as i64 > self.post_max_size
    }

    /// `fill_buffer`.
    fn fill(&mut self) -> usize {
        if self.bytes > 0 && self.begin != 0 {
            self.buffer
                .copy_within(self.begin..self.begin + self.bytes, 0);
        }
        self.begin = 0;
        // GHSA-2cv3-554h-jpmh: stop reading once over post_max_size.
        if self.over_limit() {
            return 0;
        }
        let want = self.bufsize - self.bytes;
        let n = want.min(self.src.len() - self.src_pos);
        self.buffer[self.bytes..self.bytes + n]
            .copy_from_slice(&self.src[self.src_pos..self.src_pos + n]);
        self.src_pos += n;
        self.bytes += n;
        n
    }

    /// `multipart_buffer_eof`.
    fn eof(&mut self) -> bool {
        self.bytes == 0 && self.fill() < 1
    }

    /// `next_line`.
    fn next_line(&mut self) -> Option<Vec<u8>> {
        let data = &self.buffer[self.begin..self.begin + self.bytes];
        match data.iter().position(|&b| b == b'\n') {
            Some(off) => {
                let mut end = off;
                if off > 0 && data[off - 1] == b'\r' {
                    end -= 1;
                }
                let line = data[..end].to_vec();
                self.begin += off + 1;
                self.bytes -= off + 1;
                Some(line)
            }
            None => {
                if self.bytes < self.bufsize {
                    return None;
                }
                let line = self.buffer[self.begin..self.begin + self.bufsize].to_vec();
                self.bytes = 0;
                Some(line)
            }
        }
    }

    /// `get_line`.
    fn get_line(&mut self) -> Option<Vec<u8>> {
        if let Some(l) = self.next_line() {
            return Some(l);
        }
        self.fill();
        self.next_line()
    }

    /// `find_boundary`.
    fn find_boundary(&mut self) -> bool {
        while let Some(line) = self.get_line() {
            if cstr(&line) == &self.boundary[..] {
                return true;
            }
        }
        false
    }

    /// `multipart_buffer_headers`: `None` when no boundary was found.
    fn headers(&mut self) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
        if !self.find_boundary() {
            return None;
        }
        let mut out = Vec::new();
        let mut key: Option<Vec<u8>> = None;
        let mut buf_value: Option<Vec<u8>> = None;
        while let Some(line) = self.get_line() {
            let line = cstr(&line).to_vec();
            if line.is_empty() {
                break;
            }
            let colon = if !c_isspace(line[0]) {
                line.iter().position(|&b| b == b':')
            } else {
                None
            };
            if let Some(c) = colon {
                if let (Some(v), Some(k)) = (buf_value.take(), key.take()) {
                    out.push((k, v));
                }
                let mut v = c + 1;
                while v < line.len() && c_isspace(line[v]) {
                    v += 1;
                }
                key = Some(line[..c].to_vec());
                buf_value = Some(line[v..].to_vec());
            } else if let Some(b) = buf_value.as_mut() {
                b.extend_from_slice(&line);
            }
        }
        if let (Some(v), Some(k)) = (buf_value, key) {
            out.push((k, v));
        }
        Some(out)
    }

    /// `multipart_buffer_read`: up to `bytes - 1` bytes of the part's body,
    /// stopping before a (possible) boundary.
    fn read(&mut self, bytes: usize, end: Option<&mut bool>) -> Vec<u8> {
        if bytes > self.bytes {
            self.fill();
        }
        let hay = &self.buffer[self.begin..self.begin + self.bytes];
        let bound = memstr(hay, &self.boundary_next, true);
        let max = match bound {
            Some(b) => {
                if let Some(e) = end {
                    if memstr(hay, &self.boundary_next, false).is_some() {
                        *e = true;
                    }
                }
                b
            }
            None => self.bytes,
        };
        let mut len = max.min(bytes - 1);
        let mut out = Vec::new();
        if len > 0 {
            out.extend_from_slice(&hay[..len]);
            if bound.is_some() && out[len - 1] == b'\r' {
                len -= 1;
                out.truncate(len);
            }
            self.bytes -= len;
            self.begin += len;
        }
        out
    }

    /// `multipart_buffer_read_body`.
    fn read_body(&mut self) -> Option<Vec<u8>> {
        let mut out: Option<Vec<u8>> = None;
        loop {
            let chunk = self.read(FILLUNIT, None);
            if chunk.is_empty() {
                break;
            }
            out.get_or_insert_with(Vec::new).extend_from_slice(&chunk);
        }
        out
    }
}

/// `php_ap_memstr`: the first match of `needle`, or with `partial` also a
/// prefix of it at the very end of the haystack.
fn memstr(hay: &[u8], needle: &[u8], partial: bool) -> Option<usize> {
    let mut i = 0;
    while i < hay.len() {
        let off = hay[i..].iter().position(|&b| b == needle[0])?;
        i += off;
        let len = hay.len() - i;
        let n = needle.len().min(len);
        if hay[i..i + n] == needle[..n] && (partial || len >= needle.len()) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// `php_ap_getword`: up to `stop` (outside quotes); the rest after any run
/// of `stop`s.
fn getword(line: &[u8], stop: u8) -> (Vec<u8>, &[u8]) {
    let mut pos = 0;
    while pos < line.len() && line[pos] != stop {
        let q = line[pos];
        if q == b'"' || q == b'\'' {
            pos += 1;
            while pos < line.len() && line[pos] != q {
                if line[pos] == b'\\' && pos + 1 < line.len() && line[pos + 1] == q {
                    pos += 2;
                } else {
                    pos += 1;
                }
            }
            if pos < line.len() {
                pos += 1;
            }
        } else {
            pos += 1;
        }
    }
    if pos >= line.len() {
        return (line.to_vec(), &line[line.len()..]);
    }
    let word = line[..pos].to_vec();
    while pos < line.len() && line[pos] == stop {
        pos += 1;
    }
    (word, &line[pos..])
}

/// `substring_conf`.
fn substring_conf(s: &[u8], len: usize, quote: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < len && i < s.len() && s[i] != quote {
        if s[i] == b'\\'
            && i + 1 < s.len()
            && (s[i + 1] == b'\\' || (quote != 0 && s[i + 1] == quote))
        {
            i += 1;
            out.push(s[i]);
        } else {
            out.push(s[i]);
        }
        i += 1;
    }
    out
}

/// `php_ap_getword_conf`: a (possibly quoted) parameter value.
fn getword_conf(s: &[u8]) -> Vec<u8> {
    let mut p = 0;
    while p < s.len() && c_isspace(s[p]) {
        p += 1;
    }
    let s = &s[p..];
    if s.is_empty() {
        return Vec::new();
    }
    if s[0] == b'"' || s[0] == b'\'' {
        let q = s[0];
        let rest = &s[1..];
        return substring_conf(rest, rest.len(), q);
    }
    let end = s.iter().position(|&c| c_isspace(c)).unwrap_or(s.len());
    substring_conf(s, end, 0)
}

/// `php_ap_basename`: after the last `/` or `\`.
fn basename(path: &[u8]) -> &[u8] {
    match path.iter().rposition(|&b| b == b'/' || b == b'\\') {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

/// `normalize_protected_variable`.
fn normalize_protected(var: &[u8]) -> Vec<u8> {
    let lead = var.iter().take_while(|&&b| b == b' ').count();
    let mut v = var[lead..].to_vec();
    let mut i = 0;
    while i < v.len() && v[i] != b'[' {
        if v[i] == b' ' || v[i] == b'.' {
            v[i] = b'_';
        }
        i += 1;
    }
    let Some(open) = v.iter().position(|&b| b == b'[') else {
        return v;
    };
    // s: the write position; index: where the current index text starts.
    let mut s = open + 1;
    let mut index = Some(s);
    while let Some(mut ix) = index {
        while ix < v.len() && matches!(v[ix], b' ' | b'\r' | b'\n' | b'\t') {
            ix += 1;
        }
        let iend = match v[ix..].iter().position(|&b| b == b']') {
            Some(o) => ix + o + 1,
            None => v.len(),
        };
        if s != ix {
            v.copy_within(ix.., s);
            v.truncate(v.len() - (ix - s));
            s += iend - ix;
        } else {
            s = iend;
        }
        if s < v.len() && v[s] == b'[' {
            s += 1;
            index = Some(s);
        } else {
            index = None;
        }
    }
    v.truncate(s);
    v
}

/// `strtoll(s, NULL, 10)`.
fn strtoll(s: &[u8]) -> i64 {
    let s = cstr(s);
    let mut i = 0;
    while i < s.len() && c_isspace(s[i]) {
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
    let mut v: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        let d = (s[i] - b'0') as i64;
        v = match v.checked_mul(10).and_then(|x| {
            if neg {
                x.checked_sub(d)
            } else {
                x.checked_add(d)
            }
        }) {
            Some(x) => x,
            None => return if neg { i64::MIN } else { i64::MAX },
        };
        i += 1;
    }
    v
}

/// `php_open_temporary_fd_ex(upload_tmp_dir, "php", …)`: the new file's
/// path (`None`: it could not be created).
fn open_upload_temp(dir: &[u8], diags: &mut Vec<Diag>) -> Option<Vec<u8>> {
    if !dir.is_empty() {
        if let Some(p) = crate::fs_fns::open_temp(dir, b"php") {
            return Some(p);
        }
        diags.push(Diag::Notice(
            "file created in the system's temporary directory".into(),
        ));
    }
    crate::fs_fns::open_temp(&crate::fs_fns::temp_dir(), b"php")
}

/// `rfc1867_post_handler`: parse a multipart body (`src`, what is left of
/// the request body; `read_before`: the body bytes read before it, php's
/// `SG(read_post_bytes)`) into `post` / `files`.
pub fn parse(
    src: &[u8],
    read_before: usize,
    content_type_dup: &[u8],
    content_length: i64,
    lim: &Limits,
    post: &mut PhpArray,
    files: &mut PhpArray,
) -> Outcome {
    let mut out = Outcome::default();
    let mut upload_cnt = lim.max_file_uploads;
    let mut body_parts_cnt = lim.max_parts;
    let max_input_vars = lim.max_input_vars;
    if lim.post_max_size > 0 && content_length > lim.post_max_size {
        out.diags.push(Diag::Emit(format!(
            "POST Content-Length of {content_length} bytes exceeds the limit of {} bytes",
            lim.post_max_size
        )));
        return out;
    }
    if body_parts_cnt < 0 {
        body_parts_cnt = max_input_vars.wrapping_add(upload_cnt);
    }
    let body_parts_limit = body_parts_cnt as i32;

    // The boundary.
    let ct = cstr(content_type_dup);
    let at = find(ct, b"boundary").or_else(|| find(&ct.to_ascii_lowercase(), b"boundary"));
    let Some(eq) = at.and_then(|a| ct[a..].iter().position(|&b| b == b'=').map(|e| a + e)) else {
        out.diags.push(Diag::Emit(
            "Missing boundary in multipart/form-data POST data".into(),
        ));
        return out;
    };
    let mut boundary = &ct[eq + 1..];
    if boundary.first() == Some(&b'"') {
        boundary = &boundary[1..];
        match boundary.iter().position(|&b| b == b'"') {
            Some(e) => boundary = &boundary[..e],
            None => {
                out.diags.push(Diag::Emit(
                    "Invalid boundary in multipart/form-data POST data".into(),
                ));
                return out;
            }
        }
    } else if let Some(e) = boundary.iter().position(|&b| b == b',' || b == b';') {
        boundary = &boundary[..e];
    }
    if boundary.len() > FILLUNIT - 4 {
        out.diags.push(Diag::Sapi(
            "Boundary too large in multipart/form-data POST data".into(),
        ));
        return out;
    }

    let mut mb = MBuf::new(src, boundary);
    mb.read_before = read_before;
    mb.post_max_size = lim.post_max_size;
    let mut protected: HashSet<Vec<u8>> = HashSet::new();
    let mut uploaded: Vec<Vec<u8>> = Vec::new();
    let mut count: i64 = 0;
    let mut max_file_size: i64 = 0;
    let mut anonymous_index: u32 = 0;
    let mut skip_upload = false;

    while !mb.eof() {
        let Some(header) = mb.headers() else {
            break;
        };
        let hdr = |name: &[u8]| -> Option<Vec<u8>> {
            header
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        };
        let Some(cd) = hdr(b"Content-Disposition") else {
            continue;
        };
        body_parts_cnt -= 1;
        if body_parts_cnt < 0 {
            out.diags.push(Diag::Emit(format!(
                "Multipart body parts limit exceeded {body_parts_limit}. To increase the limit change max_multipart_body_parts in php.ini."
            )));
            break;
        }
        let mut param: Option<Vec<u8>> = None;
        let mut filename: Option<Vec<u8>> = None;
        let mut cd: &[u8] = &cd;
        while !cd.is_empty() && c_isspace(cd[0]) {
            cd = &cd[1..];
        }
        while !cd.is_empty() {
            let (pair, rest) = getword(cd, b';');
            cd = rest;
            while !cd.is_empty() && c_isspace(cd[0]) {
                cd = &cd[1..];
            }
            if pair.contains(&b'=') {
                let (key, val) = getword(&pair, b'=');
                if key.eq_ignore_ascii_case(b"name") {
                    param = Some(getword_conf(val));
                } else if key.eq_ignore_ascii_case(b"filename") {
                    filename = Some(getword_conf(val));
                }
            }
        }

        // A normal form variable.
        if let (None, Some(p)) = (&filename, param.clone()) {
            let value = mb.read_body().unwrap_or_default();
            count += 1;
            if count <= max_input_vars {
                // php 8.5 registers form fields under the name as sent (no
                // protected-variable normalisation or check).
                register_variable_value(post, &p, Value::str(value.clone()), false);
            } else if count == max_input_vars.wrapping_add(1) {
                out.diags.push(Diag::Emit(format!(
                    "Input variables exceeded {max_input_vars}. To increase the limit change max_input_vars in php.ini."
                )));
            }
            if p.eq_ignore_ascii_case(b"MAX_FILE_SIZE") {
                max_file_size = strtoll(&value);
            }
            continue;
        }

        // A file part.
        if !lim.file_uploads {
            skip_upload = true;
        } else if upload_cnt <= 0 {
            skip_upload = true;
            if upload_cnt == 0 {
                upload_cnt -= 1;
                out.diags.push(Diag::Emit(
                    "Maximum number of allowable file uploads has been exceeded".into(),
                ));
            }
        }
        if param.is_none() && filename.is_none() {
            out.diags
                .push(Diag::Emit("File Upload Mime headers garbled".into()));
            break;
        }
        let mut param = match param {
            Some(p) => p,
            None => {
                let p = anonymous_index.to_string().into_bytes();
                anonymous_index += 1;
                p
            }
        };
        let filename = filename.unwrap_or_default();
        // Never repair potentially malicious input: unbalanced brackets or
        // text after a ']' skip the upload.
        if !skip_upload {
            let mut c: i64 = 0;
            let mut i = 0;
            while i < param.len() {
                if param[i] == b'[' {
                    c += 1;
                } else if param[i] == b']' {
                    c -= 1;
                    if i + 1 < param.len() && param[i + 1] != b'[' {
                        skip_upload = true;
                        break;
                    }
                }
                if c < 0 {
                    skip_upload = true;
                    break;
                }
                i += 1;
            }
            if c != 0 {
                skip_upload = true;
            }
        }
        if skip_upload {
            continue;
        }

        let mut cancel: i64 = 0;
        let mut total: i64 = 0;
        let mut temp: Option<Vec<u8>> = None;
        let mut end = false;
        if filename.is_empty() {
            cancel = UPLOAD_ERR_NO_FILE;
        }
        let mut file: Option<std::fs::File> = None;
        let mut blen_buf = Vec::new();
        if cancel == 0 {
            blen_buf = mb.read(FILLUNIT, Some(&mut end));
            temp = open_upload_temp(&lim.upload_tmp_dir, &mut out.diags);
            upload_cnt -= 1;
            match &temp {
                Some(t) => {
                    file = std::fs::OpenOptions::new()
                        .write(true)
                        .open(os_path(t))
                        .ok();
                    if file.is_none() {
                        cancel = UPLOAD_ERR_CANT_WRITE;
                    }
                }
                None => {
                    out.diags.push(Diag::Emit(
                        "File upload error - unable to create a temporary file".into(),
                    ));
                    cancel = 6;
                }
            }
        }
        while cancel == 0 && !blen_buf.is_empty() {
            let blen = blen_buf.len() as i64;
            if lim.upload_max_filesize > 0 && total + blen > lim.upload_max_filesize {
                cancel = UPLOAD_ERR_INI_SIZE;
            } else if max_file_size != 0 && total + blen > max_file_size {
                cancel = UPLOAD_ERR_FORM_SIZE;
            } else {
                use std::io::Write;
                match file.as_mut().map(|f| f.write_all(&blen_buf)) {
                    Some(Ok(())) => total += blen,
                    _ => cancel = UPLOAD_ERR_CANT_WRITE,
                }
            }
            blen_buf = mb.read(FILLUNIT, Some(&mut end));
        }
        drop(file);
        if cancel == 0 && !end {
            cancel = UPLOAD_ERR_PARTIAL;
        }
        if cancel != 0 {
            if let Some(t) = temp.take() {
                if cancel != 6 {
                    let _ = std::fs::remove_file(os_path(&t));
                }
            }
        } else if let Some(t) = &temp {
            uploaded.push(t.clone());
        }

        // is_arr_upload: the name ends in [...]; start_arr is its first '['.
        let start_arr = param.iter().position(|&b| b == b'[');
        let is_arr = start_arr.is_some() && param.last() == Some(&b']');
        let (abuf, array_index) = match start_arr.filter(|_| is_arr) {
            Some(sa) => (
                param[..sa].to_vec(),
                param[sa + 1..param.len() - 1].to_vec(),
            ),
            None => (Vec::new(), Vec::new()),
        };
        let lbuf = |param: &[u8], field: &str| -> Vec<u8> {
            let mut l = Vec::new();
            if is_arr {
                l.extend_from_slice(&abuf);
                l.push(b'[');
                l.extend_from_slice(field.as_bytes());
                l.extend_from_slice(b"][");
                l.extend_from_slice(&array_index);
                l.push(b']');
            } else {
                l.extend_from_slice(param);
                l.push(b'[');
                l.extend_from_slice(field.as_bytes());
                l.push(b']');
            }
            l
        };
        let reg = |files: &mut PhpArray,
                   protected: &HashSet<Vec<u8>>,
                   var: Vec<u8>,
                   v: Value,
                   force: bool| {
            let var = if force {
                var
            } else {
                normalize_protected(&var)
            };
            if force || !protected.contains(&var) {
                register_variable_value(files, &var, v, false);
            }
        };
        let s = basename(cstr(&filename)).to_vec();
        reg(
            files,
            &protected,
            lbuf(&param, "name"),
            Value::str(s),
            false,
        );
        reg(
            files,
            &protected,
            lbuf(&param, "full_path"),
            Value::str(cstr(&filename).to_vec()),
            false,
        );
        let ty = match hdr(b"Content-Type").filter(|_| cancel == 0) {
            Some(t) => match t.iter().position(|&b| b == b';') {
                Some(p) => t[..p].to_vec(),
                None => t,
            },
            None => Vec::new(),
        };
        reg(
            files,
            &protected,
            lbuf(&param, "type"),
            Value::str(ty),
            false,
        );
        // add_protected_variable(param) normalises param in place.
        param = normalize_protected(&param);
        protected.insert(param.clone());
        let tmp_var = normalize_protected(&lbuf(&param, "tmp_name"));
        protected.insert(tmp_var.clone());
        reg(
            files,
            &protected,
            tmp_var,
            Value::str(temp.clone().unwrap_or_default()),
            true,
        );
        reg(
            files,
            &protected,
            lbuf(&param, "error"),
            Value::Int(cancel),
            false,
        );
        let size = if cancel != 0 { 0 } else { total };
        reg(
            files,
            &protected,
            lbuf(&param, "size"),
            Value::Int(size),
            false,
        );
    }
    // fileupload_done.
    if mb.over_limit() {
        out.diags
            .push(Diag::Sapi(actual_length_exceeds(lim.post_max_size)));
    }
    out.consumed = mb.src_pos;
    out.uploaded = Some(uploaded);
    out
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

// ---------------------------------------------------------------------------
// builtins
// ---------------------------------------------------------------------------

/// A `Z_PARAM_PATH` argument: a string without NUL bytes.
fn path_param(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<Vec<u8>, Thrown> {
    let p = crate::zend::zpp::str_param(host, args, i, fname, pname)?;
    if p.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{fname}(): Argument #{} (${pname}) must not contain any null bytes",
                i + 1
            ),
        ));
    }
    Ok(p)
}

fn fn_is_uploaded_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(host, args, 0, "is_uploaded_file", "filename")?;
    Ok(Value::Bool(
        host.body_state().is_some_and(|s| s.is_uploaded(&path)),
    ))
}

fn fn_move_uploaded_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let from = path_param(host, args, 0, "move_uploaded_file", "from")?;
    let to = path_param(host, args, 1, "move_uploaded_file", "to")?;
    if !host.body_state().is_some_and(|s| s.is_uploaded(&from)) {
        return Ok(Value::Bool(false));
    }
    let mut ok = false;
    if std::fs::rename(os_path(&from), os_path(&to)).is_ok() {
        ok = true;
        use std::os::unix::fs::PermissionsExt;
        let mode = 0o666 & !crate::zend::virtual_cwd::umask();
        if let Err(e) =
            std::fs::set_permissions(os_path(&to), std::fs::Permissions::from_mode(mode))
        {
            let msg = crate::streams::strerror(e.raw_os_error().unwrap_or(0));
            crate::warn(host, "move_uploaded_file", &msg)?;
        }
    } else {
        match std::fs::read(os_path(&from)) {
            Err(e) => {
                crate::streams::note_io_err(&e);
                crate::open_failed(host, "move_uploaded_file", &from)?;
            }
            Ok(data) => match crate::zend::virtual_cwd::write_file(&to, &data) {
                Ok(()) => {
                    let _ = std::fs::remove_file(os_path(&from));
                    ok = true;
                }
                Err(e) => {
                    crate::streams::note_io_err(&e);
                    crate::open_failed(host, "move_uploaded_file", &to)?;
                }
            },
        }
    }
    if ok {
        if let Some(s) = host.body_state() {
            s.forget(&from);
        }
    } else {
        crate::warn(
            host,
            "move_uploaded_file",
            &format!(
                "Unable to move \"{}\" to \"{}\"",
                String::from_utf8_lossy(&from),
                String::from_utf8_lossy(&to)
            ),
        )?;
    }
    Ok(Value::Bool(ok))
}

/// `smart_str_append_escaped`.
fn escaped(s: &[u8]) -> String {
    let mut o = String::new();
    for &c in s {
        match c {
            b'\n' => o.push_str("\\n"),
            b'\r' => o.push_str("\\r"),
            b'\t' => o.push_str("\\t"),
            0x0c => o.push_str("\\f"),
            0x0b => o.push_str("\\v"),
            b'\\' => o.push_str("\\\\"),
            0x1b => o.push_str("\\e"),
            c if !(32..127).contains(&c) => o.push_str(&format!("\\x{c:02X}")),
            c => o.push(c as char),
        }
    }
    o
}

/// The warning `zend_ini_parse_quantity` reports for a malformed quantity.
pub(crate) fn quantity_error(v: &[u8]) -> Option<String> {
    let ws = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    let mut a = 0;
    let mut b = v.len();
    while a < b && ws(v[a]) {
        a += 1;
    }
    while b > a && ws(v[b - 1]) {
        b -= 1;
    }
    if a == b {
        return None;
    }
    let inv = escaped(v);
    let mut d = a;
    let negative = v[d] == b'-';
    if v[d] == b'+' || v[d] == b'-' {
        d += 1;
    }
    let no_digits = || {
        Some(format!(
            "Invalid quantity \"{inv}\": no valid leading digits, interpreting as \"0\" for backwards compatibility"
        ))
    };
    if d >= b || !v[d].is_ascii_digit() {
        return no_digits();
    }
    let mut base_digits: fn(u8) -> bool = |c: u8| c.is_ascii_digit();
    // ZEND_STRTOUL's base: 0 (decimal, or octal after a leading 0) unless
    // an explicit prefix picked one.
    let mut radix = if v[d] == b'0' { 8 } else { 10 };
    if v[d] == b'0' && d + 1 < b && !v[d + 1].is_ascii_digit() {
        match v[d + 1] {
            b'g' | b'G' | b'm' | b'M' | b'k' | b'K' => {}
            b'x' | b'X' => {
                base_digits = |c| c.is_ascii_hexdigit();
                radix = 16;
                d += 2;
            }
            b'o' | b'O' => {
                base_digits = |c| (b'0'..=b'7').contains(&c);
                radix = 8;
                d += 2;
            }
            b'b' | b'B' => {
                base_digits = |c| c == b'0' || c == b'1';
                radix = 2;
                d += 2;
            }
            c => {
                return Some(format!(
                    "Invalid prefix \"0{}\", interpreting as \"0\" for backwards compatibility",
                    c as char
                ))
            }
        }
        if d >= b || !base_digits(v[d]) {
            return Some(format!(
                "Invalid quantity \"{inv}\": no digits after base prefix, interpreting as \"0\" for backwards compatibility"
            ));
        }
    }
    let mut e = d;
    while e < b && base_digits(v[e]) {
        e += 1;
    }
    // strtoul's ERANGE, then the signed-result check (only PHP_INT_MIN may
    // have its top bit set).
    let mut value: u64 = 0;
    let mut overflow = false;
    for &c in &v[d..e] {
        let digit = (c as char).to_digit(radix);
        let Some(digit) = digit else { break };
        match value
            .checked_mul(u64::from(radix))
            .and_then(|x| x.checked_add(u64::from(digit)))
        {
            Some(x) => value = x,
            None => {
                overflow = true;
                value = u64::MAX;
            }
        }
    }
    let mut signed = value as i64;
    if !overflow {
        if negative && value == 1u64 << 63 {
            signed = i64::MIN;
        } else if signed < 0 {
            overflow = true;
        } else if negative {
            signed = -signed;
        }
    }
    let out_of_range = || {
        Some(format!(
            "Invalid quantity \"{inv}\": value is out of range, using overflow result for backwards compatibility"
        ))
    };
    while e < b && ws(v[e]) {
        e += 1;
    }
    if e == b {
        return if overflow { out_of_range() } else { None };
    }
    let last = v[b - 1];
    let interp = escaped(&v[..e]);
    let chr = escaped(&[last]);
    let factor: i64 = match last {
        b'g' | b'G' => 1 << 30,
        b'm' | b'M' => 1 << 20,
        b'k' | b'K' => 1 << 10,
        _ => {
            return Some(format!(
                "Invalid quantity \"{inv}\": unknown multiplier \"{chr}\", interpreting as \"{interp}\" for backwards compatibility"
            ))
        }
    };
    if !overflow {
        overflow = if signed > 0 {
            signed > i64::MAX / factor
        } else {
            signed < i64::MIN / factor
        };
    }
    if e != b - 1 {
        return Some(format!(
            "Invalid quantity \"{inv}\", interpreting as \"{interp}{chr}\" for backwards compatibility"
        ));
    }
    if overflow {
        return out_of_range();
    }
    None
}

/// `cache_request_parse_body_options`.
fn parse_options(host: &mut dyn Host, opts: &PhpArray) -> Result<BodyOptions, Thrown> {
    let mut o = BodyOptions::default();
    for (k, v) in opts.iter() {
        let ArrayKey::Str(k) = k else {
            return Err(Thrown::error(
                "ValueError",
                "Invalid integer key in $options argument",
            ));
        };
        let k = k.as_bytes();
        if k.is_empty() {
            return Err(Thrown::error(
                "ValueError",
                "Invalid empty string key in $options argument",
            ));
        }
        let lk = k.to_ascii_lowercase();
        let slot = match &lk[..] {
            b"max_file_uploads" => &mut o.max_file_uploads,
            b"max_input_vars" => &mut o.max_input_vars,
            b"max_multipart_body_parts" => &mut o.max_multipart_body_parts,
            b"post_max_size" => &mut o.post_max_size,
            b"upload_max_filesize" => &mut o.upload_max_filesize,
            _ => {
                return Err(Thrown::error(
                    "ValueError",
                    format!(
                        "Invalid key \"{}\" in $options argument",
                        String::from_utf8_lossy(cstr(k))
                    ),
                ))
            }
        };
        let v = deref(v);
        *slot = Some(match &v {
            Value::Int(i) => *i,
            Value::Str(s) => {
                if let Some(err) = quantity_error(s.as_bytes()) {
                    host.emit_diagnostic(crate::E_WARNING, &err)?;
                }
                crate::ini_parse_quantity(s.as_bytes())
            }
            other => {
                return Err(Thrown::error(
                    "ValueError",
                    format!(
                        "Invalid {} value in $options argument",
                        crate::zend::zpp::zval_value_name(host, other)
                    ),
                ))
            }
        });
    }
    Ok(o)
}

/// `new RequestParseBodyException($msg)`.
fn parse_body_exception(host: &mut dyn Host, msg: String) -> Thrown {
    match host.reflect_new_instance(
        b"RequestParseBodyException",
        vec![Value::str(msg.clone().into_bytes())],
        true,
    ) {
        Ok(Value::Object(h)) => Thrown::Object(h),
        Ok(_) => Thrown::error("RequestParseBodyException", msg),
        Err(t) => t,
    }
}

fn fn_request_parse_body(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let opts = match args.first().map(deref) {
        None | Some(Value::Null) => BodyOptions::default(),
        Some(Value::Array(a)) => parse_options(host, &a)?,
        Some(other) => {
            return Err(Thrown::type_error(format!(
                "request_parse_body(): Argument #1 ($options) must be of type ?array, {} given",
                crate::zend::zpp::zval_value_name(host, &other)
            )))
        }
    };
    let info = host.request_info().and_then(|r| {
        r.content_type.clone().map(|ct| {
            (
                ct,
                r.method.clone(),
                r.content_length.unwrap_or(r.body.len() as i64),
                r.body.clone(),
            )
        })
    });
    let Some((ct, method, content_length, body)) = info else {
        return Err(parse_body_exception(
            host,
            "Request does not provide a content type".into(),
        ));
    };
    let is_post = method.as_deref() == Some(b"POST");
    let kind = media_type(&ct);
    let lim = Limits::new(host, &opts);
    let mut post = PhpArray::new();
    let mut files = PhpArray::new();
    // sapi_read_standard_form_data.
    let read_standard = |host: &mut dyn Host| -> Result<bool, Thrown> {
        if lim.post_max_size > 0 && content_length > lim.post_max_size {
            crate::warn(
                host,
                "request_parse_body",
                &format!(
                    "POST Content-Length of {content_length} bytes exceeds the limit of {} bytes",
                    lim.post_max_size
                ),
            )?;
            return Ok(false);
        }
        let over = host
            .body_state()
            .is_some_and(|s| s.read_standard(&body, lim.post_max_size));
        if over {
            crate::warn(
                host,
                "request_parse_body",
                &actual_length_exceeds(lim.post_max_size),
            )?;
        }
        Ok(true)
    };
    match &kind[..] {
        b"application/x-www-form-urlencoded" => {
            if read_standard(host)? {
                let data = host
                    .body_state()
                    .and_then(|s| s.buffered())
                    .unwrap_or_default();
                let (a, over) = post_form_vars(&data, lim.max_input_vars);
                post = a;
                if over {
                    crate::warn(
                        host,
                        "request_parse_body",
                        &format!(
                            "Input variables exceeded {}. To increase the limit change max_input_vars in php.ini.",
                            lim.max_input_vars as u64
                        ),
                    )?;
                }
            }
        }
        b"multipart/form-data" => {
            let (rest, read_before) = match host.body_state() {
                Some(s) => (s.remaining(&body).to_vec(), s.read_bytes()),
                None => (Vec::new(), 0),
            };
            let mut out = parse(
                &rest,
                read_before,
                &content_type_dup(&ct),
                content_length,
                &lim,
                &mut post,
                &mut files,
            );
            if let Some(s) = host.body_state() {
                s.apply(&mut out);
            }
            let mut thrown: Option<Thrown> = None;
            for d in out.diags {
                match d {
                    Diag::Emit(m) => {
                        if thrown.is_none() {
                            thrown = Some(parse_body_exception(host, m));
                        }
                    }
                    Diag::Notice(m) => crate::notice(host, "request_parse_body", &m)?,
                    Diag::Sapi(m) => host.emit_diagnostic(crate::E_WARNING, &m)?,
                }
            }
            if let Some(t) = thrown {
                return Err(t);
            }
        }
        _ => {
            if is_post {
                read_standard(host)?;
            }
            return Err(parse_body_exception(
                host,
                format!(
                    "Content-Type \"{}\" is not supported",
                    String::from_utf8_lossy(cstr(&ct))
                ),
            ));
        }
    }
    let mut pair = PhpArray::new();
    pair.push(Value::Array(post));
    pair.push(Value::Array(files));
    Ok(Value::Array(pair))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("is_uploaded_file", fn_is_uploaded_file),
            ("move_uploaded_file", fn_move_uploaded_file),
            ("request_parse_body", fn_request_parse_body),
        ],
    );
}

#[cfg(test)]
mod tests;
