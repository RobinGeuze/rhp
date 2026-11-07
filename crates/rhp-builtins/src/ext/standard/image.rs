//! ext/standard/image.c: `getimagesize()`, `getimagesizefromstring()`,
//! `image_type_to_mime_type()`, `image_type_to_extension()` and the
//! `IMAGETYPE_*` constants (php_image.h). The per-format readers are php's,
//! run over the file's bytes the way they run over a php_stream; AVIF/HEIF
//! go through libavifinfo (rhp-csrc), as in php.

use rhp_values::ops::deref;
use rhp_values::{conv, PhpArray, Value};

use crate::{Host, Thrown};

const GIF: i64 = 1;
const JPEG: i64 = 2;
const PNG: i64 = 3;
const SWF: i64 = 4;
const PSD: i64 = 5;
const BMP: i64 = 6;
const TIFF_II: i64 = 7;
const TIFF_MM: i64 = 8;
const JPC: i64 = 9;
const JP2: i64 = 10;
const JPX: i64 = 11;
const JB2: i64 = 12;
const SWC: i64 = 13;
const IFF: i64 = 14;
const WBMP: i64 = 15;
const XBM: i64 = 16;
const ICO: i64 = 17;
const WEBP: i64 = 18;
const AVIF: i64 = 19;
const HEIF: i64 = 20;
/// The type ext/libxml's SVG handler registers (the first free id).
const SVG: i64 = 21;

pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    let i = Value::Int;
    vec![
        ("IMAGETYPE_GIF", i(GIF)),
        ("IMAGETYPE_JPEG", i(JPEG)),
        ("IMAGETYPE_PNG", i(PNG)),
        ("IMAGETYPE_SWF", i(SWF)),
        ("IMAGETYPE_PSD", i(PSD)),
        ("IMAGETYPE_BMP", i(BMP)),
        ("IMAGETYPE_TIFF_II", i(TIFF_II)),
        ("IMAGETYPE_TIFF_MM", i(TIFF_MM)),
        ("IMAGETYPE_JPC", i(JPC)),
        ("IMAGETYPE_JP2", i(JP2)),
        ("IMAGETYPE_JPX", i(JPX)),
        ("IMAGETYPE_JB2", i(JB2)),
        ("IMAGETYPE_SWC", i(SWC)),
        ("IMAGETYPE_IFF", i(IFF)),
        ("IMAGETYPE_WBMP", i(WBMP)),
        ("IMAGETYPE_JPEG2000", i(JPC)),
        ("IMAGETYPE_XBM", i(XBM)),
        ("IMAGETYPE_ICO", i(ICO)),
        ("IMAGETYPE_WEBP", i(WEBP)),
        ("IMAGETYPE_AVIF", i(AVIF)),
        ("IMAGETYPE_HEIF", i(HEIF)),
        ("IMAGETYPE_SVG", i(SVG)),
        ("IMAGETYPE_UNKNOWN", i(0)),
        ("IMAGETYPE_COUNT", i(22)),
    ]
}

/// A php_stream over the image's bytes: reads stop at the end, seeks may go
/// past it (as on a file or memory stream), `eof` is set by a short read.
struct Stream<'a> {
    d: &'a [u8],
    pos: usize,
    eof: bool,
}

impl<'a> Stream<'a> {
    fn read(&mut self, n: usize) -> &'a [u8] {
        let start = self.pos.min(self.d.len());
        let end = start.saturating_add(n).min(self.d.len());
        if end - start < n {
            self.eof = true;
        }
        self.pos = start.max(self.pos) + (end - start);
        &self.d[start..end]
    }

    /// `php_stream_read(...) == n`, into `buf`.
    fn read_into(&mut self, buf: &mut [u8]) -> bool {
        let got = self.read(buf.len());
        buf[..got.len()].copy_from_slice(got);
        got.len() == buf.len()
    }

    fn getc(&mut self) -> i32 {
        match self.read(1) {
            [c] => *c as i32,
            _ => -1,
        }
    }

    /// `php_stream_seek(s, off, SEEK_CUR) == 0`.
    fn seek_cur(&mut self, off: i64) -> bool {
        let new = self.pos as i64 + off;
        if new < 0 {
            self.pos = 0;
            return false;
        }
        self.pos = new as usize;
        self.eof = false;
        true
    }

    fn seek_set(&mut self, off: i64) -> bool {
        self.pos = 0;
        self.seek_cur(off)
    }

    fn rewind(&mut self) {
        self.seek_set(0);
    }

    /// `php_stream_gets(s, NULL, 0)`: the next line with its newline.
    fn gets(&mut self) -> Option<&'a [u8]> {
        let rest = self.d.get(self.pos..).filter(|r| !r.is_empty())?;
        let n = rest
            .iter()
            .position(|&c| c == b'\n')
            .map_or(rest.len(), |i| i + 1);
        self.pos += n;
        Some(&rest[..n])
    }

    fn read2(&mut self) -> u32 {
        match self.read(2) {
            [a, b] => (*a as u32) << 8 | *b as u32,
            _ => 0,
        }
    }

    fn read4(&mut self) -> u32 {
        match self.read(4) {
            [a, b, c, d] => u32::from_be_bytes([*a, *b, *c, *d]),
            _ => 0,
        }
    }
}

