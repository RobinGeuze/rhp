//! Thin safe wrappers over C zlib (libz-sys) reproducing php's exact calls:
//! `php_zlib_encode` (deflateInit2 with MAX_MEM_LEVEL, one Z_FINISH) and
//! `php_zlib_decode` / `php_zlib_inflate_rounds` (growing-buffer inflate with
//! the max_length cutoff and the raw-deflate retry for ZLIB_ENCODING_ANY).
//! Going through zlib itself keeps compressed bytes identical to php's.

use std::os::raw::{c_int, c_uint, c_void};

use libz_sys as z;

pub const ENCODING_RAW: i64 = -15;
pub const ENCODING_DEFLATE: i64 = 15;
pub const ENCODING_GZIP: i64 = 31;
pub const ENCODING_ANY: i64 = 47;

pub const MAX_MEM_LEVEL: c_int = 9;
pub const DEF_MEM_LEVEL: c_int = 8;

unsafe extern "C" fn zalloc(_: *mut c_void, items: c_uint, size: c_uint) -> *mut c_void {
    libc::calloc(items as usize, size as usize)
}
unsafe extern "C" fn zfree(_: *mut c_void, p: *mut c_void) {
    libc::free(p)
}

pub(crate) fn new_stream() -> z::z_stream {
    z::z_stream {
        next_in: std::ptr::null_mut(),
        avail_in: 0,
        total_in: 0,
        next_out: std::ptr::null_mut(),
        avail_out: 0,
        total_out: 0,
        msg: std::ptr::null_mut(),
        state: std::ptr::null_mut(),
        zalloc,
        zfree,
        opaque: std::ptr::null_mut(),
        data_type: 0,
        adler: 0,
        reserved: 0,
    }
}

/// Deflate `data` in one go. `window_bits`: -15 raw, 15 zlib, 31 gzip.
/// `sync_flush` first emits a Z_SYNC_FLUSH (what php's zlib stream does on
/// close) before the Z_FINISH.
pub fn deflate(
    data: &[u8],
    level: i64,
    window_bits: i64,
    mem_level: c_int,
    sync_flush: bool,
) -> Result<Vec<u8>, c_int> {
    // SAFETY: the stream is initialised by deflateInit2_ before use, the
    // in/out pointers stay within the buffers we own for each call, and
    // deflateEnd frees zlib's state on every path.
    unsafe {
        let mut s = new_stream();
        let rc = z::deflateInit2_(
            &mut s,
            level as c_int,
            z::Z_DEFLATED,
            window_bits as c_int,
            mem_level,
            z::Z_DEFAULT_STRATEGY,
            z::zlibVersion(),
            std::mem::size_of::<z::z_stream>() as c_int,
        );
        if rc != z::Z_OK {
            return Err(rc);
        }
        let bound = z::deflateBound(&mut s, data.len() as z::uLong) as usize + 64;
        let mut out = vec![0u8; bound];
        s.next_in = data.as_ptr() as *mut u8;
        s.avail_in = data.len() as c_uint;
        s.next_out = out.as_mut_ptr();
        s.avail_out = out.len() as c_uint;
        if sync_flush {
            let rc = z::deflate(&mut s, z::Z_SYNC_FLUSH);
            if rc != z::Z_OK && rc != z::Z_BUF_ERROR {
                z::deflateEnd(&mut s);
                return Err(rc);
            }
        }
        let rc = z::deflate(&mut s, z::Z_FINISH);
        let n = s.total_out as usize;
        z::deflateEnd(&mut s);
        if rc != z::Z_STREAM_END {
            return Err(rc);
        }
        out.truncate(n);
        Ok(out)
    }
}

/// `php_zlib_inflate_rounds` for one stream. Returns (status, output, bytes of
/// input consumed).
unsafe fn inflate_rounds(s: &mut z::z_stream, max: usize) -> (c_int, Vec<u8>) {
    let mut buf: Vec<u8> = Vec::new();
    let mut used = 0usize;
    let mut size = if max != 0 && max < s.avail_in as usize {
        max
    } else {
        s.avail_in as usize
    };
    let mut status;
    let mut round = 0;
    loop {
        if max != 0 && max <= used {
            status = z::Z_MEM_ERROR;
        } else {
            buf.resize(size, 0);
            let free = size - used;
            s.avail_out = free as c_uint;
            s.next_out = buf.as_mut_ptr().add(used);
            status = z::inflate(s, z::Z_NO_FLUSH);
            used += free - s.avail_out as usize;
            size += (size >> 3) + 1;
        }
        round += 1;
        if !((status == z::Z_BUF_ERROR || (status == z::Z_OK && s.avail_in != 0)) && round < 100) {
            break;
        }
    }
    if status == z::Z_STREAM_END {
        buf.truncate(used);
        (status, buf)
    } else {
        (
            if status == z::Z_OK {
                z::Z_DATA_ERROR
            } else {
                status
            },
            Vec::new(),
        )
    }
}

