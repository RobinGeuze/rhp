//! ext/standard/filters.c: string.rot13 / toupper / tolower, the convert.*
//! filters (base64 and quoted-printable, with php's php_conv state
//! machines), consumed and dechunk.

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::main::streams::filter::{
    filter_warning, Brigade, FilterOps, Status, PSFS_FLAG_FLUSH_CLOSE,
};
use crate::main::streams::plain_wrapper::Dechunk;
use crate::Host;

// --- string.* ---------------------------------------------------------------

struct StrFilter(fn(u8) -> u8);

impl FilterOps for StrFilter {
    fn filter(
        &mut self,
        _host: &mut dyn Host,
        _stream: &Value,
        inb: &Brigade,
        outb: &Brigade,
        consumed: Option<&mut usize>,
        _flags: i32,
    ) -> Status {
        let mut n = 0;
        while let Some(b) = inb.pop() {
            {
                let mut bm = b.borrow_mut();
                for c in bm.buf.iter_mut() {
                    *c = (self.0)(*c);
                }
                n += bm.buf.len();
            }
            outb.append(&b);
        }
        if let Some(c) = consumed {
            *c = n;
        }
        Status::PassOn
    }
}

fn rot13(c: u8) -> u8 {
    match c {
        b'a'..=b'z' => (c - b'a' + 13) % 26 + b'a',
        b'A'..=b'Z' => (c - b'A' + 13) % 26 + b'A',
        _ => c,
    }
}

pub(crate) fn create_rot13(
    _: &mut dyn Host,
    _: &[u8],
    _: Option<&Value>,
) -> Option<Box<dyn FilterOps>> {
    Some(Box::new(StrFilter(rot13)))
}

pub(crate) fn create_toupper(
    _: &mut dyn Host,
    _: &[u8],
    _: Option<&Value>,
) -> Option<Box<dyn FilterOps>> {
    Some(Box::new(StrFilter(|c| c.to_ascii_uppercase())))
}

pub(crate) fn create_tolower(
    _: &mut dyn Host,
    _: &[u8],
    _: Option<&Value>,
) -> Option<Box<dyn FilterOps>> {
    Some(Box::new(StrFilter(|c| c.to_ascii_lowercase())))
}

// --- php_conv -----------------------------------------------------------------

/// `php_conv_err_t`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ConvErr {
    Success,
    Unknown,
    TooBig,
    InvalidSeq,
    UnexpectedEos,
}

/// The output window of a conversion: bytes go to `out` while `left` (php's
/// `ocnt`) allows.
struct Out<'a> {
    out: &'a mut Vec<u8>,
    left: usize,
}

impl Out<'_> {
    fn put(&mut self, c: u8) {
        self.out.push(c);
        self.left -= 1;
    }
    fn put_all(&mut self, s: &[u8]) {
        self.out.extend_from_slice(s);
        self.left -= s.len();
    }
}

/// The input of a conversion: `None` asks for a flush.
type In<'a, 'b> = Option<(&'a [u8], &'b mut usize)>;

trait Conv {
    fn convert(&mut self, input: In, out: &mut Out) -> ConvErr;
}

const B64_ENC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64e(i: u32) -> u8 {
    B64_ENC[(i & 0x3f) as usize]
}

struct Base64Encode {
    lbchars: Option<Vec<u8>>,
    erem: [u8; 3],
    erem_len: usize,
    line_ccnt: u32,
    line_len: u32,
}

impl Base64Encode {
    /// Emit a line break when the line is full.
    fn wrap(&self, line_ccnt: &mut u32, out: &mut Out) -> bool {
        if let Some(lb) = &self.lbchars {
            if *line_ccnt < 4 {
                if out.left < lb.len() {
                    return false;
                }
                out.put_all(lb);
                *line_ccnt = self.line_len;
            }
        }
        true
    }