impl rhp_csrc::avifinfo::AvifStream for Stream<'_> {
    fn read_exact(&mut self, buf: &mut [u8]) -> bool {
        self.read_into(buf)
    }

    fn skip(&mut self, n: usize) -> bool {
        self.seek_cur(n as i64)
    }
}

/// `struct php_gfxinfo`.
#[derive(Default)]
struct GfxInfo {
    width: u32,
    height: u32,
    bits: u32,
    channels: u32,
}

fn gif(s: &mut Stream) -> Option<GfxInfo> {
    let mut dim = [0u8; 5];
    if !s.seek_cur(3) || !s.read_into(&mut dim) {
        return None;
    }
    Some(GfxInfo {
        width: dim[0] as u32 | (dim[1] as u32) << 8,
        height: dim[2] as u32 | (dim[3] as u32) << 8,
        bits: if dim[4] & 0x80 != 0 {
            (dim[4] as u32 & 0x07) + 1
        } else {
            0
        },
        channels: 3,
    })
}

fn psd(s: &mut Stream) -> Option<GfxInfo> {
    let mut dim = [0u8; 8];
    if !s.seek_cur(11) || !s.read_into(&mut dim) {
        return None;
    }
    Some(GfxInfo {
        height: u32::from_be_bytes([dim[0], dim[1], dim[2], dim[3]]),
        width: u32::from_be_bytes([dim[4], dim[5], dim[6], dim[7]]),
        ..Default::default()
    })
}

fn bmp(s: &mut Stream) -> Option<GfxInfo> {
    let mut d = [0u8; 16];
    if !s.seek_cur(11) || !s.read_into(&mut d) {
        return None;
    }
    let size = i32::from_le_bytes([d[0], d[1], d[2], d[3]]);
    if size == 12 {
        Some(GfxInfo {
            width: u16::from_le_bytes([d[4], d[5]]) as u32,
            height: u16::from_le_bytes([d[6], d[7]]) as u32,
            bits: d[11] as u32,
            channels: 0,
        })
    } else if size > 12 && (size <= 64 || size == 108 || size == 124) {
        Some(GfxInfo {
            width: u32::from_le_bytes([d[4], d[5], d[6], d[7]]),
            height: i32::from_le_bytes([d[8], d[9], d[10], d[11]]).unsigned_abs(),
            bits: u16::from_le_bytes([d[14], d[15]]) as u32,
            channels: 0,
        })
    } else {
        None
    }
}

/// php_swf_get_bits.
fn swf_bits(buf: &[u8], pos: u32, count: u32) -> u64 {
    let mut r: u64 = 0;
    for l in pos..pos + count {
        let byte = buf.get((l / 8) as usize).copied().unwrap_or(0) as u64;
        r = r.wrapping_add(((byte >> (7 - (l % 8))) & 1) << (count - (l - pos) - 1));
    }
    r
}

/// The SWF header's frame rectangle, in pixels.
fn swf_rect(b: &[u8]) -> (u32, u32) {
    let bits = swf_bits(b, 0, 5) as u32;
    // Unsigned long arithmetic, as in C: a negative extent wraps.
    let w = swf_bits(b, 5 + bits, bits).wrapping_sub(swf_bits(b, 5, bits)) / 20;
    let h = swf_bits(b, 5 + 3 * bits, bits).wrapping_sub(swf_bits(b, 5 + 2 * bits, bits)) / 20;
    (w as u32, h as u32)
}