/// `php_zlib_decode`: inflate `data` with `encoding` (window bits; 47 = any,
/// retried as raw deflate on a data error). `Err(status)` on failure.
pub fn decode(data: &[u8], encoding: i64, max_len: usize) -> Result<Vec<u8>, c_int> {
    if data.is_empty() {
        return Err(z::Z_DATA_ERROR);
    }
    // php hands zlib the trailing NUL of its zend_string too.
    let mut input = data.to_vec();
    input.push(0);
    let mut encoding = encoding;
    // SAFETY: as in `deflate`; inflateEnd is called on every initialised stream.
    unsafe {
        loop {
            let mut s = new_stream();
            let rc = z::inflateInit2_(
                &mut s,
                encoding as c_int,
                z::zlibVersion(),
                std::mem::size_of::<z::z_stream>() as c_int,
            );
            if rc != z::Z_OK {
                return Err(rc);
            }
            s.next_in = input.as_mut_ptr();
            s.avail_in = input.len() as c_uint;
            let (status, out) = inflate_rounds(&mut s, max_len);
            z::inflateEnd(&mut s);
            match status {
                z::Z_STREAM_END => return Ok(out),
                z::Z_DATA_ERROR if encoding == ENCODING_ANY => {
                    encoding = ENCODING_RAW;
                    continue;
                }
                other => return Err(other),
            }
        }
    }
}

/// zlib `gzread` semantics for a whole file: concatenated gzip members are all
/// decoded; data that is not gzip (no magic) is passed through unchanged;
/// trailing garbage after a member ends decoding. `None` on corrupt data.
pub fn gunzip_transparent(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 2 || data[0] != 0x1f || data[1] != 0x8b {
        return Some(data.to_vec());
    }
    let mut out = Vec::new();
    let mut rest = data;
    // SAFETY: as in `decode`.
    unsafe {
        while rest.len() >= 2 && rest[0] == 0x1f && rest[1] == 0x8b {
            let mut s = new_stream();
            if z::inflateInit2_(
                &mut s,
                31,
                z::zlibVersion(),
                std::mem::size_of::<z::z_stream>() as c_int,
            ) != z::Z_OK
            {
                return None;
            }
            s.next_in = rest.as_ptr() as *mut u8;
            s.avail_in = rest.len() as c_uint;
            let mut chunk = vec![0u8; 64 * 1024];
            let status = loop {
                s.next_out = chunk.as_mut_ptr();
                s.avail_out = chunk.len() as c_uint;
                let rc = z::inflate(&mut s, z::Z_NO_FLUSH);
                let got = chunk.len() - s.avail_out as usize;
                out.extend_from_slice(&chunk[..got]);
                if rc != z::Z_OK {
                    break rc;
                }
                if s.avail_in == 0 && got == 0 {
                    break z::Z_BUF_ERROR;
                }
            };
            let consumed = rest.len() - s.avail_in as usize;
            z::inflateEnd(&mut s);
            if status != z::Z_STREAM_END {
                // truncated stream: zlib's gzread returns what it decoded.
                return if status == z::Z_BUF_ERROR {
                    Some(out)
                } else {
                    None
                };
            }
            rest = &rest[consumed..];
        }
    }
    Some(out)
}

/// CRC-32 (reflected 0xEDB88320): php's `crc32()`, `hash('crc32b')` and the
/// zip entry checksum.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: z::uLong = 0;
    for chunk in data.chunks(c_uint::MAX as usize) {
        // SAFETY: `chunk` is a live slice of the length passed.
        crc = unsafe { z::crc32(crc, chunk.as_ptr(), chunk.len() as c_uint) };
    }
    crc as u32
}

/// Adler-32 (RFC 1950): `hash('adler32')`.
pub fn adler32(data: &[u8]) -> u32 {
    let mut sum: z::uLong = 1;
    for chunk in data.chunks(c_uint::MAX as usize) {
        // SAFETY: `chunk` is a live slice of the length passed.
        sum = unsafe { z::adler32(sum, chunk.as_ptr(), chunk.len() as c_uint) };
    }
    sum as u32
}

#[cfg(test)]
mod tests;