    fn flush(&mut self, out: &mut Out) -> ConvErr {
        let mut line_ccnt = self.line_ccnt;
        let mut err = ConvErr::Success;
        match self.erem_len {
            0 => {}
            1 | 2 => {
                if !self.wrap(&mut line_ccnt, out) {
                    return ConvErr::TooBig;
                }
                if out.left < 4 {
                    err = ConvErr::TooBig;
                } else {
                    let e0 = self.erem[0] as u32;
                    out.put(b64e(e0 >> 2));
                    if self.erem_len == 1 {
                        out.put(b64e(e0 << 4));
                        out.put(b'=');
                    } else {
                        let e1 = self.erem[1] as u32;
                        out.put(b64e((e0 << 4) | (e1 >> 4)));
                        out.put(b64e(e1 << 2));
                    }
                    out.put(b'=');
                    self.erem_len = 0;
                    line_ccnt = line_ccnt.wrapping_sub(4);
                }
            }
            _ => err = ConvErr::Unknown,
        }
        self.line_ccnt = line_ccnt;
        err
    }
}

impl Conv for Base64Encode {
    fn convert(&mut self, input: In, out: &mut Out) -> ConvErr {
        let Some((buf, pos)) = input else {
            return self.flush(out);
        };
        let mut err = ConvErr::Success;
        let mut line_ccnt = self.line_ccnt;
        let mut p = *pos;
        let icnt = |p: usize| buf.len() - p;
        'out: {
            // consume the remainder first
            if (self.erem_len == 1 && icnt(p) >= 2) || (self.erem_len == 2 && icnt(p) >= 1) {
                // php tests `inst->line_ccnt` (not the local) for erem_len 2
                let mut lc = if self.erem_len == 2 {
                    self.line_ccnt
                } else {
                    line_ccnt
                };
                let wrapped_before = lc;
                if !self.wrap(&mut lc, out) {
                    return ConvErr::TooBig;
                }
                if lc != wrapped_before {
                    line_ccnt = lc;
                }
                if out.left < 4 {
                    err = ConvErr::TooBig;
                    break 'out;
                }
                let e0 = self.erem[0] as u32;
                out.put(b64e(e0 >> 2));
                if self.erem_len == 1 {
                    let (a, b) = (buf[p] as u32, buf[p + 1] as u32);
                    out.put(b64e((e0 << 4) | (a >> 4)));
                    out.put(b64e((a << 2) | (b >> 6)));
                    out.put(b64e(b));
                    p += 2;
                } else {
                    let e1 = self.erem[1] as u32;
                    let a = buf[p] as u32;
                    out.put(b64e((e0 << 4) | (e1 >> 4)));
                    out.put(b64e((e1 << 2) | (a >> 6)));
                    out.put(b64e(a));
                    p += 1;
                }
                self.erem_len = 0;
                line_ccnt = line_ccnt.wrapping_sub(4);
            }
            while icnt(p) >= 3 {
                if !self.wrap(&mut line_ccnt, out) {
                    err = ConvErr::TooBig;
                    break 'out;
                }
                if out.left < 4 {
                    err = ConvErr::TooBig;
                    break 'out;
                }
                let (a, b, c) = (buf[p] as u32, buf[p + 1] as u32, buf[p + 2] as u32);
                out.put(b64e(a >> 2));
                out.put(b64e((a << 4) | (b >> 4)));
                out.put(b64e((b << 2) | (c >> 6)));
                out.put(b64e(c));
                p += 3;
                line_ccnt = line_ccnt.wrapping_sub(4);
            }
            while p < buf.len() {
                self.erem[self.erem_len] = buf[p];
                self.erem_len += 1;
                p += 1;
            }
        }
        *pos = p;
        self.line_ccnt = line_ccnt;
        err
    }
}

#[derive(Default)]
struct Base64Decode {
    urem: u32,
    urem_nbits: u32,
    ustat: u32,
    eos: bool,
}

fn b64_dec(c: u8) -> u32 {
    match c {
        b'A'..=b'Z' => (c - b'A') as u32,
        b'a'..=b'z' => (c - b'a' + 26) as u32,
        b'0'..=b'9' => (c - b'0' + 52) as u32,
        b'+' => 62,
        b'/' => 63,
        b'=' => 128,
        _ => 64,
    }
}

fn bmask(a: u32) -> u32 {
    0xffff >> (16 - a)
}

