//! zip_stream.c: entry streams (`ZipArchive::getStream*`) and the `zip://`
//! wrapper (`zip://archive.zip#entry`), on [`OpsStream`].

use std::any::Any;
use std::ffi::{c_int, CString};
use std::ptr;
use std::rc::Rc;

use rhp_values::{ArrayKey, Value};

use super::sys::{self, zip_file_t, zip_t};
use super::{c_string, expand_filepath, Archive, MAXPATHLEN};
use crate::opsstream::{OpsStream, StreamOps};
use crate::streams::{StatInfo, StreamWrapper, WrapperReg};
use crate::Host;

/// `php_zip_stream_data_t`
struct ZipStream {
    /// The archive the stream opened itself (`zip://`), closed with it.
    za: *mut zip_t,
    zf: *mut zip_file_t,
    /// The `ZipArchive`'s archive the stream borrows (`getStream`).
    archive: Option<Rc<Archive>>,
    seekable: bool,
    /// `stream->orig_path`, read by stat.
    orig_path: Vec<u8>,
}

impl StreamOps for ZipStream {
    fn label(&self) -> &'static str {
        "zip"
    }

    /// php_zip_ops_read
    fn read(&mut self, buf: &mut [u8], eof: &mut bool, diags: &mut Vec<(i64, String)>) -> isize {
        if self.zf.is_null() {
            return 0;
        }
        let n = unsafe { sys::zip_fread(self.zf, buf.as_mut_ptr().cast(), buf.len() as u64) };
        if n < 0 {
            *eof = true;
            let (_, msg) = super::file_error(self.zf);
            diags.push((crate::E_WARNING, format!("Zip stream error: {msg}")));
            return -1;
        }
        if n == 0 || (n as usize) < buf.len() {
            *eof = true;
        }
        n as isize
    }

    /// php_zip_ops_write: accepted and dropped.
    fn write(&mut self, data: &[u8]) -> isize {
        data.len() as isize
    }

    fn can_seek(&self) -> bool {
        self.seekable
    }

    /// php_zip_ops_seek
    fn seek(&mut self, offset: i64, whence: i32) -> (i32, i64) {
        if self.zf.is_null() {
            return (-1, -1);
        }
        unsafe {
            let ret = sys::zip_fseek(self.zf, offset, whence as c_int) as i32;
            (ret, sys::zip_ftell(self.zf))
        }
    }

    /// php_zip_ops_stat: reopens the archive the path names.
    fn stat(&mut self) -> Option<StatInfo> {
        let path = self.orig_path.clone();
        let hash = path.iter().position(|&b| b == b'#')?;
        let path = match path.get(..6) {
            Some(p) if p.eq_ignore_ascii_case(b"zip://") => &path[6..],
            _ => &path[..],
        };
        let hash = hash - (self.orig_path.len() - path.len());
        let fragment_len = path.len() - hash;
        if path.len() >= MAXPATHLEN {
            return None;
        }
        let dirname = &path[..path.len() - fragment_len];
        let fragment = &path[hash + 1..];
        let dirname = expand_filepath(dirname).unwrap_or_else(|| dirname.to_vec());
        let d = CString::new(dirname).ok()?;
        let mut err: c_int = 0;
        let za = unsafe { sys::zip_open(d.as_ptr(), sys::ZIP_CREATE, &mut err) };
        if za.is_null() {
            return None;
        }
        let f = c_string(fragment);
        let mut sb = sys::zip_stat_t::zeroed();
        let found = unsafe { sys::zip_stat(za, f.as_ptr(), sys::ZIP_FL_NOCASE, &mut sb) } == 0;
        unsafe { sys::zip_close(za) };
        if !found {
            return None;
        }
        let dir = path.last() == Some(&b'/');
        Some(StatInfo {
            size: if dir { 0 } else { sb.size },
            mode: if dir { 0o040000 } else { 0o100000 },
            is_dir: dir,
            is_regular: !dir,
            mtime: sb.mtime,
            atime: sb.mtime,
            ctime: sb.mtime,
            nlink: 1,
            rdev: -1,
            blksize: -1,
            blocks: -1,
            ino: -1,
            ..Default::default()
        })
    }

    /// php_zip_ops_close: the archive reference goes with the stream.
    fn close(&mut self, host: &mut dyn Host) {
        unsafe {
            if !self.zf.is_null() {
                sys::zip_fclose(self.zf);
                self.zf = ptr::null_mut();
            }
            if !self.za.is_null() {
                sys::zip_close(self.za);
                self.za = ptr::null_mut();
            }
        }
        if let Some(a) = self.archive.take() {
            let fname = "fclose";
            let _ = super::release(host, fname, a);
        }
    }
}