/// An incremental deflate or inflate stream (php's `php_zlib_context.Z`).
/// The z_stream is boxed: zlib's state points back at it.
pub struct ZStream {
    s: Box<z::z_stream>,
    deflate: bool,
}

impl ZStream {
    /// `deflateInit2`.
    pub fn deflate_init(
        level: i64,
        window_bits: i64,
        mem_level: i64,
        strategy: i64,
    ) -> Result<ZStream, c_int> {
        let mut s = Box::new(new_stream());
        // SAFETY: a fresh, zeroed stream with our allocators; it stays boxed
        // (pinned) for its whole life and deflateEnd runs on drop.
        let rc = unsafe {
            z::deflateInit2_(
                &mut *s,
                level as c_int,
                z::Z_DEFLATED,
                window_bits as c_int,
                mem_level as c_int,
                strategy as c_int,
                z::zlibVersion(),
                std::mem::size_of::<z::z_stream>() as c_int,
            )
        };
        if rc != z::Z_OK {
            return Err(rc);
        }
        Ok(ZStream { s, deflate: true })
    }

    /// `inflateInit2`.
    pub fn inflate_init(window_bits: i64) -> Result<ZStream, c_int> {
        let mut s = Box::new(new_stream());
        // SAFETY: as in deflate_init.
        let rc = unsafe {
            z::inflateInit2_(
                &mut *s,
                window_bits as c_int,
                z::zlibVersion(),
                std::mem::size_of::<z::z_stream>() as c_int,
            )
        };
        if rc != z::Z_OK {
            return Err(rc);
        }
        Ok(ZStream { s, deflate: false })
    }

    /// `deflateSetDictionary` / `inflateSetDictionary`.
    pub fn set_dictionary(&mut self, dict: &[u8]) -> c_int {
        // SAFETY: dict outlives the call; zlib copies it.
        unsafe {
            if self.deflate {
                z::deflateSetDictionary(&mut *self.s, dict.as_ptr(), dict.len() as c_uint)
            } else {
                z::inflateSetDictionary(&mut *self.s, dict.as_ptr(), dict.len() as c_uint)
            }
        }
    }

    /// `deflateReset` / `inflateReset`.
    pub fn reset(&mut self) {
        // SAFETY: an initialised stream.
        unsafe {
            if self.deflate {
                z::deflateReset(&mut *self.s);
            } else {
                z::inflateReset(&mut *self.s);
            }
        }
    }

    /// `total_in`.
    #[allow(clippy::unnecessary_cast)] // uLong's width differs by platform
    pub fn total_in(&self) -> u64 {
        self.s.total_in as u64
    }

    /// Point the stream at `input` and `out[used..]`.
    pub fn set_io(&mut self, input: &[u8], out: &mut [u8]) {
        self.s.next_in = input.as_ptr() as *mut u8;
        self.s.avail_in = input.len() as c_uint;
        self.s.next_out = out.as_mut_ptr();
        self.s.avail_out = out.len() as c_uint;
    }

    /// Move the output window to `out[used..]` (after the buffer grew).
    pub fn set_out(&mut self, out: &mut [u8], used: usize) {
        self.s.next_out = out[used..].as_mut_ptr();
        self.s.avail_out = (out.len() - used) as c_uint;
    }

    /// Re-point the remaining input after the caller's buffer moved: the
    /// last `avail_in` bytes of `input`.
    pub fn set_in_tail(&mut self, input: &[u8]) {
        let n = (self.s.avail_in as usize).min(input.len());
        self.s.next_in = input[input.len() - n..].as_ptr() as *mut u8;
    }

    pub fn avail_in(&self) -> usize {
        self.s.avail_in as usize
    }

    pub fn avail_out(&self) -> usize {
        self.s.avail_out as usize
    }

    /// One `deflate`/`inflate` call. The caller keeps the buffers given to
    /// `set_io`/`set_out` alive and unmoved until it returns.
    pub fn run(&mut self, flush: i64) -> c_int {
        // SAFETY: next_in/next_out point into live buffers of at least
        // avail_in/avail_out bytes (set_io/set_out).
        unsafe {
            if self.deflate {
                z::deflate(&mut *self.s, flush as c_int)
            } else {
                z::inflate(&mut *self.s, flush as c_int)
            }
        }
    }
}

impl Drop for ZStream {
    fn drop(&mut self) {
        // SAFETY: an initialised stream, ended once.
        unsafe {
            if self.deflate {
                z::deflateEnd(&mut *self.s);
            } else {
                z::inflateEnd(&mut *self.s);
            }
        }
    }
}