impl Conv for Base64Decode {
    fn convert(&mut self, input: In, out: &mut Out) -> ConvErr {
        let Some((buf, pos)) = input else {
            return if self.eos || self.urem_nbits == 0 {
                ConvErr::Success
            } else {
                ConvErr::UnexpectedEos
            };
        };
        let mut err = ConvErr::Success;
        let mut p = *pos;
        let (mut urem, mut urem_nbits, mut ustat) = (self.urem, self.urem_nbits, self.ustat);
        let mut pack: u32 = 0;
        let mut pack_bcnt: u32 = 8;
        loop {
            if pack_bcnt >= urem_nbits {
                pack_bcnt -= urem_nbits;
                pack |= urem << pack_bcnt;
                urem_nbits = 0;
            } else {
                urem_nbits -= pack_bcnt;
                pack |= urem >> urem_nbits;
                urem &= bmask(urem_nbits);
                pack_bcnt = 0;
            }
            if pack_bcnt > 0 {
                if p >= buf.len() {
                    break;
                }
                let i = b64_dec(buf[p]);
                p += 1;
                ustat |= i & 0x80;
                if i & 0xc0 == 0 {
                    if ustat != 0 {
                        err = ConvErr::InvalidSeq;
                        break;
                    }
                    if 6 <= pack_bcnt {
                        pack_bcnt -= 6;
                        pack |= i << pack_bcnt;
                        urem = 0;
                    } else {
                        urem_nbits = 6 - pack_bcnt;
                        pack |= i >> urem_nbits;
                        urem = i & bmask(urem_nbits);
                        pack_bcnt = 0;
                    }
                } else if ustat != 0 {
                    if pack_bcnt == 8 || pack_bcnt == 2 {
                        err = ConvErr::InvalidSeq;
                        break;
                    }
                    self.eos = true;
                }
            }
            if (pack_bcnt | ustat) == 0 {
                if out.left < 1 {
                    err = ConvErr::TooBig;
                    break;
                }
                out.put(pack as u8);
                pack = 0;
                pack_bcnt = 8;
            }
        }
        if urem_nbits >= pack_bcnt {
            urem |= pack << (urem_nbits - pack_bcnt);
        } else {
            urem |= pack >> (pack_bcnt - urem_nbits);
        }
        urem_nbits += 8 - pack_bcnt;
        self.urem = urem;
        self.urem_nbits = urem_nbits;
        self.ustat = ustat;
        *pos = p;
        err
    }
}

const QP_OPT_BINARY: i32 = 1;
const QP_OPT_FORCE_ENCODE_FIRST: i32 = 2;

struct QprintEncode {
    lbchars: Option<Vec<u8>>,
    opts: i32,
    line_ccnt: u32,
    line_len: u32,
    lb_ptr: u32,
    lb_cnt: u32,
}