fn swf(s: &mut Stream) -> Option<GfxInfo> {
    let mut a = [0u8; 32];
    if !s.seek_cur(5) || !s.read_into(&mut a) {
        return None;
    }
    let (width, height) = swf_rect(&a);
    Some(GfxInfo {
        width,
        height,
        ..Default::default()
    })
}

/// zlib's `uncompress()`: the status and the bytes written.
fn uncompress(out: &mut [u8], input: &[u8]) -> (i32, usize) {
    let mut len = out.len() as libz_sys::uLongf;
    // SAFETY: both buffers are valid for the lengths passed.
    let st = unsafe {
        libz_sys::uncompress(
            out.as_mut_ptr(),
            &mut len,
            input.as_ptr(),
            input.len() as libz_sys::uLong,
        )
    };
    (st, len as usize)
}

fn swc(s: &mut Stream) -> Option<GfxInfo> {
    let mut a = [0u8; 64];
    if !s.seek_cur(5) || !s.read_into(&mut a) {
        return None;
    }
    let mut b = [0u8; 65];
    let mut status = uncompress(&mut b[..64], &a).0;
    if status != libz_sys::Z_OK {
        // Not all in the first 64 bytes: uncompress the whole rest, in a
        // buffer twice as big each time it is too small.
        if !s.seek_set(8) {
            return None;
        }
        let rest = s.d.get(8..).unwrap_or(&[]);
        let mut factor = 1usize;
        let mut cap = 0usize;
        loop {
            factor <<= 1;
            let len = rest.len().checked_mul(factor)?;
            // php's fallible safe_ferealloc of its buffer.
            if !rhp_values::mem::reserve_realloc(cap, len) {
                return None;
            }
            cap = len;
            let mut buf = vec![0u8; len];
            status = uncompress(&mut buf, rest).0;
            if status == libz_sys::Z_OK {
                let n = buf.len().min(64);
                b[..n].copy_from_slice(&buf[..n]);
            }
            if status != libz_sys::Z_BUF_ERROR || factor >= 1 << 15 {
                break;
            }
        }
    }
    if status != libz_sys::Z_OK {
        return None;
    }
    let (width, height) = swf_rect(&b);
    Some(GfxInfo {
        width,
        height,
        ..Default::default()
    })
}

fn png(s: &mut Stream) -> Option<GfxInfo> {
    let mut d = [0u8; 9];
    if !s.seek_cur(8) || !s.read_into(&mut d) {
        return None;
    }
    Some(GfxInfo {
        width: u32::from_be_bytes([d[0], d[1], d[2], d[3]]),
        height: u32::from_be_bytes([d[4], d[5], d[6], d[7]]),
        bits: d[8] as u32,
        channels: 0,
    })
}

const M_SOS: u32 = 0xda;
const M_EOI: u32 = 0xd9;
const M_APP0: u32 = 0xe0;

/// php_next_marker.
fn next_marker(
    host: &mut dyn Host,
    fname: &str,
    s: &mut Stream,
    ff_read: bool,
) -> Result<u32, Thrown> {
    if !ff_read {
        let mut extraneous = 0usize;
        loop {
            match s.getc() {
                0xff => break,
                -1 => return Ok(M_EOI),
                _ => extraneous += 1,
            }
        }
        if extraneous > 0 {
            let m = format!("Corrupt JPEG data: {extraneous} extraneous bytes before marker");
            crate::warn(host, fname, &m)?;
        }
    }
    loop {
        match s.getc() {
            -1 => return Ok(M_EOI),
            0xff => {}
            m => return Ok(m as u32),
        }
    }
}

/// php_skip_variable.
fn skip_variable(s: &mut Stream) -> bool {
    let length = s.read2() as i64;
    if length < 2 {
        return false;
    }
    s.seek_cur(length - 2);
    true
}

/// php_read_APP: the first block of each APPn kind goes into `info`.
fn read_app(s: &mut Stream, marker: u32, info: &mut PhpArray) -> bool {
    let length = s.read2() as usize;
    if length < 2 {
        return false;
    }
    let length = length - 2;
    let buf = s.read(length);
    if buf.len() != length {
        return false;
    }
    let name = format!("APP{}", marker - M_APP0);
    let key = rhp_values::ArrayKey::from_bytes(name.as_bytes());
    if info.get(&key).is_none() {
        info.insert(key, Value::str(buf));
    }
    true
}

