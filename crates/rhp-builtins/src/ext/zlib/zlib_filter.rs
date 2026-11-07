//! ext/zlib/zlib_filter.c: the zlib.inflate and zlib.deflate stream filters,
//! on C zlib with php's parameters.

use std::os::raw::{c_int, c_uint};

use libz_sys as z;
use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, Value};

use crate::main::streams::filter::{
    filter_warning, Brigade, FilterOps, Status, PSFS_FLAG_FLUSH_CLOSE, PSFS_FLAG_FLUSH_INC,
};
use crate::Host;

const BUF_LEN: usize = 0x8000;

/// php_zlib_filter_data. The z_stream is boxed: zlib keeps a pointer back
/// to it.
struct ZFilter {
    strm: Box<z::z_stream>,
    outbuf: Vec<u8>,
    finished: bool,
    inflate: bool,
}

impl ZFilter {
    /// Move what zlib wrote to a new bucket on `outb`; true if there was any.
    fn drain(&mut self, outb: &Brigade) -> bool {
        let n = BUF_LEN - self.strm.avail_out as usize;
        if n == 0 {
            return false;
        }
        outb.append_data(self.outbuf[..n].to_vec());
        self.strm.avail_out = BUF_LEN as c_uint;
        self.strm.next_out = self.outbuf.as_mut_ptr();
        true
    }
}

impl FilterOps for ZFilter {
    fn filter(
        &mut self,
        host: &mut dyn Host,
        _stream: &Value,
        inb: &Brigade,
        outb: &Brigade,
        consumed: Option<&mut usize>,
        flags: i32,
    ) -> Status {
        let mut exit = Status::FeedMe;
        let mut n = 0;
        // SAFETY: the stream was initialised by inflateInit2_/deflateInit2_
        // and stays boxed; next_in points into `buf` only for the duration
        // of each call and next_out into our own `outbuf`.
        unsafe {
            while let Some(b) = inb.pop() {
                let buf = std::mem::take(&mut b.borrow_mut().buf);
                let mut bin = 0usize;
                while bin < buf.len() && !(self.inflate && self.finished) {
                    let desired = (buf.len() - bin).min(BUF_LEN);
                    self.strm.next_in = buf.as_ptr().add(bin) as *mut u8;
                    self.strm.avail_in = desired as c_uint;
                    let status = if self.inflate {
                        let mode = if flags & PSFS_FLAG_FLUSH_CLOSE != 0 {
                            z::Z_FINISH
                        } else {
                            z::Z_SYNC_FLUSH
                        };
                        let status = z::inflate(&mut *self.strm, mode);
                        if status == z::Z_STREAM_END {
                            z::inflateEnd(&mut *self.strm);
                            self.finished = true;
                            exit = Status::PassOn;
                        } else if status != z::Z_OK && status != z::Z_BUF_ERROR {
                            let msg = format!("zlib: {}", super::z_error(status));
                            filter_warning(host, crate::E_NOTICE, &msg);
                            self.strm.next_in = std::ptr::null_mut();
                            self.strm.avail_in = 0;
                            return Status::ErrFatal;
                        }
                        status
                    } else {
                        let mode = if flags & PSFS_FLAG_FLUSH_CLOSE != 0 {
                            z::Z_FULL_FLUSH
                        } else if flags & PSFS_FLAG_FLUSH_INC != 0 {
                            z::Z_SYNC_FLUSH
                        } else {
                            z::Z_NO_FLUSH
                        };
                        self.finished = mode != z::Z_NO_FLUSH;
                        let status = z::deflate(&mut *self.strm, mode);
                        if status != z::Z_OK {
                            return Status::ErrFatal;
                        }
                        status
                    };
                    let _ = status;
                    let used = desired - self.strm.avail_in as usize;
                    self.strm.next_in = std::ptr::null_mut();
                    self.strm.avail_in = 0;
                    bin += used;
                    if self.drain(outb) {
                        exit = Status::PassOn;
                    }
                }
                n += buf.len();
            }
            if self.inflate {
                if !self.finished && flags & PSFS_FLAG_FLUSH_CLOSE != 0 {
                    let mut status = z::Z_OK;
                    while status == z::Z_OK {
                        status = z::inflate(&mut *self.strm, z::Z_FINISH);
                        if self.drain(outb) {
                            exit = Status::PassOn;
                        }
                    }
                }
            } else if flags & PSFS_FLAG_FLUSH_CLOSE != 0
                || (flags & PSFS_FLAG_FLUSH_INC != 0 && !self.finished)
            {
                let mode = if flags & PSFS_FLAG_FLUSH_CLOSE != 0 {
                    z::Z_FINISH
                } else {
                    z::Z_SYNC_FLUSH
                };
                let mut status = z::Z_OK;
                while status == z::Z_OK {
                    status = z::deflate(&mut *self.strm, mode);
                    self.finished = true;
                    if self.drain(outb) {
                        exit = Status::PassOn;
                    }
                }
            }
        }
        if let Some(c) = consumed {
            *c = n;
        }
        exit
    }
}

