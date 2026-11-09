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

use rhp_values::{mem, PhpArray, Value};

use crate::main::php_variables::register_variable_value;
use crate::Host;

/// `FILLUNIT`: the multipart buffer's minimum size.
const FILLUNIT: usize = 1024 * 5;

/// `SAPI_POST_BLOCK_SIZE`: the block `sapi_read_standard_form_data` reads
/// the body in.
pub const SAPI_POST_BLOCK_SIZE: usize = 0x4000;

/// The warning php raises once more than `post_max_size` bytes of a body
/// were actually read, whatever its `Content-Length` said
/// (GHSA-2cv3-554h-jpmh).
pub fn actual_length_exceeds(post_max_size: i64) -> String {
    format!("Actual POST length does not match Content-Length, and exceeds {post_max_size} bytes")
}

/// `UPLOAD_ERR_*`.
const UPLOAD_ERR_OK: i64 = 0;
const UPLOAD_ERR_INI_SIZE: i64 = 1;
const UPLOAD_ERR_FORM_SIZE: i64 = 2;
const UPLOAD_ERR_PARTIAL: i64 = 3;
const UPLOAD_ERR_NO_FILE: i64 = 4;
const UPLOAD_ERR_NO_TMP_DIR: i64 = 6;
const UPLOAD_ERR_CANT_WRITE: i64 = 7;
const UPLOAD_ERR_EXTENSION: i64 = 8;