fn jpeg(
    host: &mut dyn Host,
    fname: &str,
    s: &mut Stream,
    mut info: Option<&mut PhpArray>,
) -> Result<Option<GfxInfo>, Thrown> {
    let mut result: Option<GfxInfo> = None;
    let mut ff_read = true;
    loop {
        let marker = next_marker(host, fname, s, ff_read)?;
        ff_read = false;
        match marker {
            0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => {
                if result.is_none() {
                    let length = s.read2() as u16;
                    let bits = s.getc() as u32;
                    let height = s.read2();
                    let width = s.read2();
                    let channels = s.getc() as u32;
                    result = Some(GfxInfo {
                        width,
                        height,
                        bits,
                        channels,
                    });
                    if info.is_none() || length < 8 {
                        return Ok(result);
                    }
                    if !s.seek_cur(length as i64 - 8) {
                        return Ok(result);
                    }
                } else if !skip_variable(s) {
                    return Ok(result);
                }
            }
            0xe0..=0xef => {
                let ok = match info.as_deref_mut() {
                    Some(info) => read_app(s, marker, info),
                    None => skip_variable(s),
                };
                if !ok {
                    return Ok(result);
                }
            }
            M_SOS | M_EOI => return Ok(result),
            _ => {
                if !skip_variable(s) {
                    return Ok(result);
                }
            }
        }
    }
}

fn jpc(host: &mut dyn Host, fname: &str, s: &mut Stream) -> Result<Option<GfxInfo>, Thrown> {
    // The byte after the SOC marker must be SIZ.
    if s.getc() as u8 != 0x51 {
        crate::warn(
            host,
            fname,
            "JPEG2000 codestream corrupt(Expected SIZ marker not found after SOC)",
        )?;
        return Ok(None);
    }
    s.read2(); // Lsiz
    s.read2(); // Rsiz
    let width = s.read4();
    let height = s.read4();
    if !s.seek_cur(24) {
        return Ok(None);
    }
    let channels = s.read2();
    if (channels == 0 && s.eof) || channels > 256 {
        return Ok(None);
    }
    let mut highest = 0i32;
    for _ in 0..channels {
        let depth = s.getc() + 1;
        highest = highest.max(depth);
        s.getc();
        s.getc();
    }
    Ok(Some(GfxInfo {
        width,
        height,
        bits: highest as u32,
        channels,
    }))
}

fn jp2(host: &mut dyn Host, fname: &str, s: &mut Stream) -> Result<Option<GfxInfo>, Thrown> {
    let mut result = None;
    loop {
        let box_length = s.read4();
        let mut box_type = [0u8; 4];
        if !s.read_into(&mut box_type) {
            break;
        }
        if box_length == 1 {
            // XLBoxes are not handled.
            return Ok(None);
        }
        if &box_type == b"jp2c" {
            s.seek_cur(3);
            result = jpc(host, fname, s)?;
            break;
        }
        if box_length as i32 <= 0 {
            break;
        }
        if !s.seek_cur(box_length as i64 - 8) {
            break;
        }
    }
    if result.is_none() {
        crate::warn(host, fname, "JP2 file has no codestreams at root level")?;
    }
    Ok(result)
}

fn ifd16(b: &[u8], mm: bool) -> u32 {
    if mm {
        (b[0] as u32) << 8 | b[1] as u32
    } else {
        (b[1] as u32) << 8 | b[0] as u32
    }
}

fn ifd32(b: &[u8], mm: bool) -> u32 {
    let a = [b[0], b[1], b[2], b[3]];
    if mm {
        u32::from_be_bytes(a)
    } else {
        u32::from_le_bytes(a)
    }
}