impl Drop for ZFilter {
    fn drop(&mut self) {
        // SAFETY: the stream was initialised; End is skipped after an
        // inflate stream already ended.
        unsafe {
            if self.inflate {
                if !self.finished {
                    z::inflateEnd(&mut *self.strm);
                }
            } else {
                z::deflateEnd(&mut *self.strm);
            }
        }
    }
}

fn param(params: &Value, key: &str) -> Option<Value> {
    match params {
        Value::Array(a) => a.get(&ArrayKey::from_bytes(key.as_bytes())).map(deref),
        Value::Object(o) => o
            .borrow()
            .props()
            .get(&rhp_values::PhpStr::from_bytes(key.as_bytes()))
            .map(deref),
        _ => None,
    }
}

/// php_zlib_filter_create.
pub(crate) fn create(
    host: &mut dyn Host,
    name: &[u8],
    params: Option<&Value>,
) -> Option<Box<dyn FilterOps>> {
    let params = params.map(deref);
    let warn = |host: &mut dyn Host, msg: String| filter_warning(host, crate::E_WARNING, &msg);
    let mut f = ZFilter {
        strm: Box::new(super::zraw::new_stream()),
        outbuf: vec![0u8; BUF_LEN],
        finished: false,
        inflate: true,
    };
    f.strm.avail_out = BUF_LEN as c_uint;
    f.strm.next_out = f.outbuf.as_mut_ptr();
    f.strm.data_type = z::Z_ASCII;
    // SAFETY: returns a static string.
    let version = unsafe { z::zlibVersion() };
    let size = std::mem::size_of::<z::z_stream>() as c_int;
    let status;
    if name.eq_ignore_ascii_case(b"zlib.inflate") {
        let mut window: i64 = -15;
        if let Some(t) = params.as_ref().and_then(|p| param(p, "window")) {
            let t = conv::to_int(&t);
            if !(-15..=15 + 32).contains(&t) {
                warn(
                    host,
                    format!("Invalid parameter given for window size ({t})"),
                );
            } else {
                window = t;
            }
        }
        // SAFETY: a fresh boxed stream.
        status = unsafe { z::inflateInit2_(&mut *f.strm, window as c_int, version, size) };
    } else if name.eq_ignore_ascii_case(b"zlib.deflate") {
        let mut level: i64 = -1;
        let mut window: i64 = -15;
        let mut mem: i64 = 9;
        let mut set_level = |host: &mut dyn Host, t: i64| {
            if !(-1..=9).contains(&t) {
                warn(host, format!("Invalid compression level specified. ({t})"));
            } else {
                level = t;
            }
        };
        match &params {
            None => {}
            Some(p @ (Value::Array(_) | Value::Object(_))) => {
                if let Some(t) = param(p, "memory") {
                    let t = conv::to_int(&t);
                    if !(1..=9).contains(&t) {
                        warn(
                            host,
                            format!("Invalid parameter given for memory level ({t})"),
                        );
                    } else {
                        mem = t;
                    }
                }
                if let Some(t) = param(p, "window") {
                    let t = conv::to_int(&t);
                    if !(-15..=15 + 16).contains(&t) {
                        warn(
                            host,
                            format!("Invalid parameter given for window size ({t})"),
                        );
                    } else {
                        window = t;
                    }
                }
                if let Some(t) = param(p, "level") {
                    set_level(host, conv::to_int(&t));
                }
            }
            Some(v @ (Value::Str(_) | Value::Float(_) | Value::Int(_))) => {
                set_level(host, conv::to_int(v))
            }
            Some(_) => warn(host, "Invalid filter parameter, ignored".to_string()),
        }
        f.inflate = false;
        // SAFETY: a fresh boxed stream.
        status = unsafe {
            z::deflateInit2_(
                &mut *f.strm,
                level as c_int,
                z::Z_DEFLATED,
                window as c_int,
                mem as c_int,
                0,
                version,
                size,
            )
        };
        f.finished = true;
    } else {
        // zlib's End calls check for a never-initialised stream.
        return None;
    }
    if status != z::Z_OK {
        return None;
    }
    Some(Box::new(f))
}