impl Conv for QprintEncode {
    fn convert(&mut self, input: In, out: &mut Out) -> ConvErr {
        const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
        let Some((buf, pos)) = input else {
            return ConvErr::Success;
        };
        let mut err = ConvErr::Success;
        let mut line_ccnt = self.line_ccnt;
        let opts = self.opts;
        let mut lb_ptr = self.lb_ptr;
        let mut lb_cnt = self.lb_cnt;
        let mut p = *pos;
        let mut trail_ws: u32 = 0;
        let lb: &[u8] = self.lbchars.as_deref().unwrap_or(&[]);
        let has_lb = self.lbchars.is_some();
        macro_rules! consume {
            () => {
                if lb_ptr < lb_cnt {
                    lb_ptr += 1;
                } else {
                    lb_cnt = 0;
                    lb_ptr = 0;
                    p += 1;
                }
            };
        }
        loop {
            let icnt = buf.len() - p;
            if opts & QP_OPT_BINARY == 0 && has_lb && !lb.is_empty() {
                // look ahead for the line break chars
                if icnt > 0 && buf[p] == lb[lb_cnt as usize] {
                    lb_cnt += 1;
                    if lb_cnt as usize >= lb.len() {
                        if out.left < lb_cnt as usize {
                            lb_cnt -= 1;
                            err = ConvErr::TooBig;
                            break;
                        }
                        out.put_all(&lb[..lb_cnt as usize]);
                        line_ccnt = self.line_len;
                        lb_ptr = 0;
                        lb_cnt = 0;
                    }
                    p += 1;
                    continue;
                }
            }
            if lb_ptr >= lb_cnt && icnt == 0 {
                break;
            }
            let c = if lb_ptr < lb_cnt {
                lb[lb_ptr as usize]
            } else {
                buf[p]
            } as u32;
            if opts & QP_OPT_BINARY == 0 && trail_ws == 0 && (c == b'\t' as u32 || c == b' ' as u32)
            {
                if line_ccnt < 2 && has_lb {
                    if out.left < lb.len() + 1 {
                        err = ConvErr::TooBig;
                        break;
                    }
                    out.put(b'=');
                    out.put_all(lb);
                    line_ccnt = self.line_len;
                } else {
                    if out.left < 1 {
                        err = ConvErr::TooBig;
                        break;
                    }
                    // Check to see if this is EOL whitespace.
                    if has_lb {
                        let mut lb_cnt2 = 0usize;
                        let mut q = p;
                        trail_ws = 1;
                        let mut j = icnt.wrapping_sub(1);
                        while j > 0 && j != usize::MAX {
                            let c2 = buf[q];
                            if c2 == lb[lb_cnt2] {
                                lb_cnt2 += 1;
                                if lb_cnt2 >= lb.len() {
                                    break;
                                }
                            } else if lb_cnt2 != 0 || (c2 != b'\t' && c2 != b' ') {
                                trail_ws = 0;
                                break;
                            } else {
                                trail_ws += 1;
                            }
                            j -= 1;
                            q += 1;
                        }
                    }
                    if trail_ws == 0 {
                        out.put(c as u8);
                        line_ccnt = line_ccnt.wrapping_sub(1);
                        consume!();
                    }
                }
            } else if (opts & QP_OPT_FORCE_ENCODE_FIRST == 0 || line_ccnt < self.line_len)
                && ((33..=60).contains(&c) || (62..=126).contains(&c))
            {
                if line_ccnt < 2 && has_lb {
                    if out.left < lb.len() + 1 {
                        err = ConvErr::TooBig;
                        break;
                    }
                    out.put(b'=');
                    out.put_all(lb);
                    line_ccnt = self.line_len;
                }
                if out.left < 1 {
                    err = ConvErr::TooBig;
                    break;
                }
                out.put(c as u8);
                line_ccnt = line_ccnt.wrapping_sub(1);
                consume!();
            } else {
                if line_ccnt < 4 && has_lb {
                    if out.left < lb.len() + 1 {
                        err = ConvErr::TooBig;
                        break;
                    }
                    out.put(b'=');
                    out.put_all(lb);
                    line_ccnt = self.line_len;
                }
                if out.left < 3 {
                    err = ConvErr::TooBig;
                    break;
                }
                out.put(b'=');
                out.put(DIGITS[(c >> 4) as usize & 0xf]);
                out.put(DIGITS[(c & 0x0f) as usize]);
                line_ccnt = line_ccnt.wrapping_sub(3);
                trail_ws = trail_ws.saturating_sub(1);
                consume!();
            }
        }
        *pos = p;
        self.line_ccnt = line_ccnt;
        self.lb_ptr = lb_ptr;
        self.lb_cnt = lb_cnt;
        err
    }
}

struct QprintDecode {
    lbchars: Option<Vec<u8>>,
    scan_stat: u32,
    next_char: u32,
    lb_ptr: u32,
    lb_cnt: u32,
}