fn tiff(s: &mut Stream, mm: bool) -> Option<GfxInfo> {
    let mut p = [0u8; 4];
    if !s.read_into(&mut p) {
        return None;
    }
    let ifd_addr = ifd32(&p, mm) as usize;
    // size_t arithmetic, then php's zend_off_t.
    if !s.seek_cur(ifd_addr.wrapping_sub(8) as i64) {
        return None;
    }
    let mut n = [0u8; 2];
    if !s.read_into(&mut n) {
        return None;
    }
    let num_entries = ifd16(&n, mm) as usize;
    let dir = s.read(12 * num_entries + 4);
    if dir.len() != 12 * num_entries + 4 {
        return None;
    }
    let (mut width, mut height) = (0usize, 0usize);
    for e in dir.as_chunks::<12>().0.iter().take(num_entries) {
        let tag = ifd16(&e[0..], mm);
        let value = match ifd16(&e[2..], mm) {
            1 | 6 => e[8] as usize,
            3 => ifd16(&e[8..], mm) as usize,
            8 => ifd16(&e[8..], mm) as i16 as isize as usize,
            4 => ifd32(&e[8..], mm) as usize,
            9 => ifd32(&e[8..], mm) as i32 as isize as usize,
            _ => continue,
        };
        match tag {
            0x0100 | 0xa002 => width = value,
            0x0101 | 0xa003 => height = value,
            _ => {}
        }
    }
    (width != 0 && height != 0).then_some(GfxInfo {
        width: width as u32,
        height: height as u32,
        ..Default::default()
    })
}

fn iff(s: &mut Stream) -> Option<GfxInfo> {
    let mut a = [0u8; 10];
    if !s.read_into(&mut a[..8]) {
        return None;
    }
    if &a[4..8] != b"ILBM" && &a[4..8] != b"PBM " {
        return None;
    }
    loop {
        if !s.read_into(&mut a[..8]) {
            return None;
        }
        let chunk_id = ifd32(&a[0..], true) as i32;
        let mut size = ifd32(&a[4..], true) as i32;
        if size < 0 {
            return None;
        }
        if size & 1 == 1 {
            if size == i32::MAX {
                return None;
            }
            size += 1;
        }
        if chunk_id == 0x424d4844 {
            // BMHD
            if size < 9 || !s.read_into(&mut a[..9]) {
                return None;
            }
            let width = ifd16(&a[0..], true) as i16;
            let height = ifd16(&a[2..], true) as i16;
            let bits = a[8] as i16;
            if width > 0 && height > 0 && bits > 0 && bits < 33 {
                return Some(GfxInfo {
                    width: width as u32,
                    height: height as u32,
                    bits: bits as u32,
                    channels: 0,
                });
            }
        } else if !s.seek_cur(size as i64) {
            return None;
        }
    }
}

/// php_get_wbmp: the size, when the stream is a WBMP.
fn get_wbmp(s: &mut Stream) -> Option<(u32, u32)> {
    s.rewind();
    if s.getc() != 0 {
        return None;
    }
    loop {
        let i = s.getc();
        if i < 0 {
            return None;
        }
        if i & 0x80 == 0 {
            break;
        }
    }
    let dim = |s: &mut Stream| -> Option<i32> {
        let mut v = 0i32;
        loop {
            let i = s.getc();
            if i < 0 {
                return None;
            }
            v = (v << 7) | (i & 0x7f);
            if v > 2048 {
                return None;
            }
            if i & 0x80 == 0 {
                return Some(v);
            }
        }
    };
    let width = dim(s)?;
    let height = dim(s)?;
    (width != 0 && height != 0).then_some((width as u32, height as u32))
}

/// `sscanf(line, "#define %s %d", name, &value) == 2`.
fn scan_define(line: &[u8]) -> Option<(&[u8], i32)> {
    let ws = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    let rest = line.strip_prefix(b"#define")?;
    let skip = |r: &[u8]| -> usize { r.iter().position(|&c| !ws(c)).unwrap_or(r.len()) };
    let rest = &rest[skip(rest)..];
    let n = rest.iter().position(|&c| ws(c)).unwrap_or(rest.len());
    if n == 0 {
        return None;
    }
    let (name, rest) = rest.split_at(n);
    let rest = &rest[skip(rest)..];
    let (neg, digits) = match rest.first() {
        Some(b'-') => (true, &rest[1..]),
        Some(b'+') => (false, &rest[1..]),
        _ => (false, rest),
    };
    let nd = digits.iter().take_while(|c| c.is_ascii_digit()).count();
    if nd == 0 {
        return None;
    }
    let mut v: i64 = 0;
    for &c in &digits[..nd] {
        v = v.saturating_mul(10).saturating_add((c - b'0') as i64);
    }
    if neg {
        v = -v;
    }
    Some((name, v as i32))
}