impl Drop for ZipStream {
    fn drop(&mut self) {
        unsafe {
            if !self.zf.is_null() {
                sys::zip_fclose(self.zf);
            }
            if !self.za.is_null() {
                sys::zip_close(self.za);
            }
        }
    }
}

/// php_stream_zip_open: a stream over entry `sb` of the object's archive,
/// which it keeps open.
pub(super) fn open_entry(
    archive: Rc<Archive>,
    sb: &sys::zip_stat_t,
    mode: &[u8],
    flags: u32,
) -> Option<OpsStream> {
    if mode.first() != Some(&b'r') {
        return None;
    }
    let za = archive.za();
    if za.is_null() {
        return None;
    }
    let zf = unsafe { sys::zip_fopen_index(za, sb.index, flags) };
    if zf.is_null() {
        return None;
    }
    let seekable = unsafe { sys::rhp_zip_file_is_seekable(zf) } > 0;
    let name = unsafe { sys::cstr(sb.name) }.to_vec();
    let ops = ZipStream {
        za: ptr::null_mut(),
        zf,
        archive: Some(archive),
        seekable,
        orig_path: name.clone(),
    };
    Some(OpsStream::new(Box::new(ops), mode, name))
}

/// The `zip://` wrapper (`php_stream_zip_wrapper`): an opener only.
struct ZipWrapper;

impl ZipWrapper {
    /// php_stream_zip_opener
    fn opener(
        &self,
        url: &[u8],
        mode: &[u8],
        options: &Value,
        diags: &mut Vec<(i64, String)>,
    ) -> Option<OpsStream> {
        let hash = url.iter().position(|&b| b == b'#')?;
        let path = match url.get(..6) {
            Some(p) if p.eq_ignore_ascii_case(b"zip://") => &url[6..],
            _ => url,
        };
        let hash = hash - (url.len() - path.len());
        if path.len() >= MAXPATHLEN || mode.first() != Some(&b'r') {
            return None;
        }
        let dirname = &path[..hash];
        let fragment = &path[hash + 1..];
        // libzip opens the file itself: relative to the process directory,
        // which is rhp's.
        let dirname = expand_filepath(dirname).unwrap_or_else(|| dirname.to_vec());
        let d = CString::new(dirname).ok()?;
        let mut err: c_int = 0;
        let za = unsafe { sys::zip_open(d.as_ptr(), sys::ZIP_CREATE, &mut err) };
        if za.is_null() {
            return None;
        }
        let password = match options {
            Value::Array(o) => match o.get(&ArrayKey::from_bytes(b"zip")) {
                Some(Value::Array(z)) => z.get(&ArrayKey::from_bytes(b"password")).cloned(),
                _ => None,
            },
            _ => None,
        };
        if let Some(pw) = password {
            let ok = match super::deref(&pw) {
                Value::Str(s) => {
                    let p = c_string(s.as_bytes());
                    (unsafe { sys::zip_set_default_password(za, p.as_ptr()) }) == 0
                }
                _ => false,
            };
            if !ok {
                diags.push((crate::E_WARNING, "Can't set zip password".into()));
            }
        }
        let f = c_string(fragment);
        let zf = unsafe { sys::zip_fopen(za, f.as_ptr(), 0) };
        if zf.is_null() {
            unsafe { sys::zip_close(za) };
            return None;
        }
        let seekable = unsafe { sys::rhp_zip_file_is_seekable(zf) } > 0;
        let ops = ZipStream {
            za,
            zf,
            archive: None,
            seekable,
            orig_path: url.to_vec(),
        };
        let mut s = OpsStream::new(Box::new(ops), mode, url.to_vec());
        s.wrapper_label = Some("zip wrapper");
        Some(s)
    }
}

impl StreamWrapper for ZipWrapper {
    /// No url_stat: `file_exists("zip://…")` is false, as in php.
    fn exists(&self, _url: &[u8]) -> bool {
        false
    }

    /// A whole-entry read for the callers that take the contents at once.
    fn get_contents(&self, url: &[u8]) -> Option<Vec<u8>> {
        let mut diags = Vec::new();
        let mut s = self.opener(url, b"rb", &Value::Null, &mut diags)?;
        Some(s.read_all())
    }

    fn stat(&self, _url: &[u8]) -> Option<StatInfo> {
        None
    }

    fn has_stream_opener(&self) -> bool {
        true
    }

    fn open_stream(
        &self,
        url: &[u8],
        mode: &[u8],
        options: &Value,
        diags: &mut Vec<(i64, String)>,
    ) -> Option<Option<OpsStream>> {
        Some(self.opener(url, mode, options, diags))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The wrappers ext/zip registers.
pub fn wrappers() -> Vec<WrapperReg> {
    vec![WrapperReg {
        scheme: "zip",
        factory: || Rc::new(ZipWrapper),
    }]
}