/// php_rfc1867_register_constants.
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("UPLOAD_ERR_OK", Value::Int(UPLOAD_ERR_OK)),
        ("UPLOAD_ERR_INI_SIZE", Value::Int(UPLOAD_ERR_INI_SIZE)),
        ("UPLOAD_ERR_FORM_SIZE", Value::Int(UPLOAD_ERR_FORM_SIZE)),
        ("UPLOAD_ERR_PARTIAL", Value::Int(UPLOAD_ERR_PARTIAL)),
        ("UPLOAD_ERR_NO_FILE", Value::Int(UPLOAD_ERR_NO_FILE)),
        ("UPLOAD_ERR_NO_TMP_DIR", Value::Int(UPLOAD_ERR_NO_TMP_DIR)),
        ("UPLOAD_ERR_CANT_WRITE", Value::Int(UPLOAD_ERR_CANT_WRITE)),
        ("UPLOAD_ERR_EXTENSION", Value::Int(UPLOAD_ERR_EXTENSION)),
    ]
}

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
    /// The temporary files `request_body` spilled into (a body of 16 KiB
    /// or more), deleted at request end.
    spills: RefCell<Vec<Vec<u8>>>,
    /// A body the SAPI streams ([`RequestInfo::body_reader`]): read from
    /// here, as php's `sapi_module.read_post`, instead of the in-memory
    /// body.
    ///
    /// [`RequestInfo::body_reader`]: crate::main::sapi::RequestInfo::body_reader
    stream: RefCell<Option<Box<dyn std::io::Read>>>,
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

    /// Read the body from `reader` instead of the in-memory body.
    pub fn set_stream(&mut self, reader: Box<dyn std::io::Read>) {
        *self.stream.get_mut() = Some(reader);
    }

    /// A reader of the body bytes the SAPI has not delivered yet, from
    /// `body` (the in-memory body) or the stream. It fills the buffer
    /// unless the body ends (php's SAPIs' `read_post` loop so). The bytes
    /// it hands out still count as unread until [`BodyState::apply`].
    pub fn reader<'a>(&'a self, body: &'a [u8]) -> impl FnMut(&mut [u8]) -> usize + 'a {
        let mut pos = self.consumed.get();
        move |buf: &mut [u8]| {
            let n = match self.stream.borrow_mut().as_mut() {
                Some(r) => read_full(r.as_mut(), buf),
                None => {
                    let rest = body.get(pos..).unwrap_or_default();
                    let n = buf.len().min(rest.len());
                    buf[..n].copy_from_slice(&rest[..n]);
                    n
                }
            };
            pos += n;
            n
        }
    }

    /// `sapi_read_post_block`: up to `buf.len()` body bytes, counted read.
    fn read_block(&self, body: &[u8], buf: &mut [u8]) -> usize {
        let n = self.reader(body)(buf);
        self.consumed.set(self.consumed.get() + n);
        n
    }

    /// The rest of the body, read.
    pub fn read_rest(&self, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buf = vec![0; SAPI_POST_BLOCK_SIZE];
        loop {
            let n = self.read_block(body, &mut buf);
            out.extend_from_slice(&buf[..n]);
            if n < buf.len() {
                return out;
            }
        }
    }

    /// `sapi_read_standard_form_data` (within `post_max_size`): the rest of
    /// the body becomes `request_body`.
    pub fn buffer_rest(&self, body: &[u8]) {
        let rest = self.read_rest(body);
        *self.buffered.borrow_mut() = Some(rest);
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
    ///
    /// `request_body` is a `php://temp` stream that keeps 16 KiB in memory:
    /// a body that reaches that size goes to a temporary file in
    /// `upload_tmp_dir` (else the system's). When none can be created the
    /// body is discarded, with php's warnings (`diags`).
    pub fn read_standard(&self, body: &[u8], post_max_size: i64, diags: &mut Vec<Diag>) -> bool {
        let mut data = Vec::new();
        let mut block = vec![0; SAPI_POST_BLOCK_SIZE];
        let mut over = false;
        let mut spilled = false;
        loop {
            let n = self.read_block(body, &mut block);
            // php_stream_temp_write: the write that reaches the memory
            // limit moves the stream to a file.
            if n > 0 && !spilled && data.len() + n >= SAPI_POST_BLOCK_SIZE {
                match self.spill(diags) {
                    Some(path) => {
                        spilled = true;
                        self.spills.borrow_mut().push(path);
                    }
                    None => {
                        diags.push(Diag::Warning(
                            "Unable to create temporary file, Check permissions in temporary files directory.".into(),
                        ));
                        diags.push(Diag::Warning(
                            "POST data can't be buffered; all data discarded".into(),
                        ));
                        *self.buffered.borrow_mut() = Some(Vec::new());
                        return false;
                    }
                }
            }
            data.extend_from_slice(&block[..n]);
            if post_max_size > 0 && self.consumed.get() as i64 > post_max_size {
                over = true;
                break;
            }
            if n < SAPI_POST_BLOCK_SIZE {
                break;
            }
        }
        if let Some(path) = self.spills.borrow().last().filter(|_| spilled) {
            // The file holds what php's stream holds; reads come from memory.
            let _ = std::fs::write(os_path(path), &data);
        }
        *self.buffered.borrow_mut() = Some(data);
        over
    }

    /// `php_stream_fopen_temporary_file(upload_tmp_dir, "php")`.
    fn spill(&self, diags: &mut Vec<Diag>) -> Option<Vec<u8>> {
        let dir = crate::ini_value(b"upload_tmp_dir")
            .flatten()
            .unwrap_or_default();
        open_upload_temp(&dir, diags)
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
        for f in self.spills.take() {
            let _ = std::fs::remove_file(os_path(&f));
        }
        let files = self.uploaded.take().into_iter().flatten();
        for f in files.chain(std::mem::take(&mut self.orphans)) {
            let _ = std::fs::remove_file(os_path(&f));
        }
    }

    pub(crate) fn is_uploaded(&self, path: &[u8]) -> bool {
        self.uploaded
            .as_ref()
            .is_some_and(|l| l.iter().any(|p| p == path))
    }

    pub(crate) fn forget(&mut self, path: &[u8]) {
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
    /// mbstring.encoding_translation.
    pub translate: bool,
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
            translate: cfg!(feature = "mbstring-libmbfl")
                && crate::ini_parse_bool(&ini(b"mbstring.encoding_translation")),
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
    /// `php_error_docref(NULL, E_WARNING, …)` (never an exception).
    Warning(String),
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
pub(crate) fn cstr(s: &[u8]) -> &[u8] {
    match s.iter().position(|&b| b == 0) {
        Some(p) => &s[..p],
        None => s,
    }
}

/// The encoding of a part's headers under mbstring.encoding_translation.
#[cfg(feature = "mbstring-libmbfl")]
type InEnc = crate::ext::mbstring::GpcEnc;
#[cfg(not(feature = "mbstring-libmbfl"))]
#[derive(Clone, Copy)]
struct InEnc;
#[cfg(not(feature = "mbstring-libmbfl"))]
impl InEnc {
    fn char_bytes(self, _: u8) -> usize {
        1
    }
    fn into_internal(self, s: &[u8]) -> Vec<u8> {
        s.to_vec()
    }
}

fn detect_encoding(_line: &[u8]) -> Option<InEnc> {
    #[cfg(feature = "mbstring-libmbfl")]
    return crate::ext::mbstring::gpc_detect(_line);
    #[cfg(not(feature = "mbstring-libmbfl"))]
    None
}

/// php_mb_mbchar_bytes for an optional encoding.
fn char_bytes(enc: Option<InEnc>, b: u8) -> usize {
    enc.map_or(1, |e| e.char_bytes(b)).max(1)
}

/// `multipart_buffer`, reading from the body.
struct MBuf<'a> {
    src: &'a mut dyn FnMut(&mut [u8]) -> usize,
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
    /// Detect each header line's encoding (encoding_translation).
    translate: bool,
    /// `input_encoding`: what the last header line was detected as.
    input_enc: Option<InEnc>,
}

impl<'a> MBuf<'a> {
    fn new(src: &'a mut dyn FnMut(&mut [u8]) -> usize, boundary: &[u8]) -> MBuf<'a> {
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
            translate: false,
            input_enc: None,
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
        let n = (self.src)(&mut self.buffer[self.bytes..self.bytes + want]);
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

    /// `multipart_buffer_headers`: `None` when no boundary was found or the
    /// headers reached the memory limit.
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
            if self.translate {
                self.input_enc = detect_encoding(&line);
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
                // Keep only the headers the parser uses, and the first of
                // each.
                let name = &line[..c];
                let save = (name.eq_ignore_ascii_case(b"Content-Disposition")
                    || name.eq_ignore_ascii_case(b"Content-Type"))
                    && !out.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));
                if save {
                    let mut v = c + 1;
                    while v < line.len() && c_isspace(line[v]) {
                        v += 1;
                    }
                    key = Some(name.to_vec());
                    buf_value = Some(line[v..].to_vec());
                }
            } else if let Some(b) = buf_value.as_mut() {
                b.extend_from_slice(&line);
            } else {
                continue;
            }
            if mem::has_oom() {
                return None;
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
fn getword(line: &[u8], stop: u8, enc: Option<InEnc>) -> (Vec<u8>, &[u8]) {
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
            pos += char_bytes(enc, line[pos]);
        }
    }
    if pos >= line.len() {
        return (line.to_vec(), &line[line.len()..]);
    }
    let word = line[..pos].to_vec();
    while pos < line.len() && line[pos] == stop {
        pos += char_bytes(enc, line[pos]);
    }
    let pos = pos.min(line.len());
    (word, &line[pos..])
}

/// `substring_conf`.
fn substring_conf(s: &[u8], len: usize, quote: u8, enc: Option<InEnc>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < len && i < s.len() && s[i] != quote {
        if s[i] == b'\\'
            && i + 1 < s.len()
            && (s[i + 1] == b'\\' || (quote != 0 && s[i + 1] == quote))
        {
            i += 1;
            out.push(s[i]);
            i += 1;
        } else {
            // A multibyte character is copied whole.
            let n = char_bytes(enc, s[i]);
            let mut j = 0;
            while j < n && i < len && i < s.len() {
                out.push(s[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}

/// `php_ap_getword_conf`: a (possibly quoted) parameter value.
fn getword_conf(s: &[u8], enc: Option<InEnc>) -> Vec<u8> {
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
        return substring_conf(rest, rest.len(), q, enc);
    }
    let end = s.iter().position(|&c| c_isspace(c)).unwrap_or(s.len());
    substring_conf(s, end, 0, enc)
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
        if let Some(p) = crate::ext::standard::file::open_temp(dir, b"php") {
            return Some(p);
        }
        diags.push(Diag::Notice(
            "file created in the system's temporary directory".into(),
        ));
    }
    crate::ext::standard::file::open_temp(&crate::ext::standard::file::temp_dir(), b"php")
}

/// `rfc1867_post_handler`: parse a multipart body (`src` reads what is
/// left of the request body, [`BodyState::reader`]; `read_before`: the body
/// bytes read before it, php's `SG(read_post_bytes)`) into `post` /
/// `files`.
pub fn parse(
    src: &mut dyn FnMut(&mut [u8]) -> usize,
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
    mb.translate = lim.translate;
    let mut protected: HashSet<Vec<u8>> = HashSet::new();
    let mut uploaded: Vec<Vec<u8>> = Vec::new();
    let mut count: i64 = 0;
    let mut max_file_size: i64 = 0;
    let mut anonymous_index: u32 = 0;
    let mut skip_upload = false;

    while !mb.eof() {
        // php checks ZEND_HAS_OOM() after each step that allocates and stops
        // parsing there (fileupload_done).
        if mem::has_oom() {
            break;
        }
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
        let enc = mb.input_enc;
        let conv = |v: Vec<u8>| match enc {
            Some(e) => e.into_internal(&v),
            None => v,
        };
        while !cd.is_empty() {
            let (pair, rest) = getword(cd, b';', enc);
            cd = rest;
            while !cd.is_empty() && c_isspace(cd[0]) {
                cd = &cd[1..];
            }
            if pair.contains(&b'=') {
                let (key, val) = getword(&pair, b'=', enc);
                if key.eq_ignore_ascii_case(b"name") {
                    param = Some(conv(getword_conf(val, enc)));
                } else if key.eq_ignore_ascii_case(b"filename") {
                    filename = Some(conv(getword_conf(val, enc)));
                }
            }
            if mem::has_oom() {
                break;
            }
        }
        if mem::has_oom() {
            break;
        }

        // A normal form variable.
        if let (None, Some(p)) = (&filename, param.clone()) {
            let value = mb.read_body().unwrap_or_default();
            if mem::has_oom() {
                break;
            }
            let value = conv(value);
            count += 1;
            if count <= max_input_vars {
                // php 8.5 registers form fields under the name as sent (no
                // protected-variable normalisation or check).
                register_variable_value(post, &p, Value::str(value.clone()), false);
                if mem::has_oom() {
                    break;
                }
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
        if mem::has_oom() {
            break;
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
            Value::str(cstr(&filename)),
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

/// Read until `buf` is full or the reader ends (an error ends it too).
fn read_full(r: &mut dyn std::io::Read, buf: &mut [u8]) -> usize {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    n
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests;