/// php_get_xbm.
fn get_xbm(s: &mut Stream) -> Option<(u32, u32)> {
    s.rewind();
    let (mut width, mut height) = (0u32, 0u32);
    while let Some(line) = s.gets() {
        // sscanf works on the C string: up to a NUL.
        let line = &line[..line.iter().position(|&c| c == 0).unwrap_or(line.len())];
        let Some((name, value)) = scan_define(line) else {
            continue;
        };
        let ty = match name.iter().rposition(|&c| c == b'_') {
            Some(i) => &name[i + 1..],
            None => name,
        };
        if ty == b"width" {
            width = value as u32;
            if height != 0 {
                break;
            }
        }
        if ty == b"height" {
            height = value as u32;
            if width != 0 {
                break;
            }
        }
    }
    (width != 0 && height != 0).then_some((width, height))
}

fn ico(s: &mut Stream) -> Option<GfxInfo> {
    let mut d = [0u8; 16];
    if !s.read_into(&mut d[..2]) {
        return None;
    }
    let mut num = u16::from_le_bytes([d[0], d[1]]) as i32;
    if !(1..=255).contains(&num) {
        return None;
    }
    let mut r = GfxInfo::default();
    while num > 0 {
        if !s.read_into(&mut d) {
            break;
        }
        let bits = u16::from_le_bytes([d[6], d[7]]) as u32;
        if bits >= r.bits {
            r.width = d[0] as u32;
            r.height = d[1] as u32;
            r.bits = bits;
        }
        num -= 1;
    }
    if r.width == 0 {
        r.width = 256;
    }
    if r.height == 0 {
        r.height = 256;
    }
    Some(r)
}

fn webp(s: &mut Stream) -> Option<GfxInfo> {
    let mut b = [0u8; 18];
    if !s.read_into(&mut b) || &b[..3] != b"VP8" {
        return None;
    }
    let b32 = |i: usize| b[i] as u32;
    let (width, height) = match b[3] {
        b' ' => (
            b32(14) + ((b32(15) & 0x3f) << 8),
            b32(16) + ((b32(17) & 0x3f) << 8),
        ),
        b'L' => (
            b32(9) + ((b32(10) & 0x3f) << 8) + 1,
            (b32(10) >> 6) + (b32(11) << 2) + ((b32(12) & 0xf) << 10) + 1,
        ),
        b'X' => (
            b32(12) + (b32(13) << 8) + (b32(14) << 16) + 1,
            b32(15) + (b32(16) << 8) + (b32(17) << 16) + 1,
        ),
        _ => return None,
    };
    Some(GfxInfo {
        width,
        height,
        bits: 8,
        channels: 0,
    })
}

fn avif(s: &mut Stream) -> Option<GfxInfo> {
    let f = rhp_csrc::avifinfo::features(s)?;
    Some(GfxInfo {
        width: f.width,
        height: f.height,
        bits: f.bit_depth,
        channels: f.num_channels,
    })
}

