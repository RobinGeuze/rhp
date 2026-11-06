//! ext/iconv/iconv.c's convert.iconv.* stream filter.

use std::ffi::CString;

use rhp_values::Value;

use super::sys::{errno, Cd, Locale, FAILED};
use super::CSNMAXLEN;
use crate::main::streams::filter::{filter_warning, Brigade, FilterOps, Status, PSFS_FLAG_NORMAL};
use crate::Host;

struct IconvFilter {
    cd: Cd,
    from: String,
    to: String,
    stub: Vec<u8>,
}

enum Fail {
    Warn(&'static str),
}

impl IconvFilter {
    /// php_iconv_stream_filter_append_bucket.
    fn append_bucket(
        &mut self,
        outb: &Brigade,
        input: Option<&[u8]>,
        consumed: &mut usize,
    ) -> Result<(), Fail> {
        let _loc = Locale::enter();
        let initial = input.map_or(64, |b| b.len());
        let mut out = vec![0u8; initial];
        let mut pos = 0usize;
        let mut rest: &[u8] = input.unwrap_or(&[]);
        let mut icnt_null = 1usize;
        let grow = |out: &mut Vec<u8>| {
            let n = out.len() << 1;
            out.resize(n, 0);
        };
        if !self.stub.is_empty() {
            let mut t: Vec<u8> = std::mem::take(&mut self.stub);
            let mut tpos = 0usize;
            while tpos < t.len() {
                let mut tin = &t[tpos..];
                let before = tin.len();
                let r = self.cd.conv(&mut tin, &mut out, &mut pos);
                tpos += before - tin.len();
                if r == FAILED {
                    match errno() {
                        libc::EILSEQ => return Err(Fail::Warn("invalid multibyte sequence")),
                        libc::EINVAL => {
                            if input.is_some() {
                                if !rest.is_empty() {
                                    let left = t.len() - tpos;
                                    if left >= 128 {
                                        return Err(Fail::Warn("insufficient buffer"));
                                    }
                                    t.drain(..tpos);
                                    tpos = 0;
                                    t.push(rest[0]);
                                    rest = &rest[1..];
                                } else {
                                    break;
                                }
                            } else {
                                return Err(Fail::Warn("invalid multibyte sequence"));
                            }
                        }
                        libc::E2BIG => grow(&mut out),
                        _ => return Err(Fail::Warn("unknown error")),
                    }
                }
            }
            self.stub = t[tpos..].to_vec();
        }
        loop {
            let icnt = if input.is_some() {
                rest.len()
            } else {
                icnt_null
            };
            if icnt == 0 {
                break;
            }
            let r = if input.is_none() {
                self.cd.flush(&mut out, &mut pos)
            } else {
                self.cd.conv(&mut rest, &mut out, &mut pos)
            };
            if r == FAILED {
                match errno() {
                    libc::EILSEQ => return Err(Fail::Warn("invalid multibyte sequence")),
                    libc::EINVAL => {
                        if input.is_some() {
                            if rest.len() > 128 {
                                return Err(Fail::Warn("insufficient buffer"));
                            }
                            self.stub = rest.to_vec();
                            rest = &[];
                        } else {
                            return Err(Fail::Warn("unexpected octet values"));
                        }
                    }
                    libc::E2BIG => grow(&mut out),
                    _ => return Err(Fail::Warn("unknown error")),
                }
            } else if input.is_none() {
                icnt_null = 0;
            }
        }
        if pos > 0 {
            out.truncate(pos);
            outb.append_data(out);
        }
        *consumed += input.map_or(0, |b| b.len()) - rest.len();
        Ok(())
    }

    fn warn(&self, host: &mut dyn Host, f: Fail) {
        let Fail::Warn(what) = f;
        let msg = format!(
            "iconv stream filter (\"{}\"=>\"{}\"): {what}",
            self.from, self.to
        );
        filter_warning(host, crate::E_WARNING, &msg);
    }
}

impl FilterOps for IconvFilter {
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
            if let Err(f) = self.append_bucket(outb, Some(&data), &mut n) {
                self.warn(host, f);
                return Status::ErrFatal;
            }
        }
        if flags != PSFS_FLAG_NORMAL {
            if let Err(f) = self.append_bucket(outb, None, &mut n) {
                self.warn(host, f);
                return Status::ErrFatal;
            }
        }
        if let Some(c) = consumed {
            *c = n;
        }
        Status::PassOn
    }
}

/// php_iconv_stream_filter_factory_create: "convert.iconv.FROM/TO" (or
/// "FROM.TO").
pub(crate) fn create(
    _host: &mut dyn Host,
    name: &[u8],
    _params: Option<&Value>,
) -> Option<Box<dyn FilterOps>> {
    let first = name.iter().position(|&c| c == b'.')? + 1;
    let from_start = name[first..].iter().position(|&c| c == b'.')? + first + 1;
    let rest = &name[from_start..];
    let sep = rest.iter().position(|&c| c == b'/' || c == b'.')?;
    let (from, to) = (&rest[..sep], &rest[sep + 1..]);
    if from.len() >= CSNMAXLEN || to.len() >= CSNMAXLEN {
        return None;
    }
    let cfrom = CString::new(from).ok()?;
    let cto = CString::new(to).ok()?;
    let _loc = Locale::enter();
    let cd = Cd::open(&cto, &cfrom)?;
    Some(Box::new(IconvFilter {
        cd,
        from: String::from_utf8_lossy(from).into_owned(),
        to: String::from_utf8_lossy(to).into_owned(),
        stub: Vec::new(),
    }))
}
