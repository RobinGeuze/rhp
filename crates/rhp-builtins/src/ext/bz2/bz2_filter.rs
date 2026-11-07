//! ext/bz2/bz2_filter.c: the bzip2.compress and bzip2.decompress stream
//! filters (libbz2 through the bzip2 crate, 2048-byte windows as php).

use bzip2::{Action, Compress, Compression, Decompress, Status as BzStatus};
use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, Value};

use crate::main::streams::filter::{
    filter_warning, Brigade, FilterOps, Status, PSFS_FLAG_FLUSH_CLOSE, PSFS_FLAG_FLUSH_INC,
};
use crate::Host;

const BUF_LEN: usize = 2048;

enum DState {
    Uninit,
    Running(Decompress),
    Finished,
}

struct BzDecompress {
    state: DState,
    small: bool,
    concatenated: bool,
}

struct BzCompress {
    strm: Compress,
    flushed: bool,
}

impl FilterOps for BzDecompress {
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
        let mut n = 0usize;
        let mut out = [0u8; BUF_LEN];
        while let Some(b) = inb.pop() {
            let buf = std::mem::take(&mut b.borrow_mut().buf);
            let mut bin = 0;
            while bin < buf.len() {
                if matches!(self.state, DState::Uninit) {
                    self.state = DState::Running(Decompress::new(self.small));
                }
                let DState::Running(d) = &mut self.state else {
                    n += buf.len();
                    break;
                };
                let desired = (buf.len() - bin).min(BUF_LEN);
                let (in0, out0) = (d.total_in(), d.total_out());
                let status = d.decompress(&buf[bin..bin + desired], &mut out);
                let used = (d.total_in() - in0) as usize;
                let produced = (d.total_out() - out0) as usize;
                let end = match status {
                    Ok(BzStatus::StreamEnd) => {
                        self.state = if self.concatenated {
                            DState::Uninit
                        } else {
                            DState::Finished
                        };
                        true
                    }
                    Ok(BzStatus::Ok) => false,
                    _ => {
                        filter_warning(host, crate::E_NOTICE, "bzip2 decompression failed");
                        return Status::ErrFatal;
                    }
                };
                n += used;
                bin += used;
                if produced > 0 {
                    outb.append_data(out[..produced].to_vec());
                    exit = Status::PassOn;
                } else if end {
                    return Status::PassOn;
                }
            }
        }
        if flags & PSFS_FLAG_FLUSH_CLOSE != 0 {
            if let DState::Running(d) = &mut self.state {
                loop {
                    let out0 = d.total_out();
                    let status = d.decompress(&[], &mut out);
                    let produced = (d.total_out() - out0) as usize;
                    if produced > 0 {
                        outb.append_data(out[..produced].to_vec());
                        exit = Status::PassOn;
                    } else if matches!(status, Ok(BzStatus::Ok)) {
                        break;
                    }
                    if !matches!(status, Ok(BzStatus::Ok)) {
                        break;
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

impl FilterOps for BzCompress {
    fn filter(
        &mut self,
        _host: &mut dyn Host,
        _stream: &Value,
        inb: &Brigade,
        outb: &Brigade,
        consumed: Option<&mut usize>,
        flags: i32,
    ) -> Status {
        let mut exit = Status::FeedMe;
        let mut n = 0usize;
        let mut out = [0u8; BUF_LEN];
        let close = flags & PSFS_FLAG_FLUSH_CLOSE != 0;
        while let Some(b) = inb.pop() {
            let buf = std::mem::take(&mut b.borrow_mut().buf);
            let mut bin = 0;
            while bin < buf.len() {
                let desired = (buf.len() - bin).min(BUF_LEN);
                let action = if close {
                    Action::Finish
                } else if flags & PSFS_FLAG_FLUSH_INC != 0 {
                    Action::Flush
                } else {
                    Action::Run
                };
                self.flushed = action != Action::Run;
                let (in0, out0) = (self.strm.total_in(), self.strm.total_out());
                match self
                    .strm
                    .compress(&buf[bin..bin + desired], &mut out, action)
                {
                    Ok(BzStatus::RunOk | BzStatus::FlushOk | BzStatus::FinishOk) => {}
                    _ => return Status::ErrFatal,
                }
                let used = (self.strm.total_in() - in0) as usize;
                let produced = (self.strm.total_out() - out0) as usize;
                n += used;
                bin += used;
                if produced > 0 {
                    outb.append_data(out[..produced].to_vec());
                    exit = Status::PassOn;
                }
            }
        }
        if close || (flags & PSFS_FLAG_FLUSH_INC != 0 && !self.flushed) {
            let (action, again) = if close {
                (Action::Finish, BzStatus::FinishOk)
            } else {
                (Action::Flush, BzStatus::FlushOk)
            };
            loop {
                let out0 = self.strm.total_out();
                let status = self.strm.compress(&[], &mut out, action);
                self.flushed = true;
                let produced = (self.strm.total_out() - out0) as usize;
                if produced > 0 {
                    outb.append_data(out[..produced].to_vec());
                    exit = Status::PassOn;
                }
                if status.ok() != Some(again) {
                    break;
                }
            }
        }
        if let Some(c) = consumed {
            *c = n;
        }
        exit
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

/// php_bz2_filter_create.
pub(crate) fn create(
    host: &mut dyn Host,
    name: &[u8],
    params: Option<&Value>,
) -> Option<Box<dyn FilterOps>> {
    let params = params.map(deref);
    if name.eq_ignore_ascii_case(b"bzip2.decompress") {
        let (mut small, mut concatenated) = (false, false);
        match &params {
            None => {}
            Some(p @ (Value::Array(_) | Value::Object(_))) => {
                if let Some(v) = param(p, "concatenated") {
                    concatenated = conv::to_bool(&v);
                }
                if let Some(v) = param(p, "small") {
                    small = conv::to_bool(&v);
                }
            }
            Some(v) => small = conv::to_bool(v),
        }
        return Some(Box::new(BzDecompress {
            state: DState::Uninit,
            small,
            concatenated,
        }));
    }
    if name.eq_ignore_ascii_case(b"bzip2.compress") {
        // PHP_BZ2_FILTER_DEFAULT_BLOCKSIZE / _WORKFACTOR
        let (mut blocks, mut work) = (4u32, 0u32);
        if let Some(p @ (Value::Array(_) | Value::Object(_))) = &params {
            if let Some(v) = param(p, "blocks") {
                let b = conv::to_int(&v);
                if !(1..=9).contains(&b) {
                    let m =
                        format!("Invalid parameter given for number of blocks to allocate ({b})");
                    filter_warning(host, crate::E_WARNING, &m);
                } else {
                    blocks = b as u32;
                }
            }
            if let Some(v) = param(p, "work") {
                let w = conv::to_int(&v);
                if !(0..=250).contains(&w) {
                    let m = format!("Invalid parameter given for work factor ({w})");
                    filter_warning(host, crate::E_WARNING, &m);
                } else {
                    work = w as u32;
                }
            }
        }
        return Some(Box::new(BzCompress {
            strm: Compress::new(Compression::new(blocks), work),
            flushed: true,
        }));
    }
    None
}