/// php_getimagetype: the type from the first bytes. `input` names the
/// stream in the "Error reading" notice.
fn image_type(
    host: &mut dyn Host,
    fname: &str,
    s: &mut Stream,
    input: &[u8],
) -> Result<i64, Thrown> {
    let read_error = |host: &mut dyn Host| -> Result<i64, Thrown> {
        let mut m = b"Error reading from ".to_vec();
        // A C string: getimagesizefromstring's data ends at its first NUL.
        let end = input.iter().position(|&c| c == 0).unwrap_or(input.len());
        m.extend_from_slice(&input[..end]);
        m.push(b'!');
        crate::notice(host, fname, &m)?;
        Ok(0)
    };
    let mut t = [0u8; 12];
    if !s.read_into(&mut t[..3]) {
        return read_error(host);
    }
    const PNG_SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    match &t[..3] {
        b"GIF" => return Ok(GIF),
        b"\xff\xd8\xff" => return Ok(JPEG),
        b"\x89PN" => {
            if !s.read_into(&mut t[3..8]) {
                return read_error(host);
            }
            if &t[..8] == PNG_SIG {
                return Ok(PNG);
            }
            crate::warn(host, fname, "PNG file corrupted by ASCII conversion")?;
            return Ok(0);
        }
        b"FWS" => return Ok(SWF),
        b"CWS" => return Ok(SWC),
        b"8BP" => return Ok(PSD),
        [b'B', b'M', _] => return Ok(BMP),
        b"\xffO\xff" => return Ok(JPC),
        b"RIF" => {
            if !s.read_into(&mut t[3..12]) {
                return read_error(host);
            }
            return Ok(if &t[8..12] == b"WEBP" { WEBP } else { 0 });
        }
        _ => {}
    }
    if !s.read_into(&mut t[3..4]) {
        return read_error(host);
    }
    match &t[..4] {
        b"II\x2a\x00" => return Ok(TIFF_II),
        b"MM\x00\x2a" => return Ok(TIFF_MM),
        b"FORM" => return Ok(IFF),
        b"\x00\x00\x01\x00" => return Ok(ICO),
        _ => {}
    }
    // A WBMP may be shorter than 12 bytes: the error comes later.
    let twelve = s.read_into(&mut t[4..12]);
    if twelve && t == *b"\x00\x00\x00\x0cjP  \r\n\x87\n" {
        return Ok(JP2);
    }
    s.rewind();
    if rhp_csrc::avifinfo::identify(s) {
        return Ok(AVIF);
    }
    if twelve && &t[4..8] == b"ftyp" && matches!(&t[8..12], b"mif1" | b"heic" | b"heix") {
        return Ok(HEIF);
    }
    if get_wbmp(s).is_some() {
        return Ok(WBMP);
    }
    if !twelve {
        return read_error(host);
    }
    if get_xbm(s).is_some() {
        return Ok(XBM);
    }
    Ok(0)
}

/// php_getimagesize_from_stream.
fn from_stream(
    host: &mut dyn Host,
    fname: &str,
    data: &[u8],
    input: &[u8],
    info: Option<&mut PhpArray>,
) -> Result<Value, Thrown> {
    let s = &mut Stream {
        d: data,
        pos: 0,
        eof: false,
    };
    let itype = image_type(host, fname, s, input)?;
    let result = match itype {
        GIF => gif(s),
        JPEG => jpeg(host, fname, s, info)?,
        PNG => png(s),
        SWF => swf(s),
        SWC => swc(s),
        PSD => psd(s),
        BMP => bmp(s),
        TIFF_II => tiff(s, false),
        TIFF_MM => tiff(s, true),
        JPC => jpc(host, fname, s)?,
        JP2 => jp2(host, fname, s)?,
        IFF => iff(s),
        WBMP => get_wbmp(s).map(|(width, height)| GfxInfo {
            width,
            height,
            ..Default::default()
        }),
        XBM => get_xbm(s).map(|(width, height)| GfxInfo {
            width,
            height,
            ..Default::default()
        }),
        ICO => ico(s),
        WEBP => webp(s),
        AVIF => avif(s),
        HEIF => {
            s.rewind();
            avif(s)
        }
        _ => None,
    };
    let Some(r) = result else {
        return Ok(Value::Bool(false));
    };
    let mut a = PhpArray::new();
    // php_gfxinfo's fields are unsigned int, added as zend_long.
    a.push(Value::Int(r.width as i64));
    a.push(Value::Int(r.height as i64));
    a.push(Value::Int(itype));
    // snprintf's %d of the unsigned fields.
    a.push(Value::str(
        format!(
            "width=\"{}\" height=\"{}\"",
            r.width as i32, r.height as i32
        )
        .into_bytes(),
    ));
    let mut set = |k: &str, v: Value| a.insert(rhp_values::ArrayKey::from_bytes(k.as_bytes()), v);
    if r.bits != 0 {
        set("bits", Value::Int(r.bits as i64));
    }
    if r.channels != 0 {
        set("channels", Value::Int(r.channels as i64));
    }
    set("mime", Value::str(mime_type(itype).as_bytes()));
    set("width_unit", Value::str(b"px"));
    set("height_unit", Value::str(b"px"));
    Ok(Value::Array(a))
}