impl Conv for QprintDecode {
    fn convert(&mut self, input: In, out: &mut Out) -> ConvErr {
        let Some((buf, pos)) = input else {
            return if self.scan_stat != 0 {
                ConvErr::UnexpectedEos
            } else {
                ConvErr::Success
            };
        };
        let mut err = ConvErr::Success;
        let mut p = *pos;
        let mut scan_stat = self.scan_stat;
        let mut next_char = self.next_char;
        let mut lb_ptr = self.lb_ptr;
        let mut lb_cnt = self.lb_cnt;
        let lb: &[u8] = self.lbchars.as_deref().unwrap_or(&[]);
        let auto = self.lbchars.is_none();
        let end = buf.len();
        'out: loop {
            match scan_stat {
                0 => {
                    if p == end {
                        break 'out;
                    }
                    if buf[p] == b'=' {
                        scan_stat = 1;
                    } else {
                        if out.left < 1 {
                            err = ConvErr::TooBig;
                            break 'out;
                        }
                        out.put(buf[p]);
                    }
                    p += 1;
                }
                1..=3 => {
                    if scan_stat == 1 {
                        if p == end {
                            break 'out;
                        }
                        let c = buf[p];
                        if c == b' ' || c == b'\t' {
                            scan_stat = 4;
                            p += 1;
                            continue;
                        } else if auto && lb_cnt == 0 && c == b'\r' {
                            lb_cnt += 1;
                            scan_stat = 5;
                            p += 1;
                            continue;
                        } else if auto && lb_cnt == 0 && c == b'\n' {
                            lb_cnt = 0;
                            lb_ptr = 0;
                            scan_stat = 0;
                            p += 1;
                            continue;
                        } else if (lb_cnt as usize) < lb.len() && c == lb[lb_cnt as usize] {
                            lb_cnt += 1;
                            scan_stat = 5;
                            p += 1;
                            continue;
                        }
                    }
                    if scan_stat != 3 {
                        // case 2 (and 1 falling through)
                        if p == end {
                            break 'out;
                        }
                        let c = buf[p];
                        if !c.is_ascii_hexdigit() {
                            err = ConvErr::InvalidSeq;
                            break 'out;
                        }
                        let d = if c >= b'a' {
                            c - 0x57
                        } else if c >= b'A' {
                            c - 0x37
                        } else {
                            c - 0x30
                        };
                        next_char = (next_char << 4) | d as u32;
                        scan_stat += 1;
                        p += 1;
                        if scan_stat != 3 {
                            continue;
                        }
                    }
                    // case 3
                    if out.left < 1 {
                        err = ConvErr::TooBig;
                        break 'out;
                    }
                    out.put(next_char as u8);
                    scan_stat = 0;
                }
                4 => {
                    if p == end {
                        break 'out;
                    }
                    let c = buf[p];
                    if (lb_cnt as usize) < lb.len() && c == lb[lb_cnt as usize] {
                        lb_cnt += 1;
                        scan_stat = 5;
                    } else if c != b'\t' && c != b' ' {
                        err = ConvErr::InvalidSeq;
                        break 'out;
                    }
                    p += 1;
                }
                5 => {
                    if p == end {
                        break 'out;
                    }
                    if auto && lb_cnt == 1 && buf[p] == b'\n' {
                        lb_cnt = 0;
                        lb_ptr = 0;
                        scan_stat = 0;
                        p += 1;
                    } else if (auto && lb_cnt > 0) || lb_cnt as usize >= lb.len() {
                        // a soft line break (auto-detected, or complete)
                        lb_cnt = 0;
                        lb_ptr = 0;
                        scan_stat = 0;
                    } else if buf[p] == lb[lb_cnt as usize] {
                        lb_cnt += 1;
                        p += 1;
                    } else {
                        scan_stat = 6;
                    }
                }
                _ => {
                    if lb_ptr < lb_cnt {
                        if out.left < 1 {
                            err = ConvErr::TooBig;
                            break 'out;
                        }
                        out.put(lb[lb_ptr as usize]);
                        lb_ptr += 1;
                    } else {
                        scan_stat = 0;
                        lb_cnt = 0;
                        lb_ptr = 0;
                    }
                }
            }
        }
        *pos = p;
        self.scan_stat = scan_stat;
        self.lb_ptr = lb_ptr;
        self.lb_cnt = lb_cnt;
        self.next_char = next_char;
        err
    }
}

// --- convert.* ----------------------------------------------------------------

struct ConvertFilter {
    cd: Box<dyn Conv>,
    filtername: String,
}

impl ConvertFilter {
    fn warn(&self, host: &mut dyn Host, what: &str) {
        let msg = format!("Stream filter ({}): {what}", self.filtername);
        filter_warning(host, crate::E_WARNING, &msg);
    }