/// php_getimagesize_from_any.
fn from_any(host: &mut dyn Host, args: &[Value], from_path: bool) -> Result<Value, Thrown> {
    let fname = if from_path {
        "getimagesize"
    } else {
        "getimagesizefromstring"
    };
    let input = if from_path {
        crate::zend::zpp::path_param(host, args, 0, fname, "filename")?
    } else {
        conv::to_string_bytes(&deref(&args[0]))
    };
    let want_info = args.len() >= 2;
    if want_info {
        // zend_try_array_init: the old value's destructor runs now.
        crate::zend::zpp::try_assign_ref(host, args, 1, Value::Array(PhpArray::new()))?;
    }
    let data = if from_path {
        match read_path(host, fname, &input)? {
            Some(d) => d,
            None => return Ok(Value::Bool(false)),
        }
    } else {
        input.clone()
    };
    let mut info = want_info.then(PhpArray::new);
    let r = from_stream(host, fname, &data, &input, info.as_mut())?;
    if let Some(info) = info {
        if !info.is_empty() {
            crate::zend::zpp::set_ref(args, 1, Value::Array(info));
        }
    }
    Ok(r)
}

/// The bytes behind `path`: php:// and user wrapper streams read through, other
/// paths read whole (php copies an unseekable stream to a temp one anyway).
fn read_path(host: &mut dyn Host, fname: &str, path: &[u8]) -> Result<Option<Vec<u8>>, Thrown> {
    let scheme = crate::main::streams::split_scheme(path).0.to_vec();
    let via_stream = scheme.eq_ignore_ascii_case(b"php")
        || host
            .stream_wrapper(&scheme)
            .is_some_and(|w| w.has_stream_opener());
    if !via_stream {
        return crate::ext::standard::md5::read_file_for_hash(host, fname, path);
    }
    let Some(res) = host.stream_open_ctx(path, b"rb", &Value::Null) else {
        crate::open_failed(host, fname, path)?;
        return Ok(None);
    };
    let data = host.stream_read(&res, -1).unwrap_or_default();
    host.stream_close(&res);
    Ok(Some(data))
}

fn fn_getimagesize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    from_any(host, args, true)
}

fn fn_getimagesizefromstring(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    from_any(host, args, false)
}

/// php_image_type_to_mime_type.
pub(crate) fn mime_type(t: i64) -> &'static str {
    match t {
        GIF => "image/gif",
        JPEG => "image/jpeg",
        PNG => "image/png",
        SWF | SWC => "application/x-shockwave-flash",
        PSD => "image/psd",
        BMP => "image/bmp",
        TIFF_II | TIFF_MM => "image/tiff",
        IFF => "image/iff",
        WBMP => "image/vnd.wap.wbmp",
        JPC => "application/octet-stream",
        JP2 => "image/jp2",
        XBM => "image/xbm",
        ICO => "image/vnd.microsoft.icon",
        WEBP => "image/webp",
        AVIF => "image/avif",
        HEIF => "image/heif",
        SVG => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn fn_image_type_to_mime_type(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let t = conv::to_int(&deref(&args[0]));
    Ok(Value::str(mime_type(t).as_bytes()))
}

fn fn_image_type_to_extension(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let t = conv::to_int(&deref(&args[0]));
    let inc_dot = args.get(1).is_none_or(|v| conv::to_bool(&deref(v)));
    let ext = match t {
        GIF => ".gif",
        JPEG => ".jpeg",
        PNG => ".png",
        SWF | SWC => ".swf",
        PSD => ".psd",
        BMP | WBMP => ".bmp",
        TIFF_II | TIFF_MM => ".tiff",
        IFF => ".iff",
        JPC => ".jpc",
        JP2 => ".jp2",
        JPX => ".jpx",
        JB2 => ".jb2",
        XBM => ".xbm",
        ICO => ".ico",
        WEBP => ".webp",
        AVIF => ".avif",
        HEIF => ".heif",
        SVG => ".svg",
        _ => return Ok(Value::Bool(false)),
    };
    let ext = if inc_dot { ext } else { &ext[1..] };
    Ok(Value::str(ext.as_bytes()))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("getimagesize", fn_getimagesize),
            ("getimagesizefromstring", fn_getimagesizefromstring),
            ("image_type_to_mime_type", fn_image_type_to_mime_type),
            ("image_type_to_extension", fn_image_type_to_extension),
        ],
    );
}