    /// `strfilter_convert_append_bucket` (php's stub is never used by these
    /// conversions: none returns PHP_CONV_ERR_MORE).
    fn append_bucket(
        &mut self,
        host: &mut dyn Host,
        outb: &Brigade,
        input: Option<&[u8]>,
        consumed: &mut usize,
    ) -> bool {
        let initial = input.map_or(64, |b| b.len());
        let mut out_buf: Vec<u8> = Vec::with_capacity(initial);
        let mut size = initial;
        let mut icnt = input.map_or(1, |b| b.len());
        let mut pos = 0usize;
        while icnt > 0 {
            let mut out = Out {
                left: size - out_buf.len(),
                out: &mut out_buf,
            };
            let err = match input {
                None => self.cd.convert(None, &mut out),
                Some(b) => {
                    let e = self.cd.convert(Some((b, &mut pos)), &mut out);
                    icnt = b.len() - pos;
                    e
                }
            };
            match err {
                ConvErr::InvalidSeq => {
                    self.warn(host, "invalid byte sequence");
                    return false;
                }
                ConvErr::Unknown => {
                    self.warn(host, "unknown error");
                    return false;
                }
                ConvErr::TooBig => size <<= 1,
                // php only reports an unexpected end for its stub
                ConvErr::Success | ConvErr::UnexpectedEos => {
                    if input.is_none() {
                        icnt = 0;
                    }
                }
            }
        }
        if !out_buf.is_empty() {
            outb.append_data(out_buf);
        }
        *consumed += input.map_or(0, |b| b.len()) - icnt;
        true
    }
}

impl FilterOps for ConvertFilter {
    fn filter(
        &mut self,
        host: &mut dyn Host,
        _stream: &Value,
        inb: &Brigade,
        outb: &Brigade,
        consumed: Option<&mut usize>,
        flags: i32,
    ) -> Status {
        let mut n = 0;
        while let Some(b) = inb.pop() {
            let data = std::mem::take(&mut b.borrow_mut().buf);
            if !self.append_bucket(host, outb, Some(&data), &mut n) {
                return Status::ErrFatal;
            }
        }
        if flags & PSFS_FLAG_FLUSH_CLOSE != 0 && !self.append_bucket(host, outb, None, &mut n) {
            return Status::ErrFatal;
        }
        if let Some(c) = consumed {
            *c = n;
        }
        Status::PassOn
    }
}

/// The convert options of a filter's parameter array.
struct Opts<'a>(Option<&'a PhpArray>);

impl Opts<'_> {
    fn get(&self, k: &str) -> Option<Value> {
        self.0?.get(&ArrayKey::from_bytes(k.as_bytes())).map(deref)
    }
    fn str(&self, k: &str) -> Option<Vec<u8>> {
        self.get(k).map(|v| conv::to_string_bytes(&v))
    }
    fn uint(&self, k: &str) -> u32 {
        self.get(k).map_or(0, |v| conv::to_int(&v).max(0) as u32)
    }
    fn bool(&self, k: &str) -> bool {
        self.get(k).is_some_and(|v| conv::to_bool(&v))
    }
    /// line-break-chars / line-length, as php_conv_open reads them.
    fn line_opts(&self) -> (u32, Option<Vec<u8>>) {
        if self.0.is_none() {
            return (0, None);
        }
        let lbchars = self.str("line-break-chars");
        let line_len = self.uint("line-length");
        if line_len < 4 {
            (line_len, None)
        } else {
            (line_len, Some(lbchars.unwrap_or_else(|| b"\r\n".to_vec())))
        }
    }
}

pub(crate) fn create_convert(
    host: &mut dyn Host,
    name: &[u8],
    params: Option<&Value>,
) -> Option<Box<dyn FilterOps>> {
    let filtername = String::from_utf8_lossy(name).into_owned();
    let params = params.map(deref);
    let arr = match &params {
        None => None,
        Some(Value::Array(a)) => Some(a.clone()),
        Some(_) => {
            let msg = format!("Stream filter ({filtername}): invalid filter parameter");
            filter_warning(host, crate::E_WARNING, &msg);
            return None;
        }
    };
    let dot = name.iter().position(|&c| c == b'.')?;
    let mode = name[dot + 1..].to_ascii_lowercase();
    let opts = Opts(arr.as_ref());
    let cd: Box<dyn Conv> = match mode.as_slice() {
        b"base64-encode" => {
            let (line_len, lbchars) = opts.line_opts();
            let line_len = if lbchars.is_some() { line_len } else { 0 };
            Box::new(Base64Encode {
                lbchars,
                erem: [0; 3],
                erem_len: 0,
                line_ccnt: line_len,
                line_len,
            })
        }
        b"base64-decode" => Box::new(Base64Decode::default()),
        b"quoted-printable-encode" => {
            let (line_len, lbchars) = opts.line_opts();
            let mut o = 0;
            if arr.is_some() {
                if opts.bool("binary") {
                    o |= QP_OPT_BINARY;
                }
                if opts.bool("force-encode-first") {
                    o |= QP_OPT_FORCE_ENCODE_FIRST;
                }
            }
            let line_len = if lbchars.is_some() { line_len } else { 0 };
            Box::new(QprintEncode {
                lbchars,
                opts: o,
                line_ccnt: line_len,
                line_len,
                lb_ptr: 0,
                lb_cnt: 0,
            })
        }
        b"quoted-printable-decode" => Box::new(QprintDecode {
            lbchars: opts.str("line-break-chars"),
            scan_stat: 0,
            next_char: 0,
            lb_ptr: 0,
            lb_cnt: 0,
        }),
        _ => return None,
    };
    Some(Box::new(ConvertFilter { cd, filtername }))
}

// --- consumed ---------------------------------------------------------------

struct Consumed {
    consumed: usize,
    offset: Option<i64>,
}

impl FilterOps for Consumed {
    fn filter(
        &mut self,
        host: &mut dyn Host,
        stream: &Value,
        inb: &Brigade,
        outb: &Brigade,
        consumed: Option<&mut usize>,
        flags: i32,
    ) -> Status {
        if self.offset.is_none() {
            self.offset = Some(host.stream_tell(stream).unwrap_or(-1));
        }
        let mut n = 0;
        while let Some(b) = inb.pop() {
            n += b.borrow().buf.len();
            outb.append(&b);
        }
        if let Some(c) = consumed {
            *c = n;
        }
        if flags & PSFS_FLAG_FLUSH_CLOSE != 0 {
            let to = self.offset.unwrap_or(0) + self.consumed as i64;
            host.stream_seek(stream, to, libc::SEEK_SET);
        }
        self.consumed += n;
        Status::PassOn
    }
}

pub(crate) fn create_consumed(
    _: &mut dyn Host,
    name: &[u8],
    _: Option<&Value>,
) -> Option<Box<dyn FilterOps>> {
    if !name.eq_ignore_ascii_case(b"consumed") {
        return None;
    }
    Some(Box::new(Consumed {
        consumed: 0,
        offset: None,
    }))
}

// --- dechunk ----------------------------------------------------------------

struct DechunkFilter(Dechunk);

impl FilterOps for DechunkFilter {
    fn filter(
        &mut self,
        _host: &mut dyn Host,
        _stream: &Value,
        inb: &Brigade,
        outb: &Brigade,
        consumed: Option<&mut usize>,
        _flags: i32,
    ) -> Status {
        let mut n = 0;
        while let Some(b) = inb.pop() {
            let data = std::mem::take(&mut b.borrow_mut().buf);
            n += data.len();
            b.borrow_mut().buf = self.0.feed(data);
            outb.append(&b);
        }
        if let Some(c) = consumed {
            *c = n;
        }
        Status::PassOn
    }
}

pub(crate) fn create_dechunk(
    _: &mut dyn Host,
    name: &[u8],
    _: Option<&Value>,
) -> Option<Box<dyn FilterOps>> {
    if !name.eq_ignore_ascii_case(b"dechunk") {
        return None;
    }
    Some(Box::new(DechunkFilter(Dechunk::default())))
}
