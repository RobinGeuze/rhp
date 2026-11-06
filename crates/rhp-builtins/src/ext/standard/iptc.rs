//! ext/standard/iptc.c: `iptcparse()` and `iptcembed()`.

use rhp_values::{conv, ops::deref, ArrayKey, PhpArray, Value};

use crate::{Host, Thrown};

const M_SOI: u32 = 0xd8;
const M_EOI: u32 = 0xd9;
const M_SOS: u32 = 0xda;
const M_APP0: u32 = 0xe0;
const M_APP1: u32 = 0xe1;
const M_APP13: u32 = 0xed;

/// iptcembed's reader over the file's bytes, echoing what it copies to the
/// output (`spool > 0`) and/or into the result (`spool < 2`).
struct Embed<'a> {
    data: &'a [u8],
    pos: usize,
    spool: i64,
    buf: Option<Vec<u8>>,
    /// The result buffer's size: php allocates the IPTC data, the header
    /// and 1025 bytes more than the file's fstat size.
    cap: usize,
}

impl Embed<'_> {
    /// php_iptc_put1: false when the result buffer is full.
    fn put1(&mut self, host: &mut dyn Host, c: u8) -> bool {
        if self.spool > 0 {
            host.write_out(&[c]);
        }
        if let Some(b) = &mut self.buf {
            if b.len() >= self.cap {
                return false;
            }
            b.push(c);
        }
        true
    }

    /// php_iptc_get1 (`copy`: false is php's spool 0 without a buffer).
    fn get1(&mut self, host: &mut dyn Host, copy: bool) -> Option<u8> {
        let c = *self.data.get(self.pos)?;
        self.pos += 1;
        // A full result buffer reads as EOF (the byte is still spooled).
        if copy && !self.put1(host, c) {
            return None;
        }
        Some(c)
    }

    fn read_remaining(&mut self, host: &mut dyn Host) {
        while self.get1(host, true).is_some() {}
    }

    fn skip_variable(&mut self, host: &mut dyn Host, copy: bool) {
        let Some(c1) = self.get1(host, copy) else {
            return;
        };
        let Some(c2) = self.get1(host, copy) else {
            return;
        };
        let len = ((c1 as u32) << 8 | c2 as u32).wrapping_sub(2);
        for _ in 0..len {
            if self.get1(host, copy).is_none() {
                return;
            }
        }
    }

    fn next_marker(&mut self, host: &mut dyn Host) -> u32 {
        let Some(mut c) = self.get1(host, true) else {
            return M_EOI;
        };
        while c != 0xff {
            match self.get1(host, true) {
                Some(n) => c = n,
                None => return M_EOI,
            }
        }
        // The marker byte, swallowing padding 0xff bytes.
        loop {
            match self.get1(host, false) {
                None => return M_EOI,
                Some(0xff) => {
                    self.put1(host, 0xff);
                }
                Some(m) => return m as u32,
            }
        }
    }
}

fn fn_iptcembed(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut iptc = conv::to_string_bytes(&deref(&args[0]));
    let path = crate::zend::zpp::path_param(host, args, 1, "iptcembed", "filename")?;
    let spool = args.get(2).map_or(0, |v| conv::to_int(&deref(v)));
    let Ok(data) = std::fs::read(crate::zend::virtual_cwd::os_path(&path)) else {
        let msg = [b"Unable to open ".as_slice(), &path].concat();
        crate::warn(host, "iptcembed", &msg)?;
        return Ok(Value::Bool(false));
    };
    let st_size =
        std::fs::metadata(crate::zend::virtual_cwd::os_path(&path)).map_or(0, |m| m.len() as usize);
    let mut e = Embed {
        data: &data,
        pos: 0,
        spool,
        buf: (spool < 2).then(Vec::new),
        cap: iptc.len() + 29 + 1025 + st_size,
    };
    if e.get1(host, true) != Some(0xff) || e.get1(host, true) != Some(M_SOI as u8) {
        return Ok(Value::Bool(false));
    }
    let mut written = false;
    loop {
        let marker = e.next_marker(host);
        if marker == M_EOI {
            break;
        }
        if marker != M_APP13 && !e.put1(host, marker as u8) {
            return Ok(Value::Bool(false));
        }
        match marker {
            M_APP13 => {
                // The old APP13 is replaced: skip it (and the 0xff already
                // copied after it).
                e.skip_variable(host, false);
                e.get1(host, false);
                e.read_remaining(host);
                break;
            }
            M_APP0 | M_APP1 if !written => {
                written = true;
                e.skip_variable(host, true);
                if iptc.len() & 1 == 1 {
                    // Made even (php reads the string's terminating NUL).
                    iptc.push(0);
                }
                let n = iptc.len();
                let mut ps = *b"\xFF\xED\0\0Photoshop 3.0\x008BIM\x04\x04\0\0\0\0";
                ps[2] = ((n + 28) >> 8) as u8;
                ps[3] = ((n + 28) & 0xff) as u8;
                let bytes = ps.iter().copied().chain([(n >> 8) as u8, (n & 0xff) as u8]);
                for c in bytes.chain(iptc.iter().copied()) {
                    if !e.put1(host, c) {
                        return Ok(Value::Bool(false));
                    }
                }
            }
            M_APP0 | M_APP1 => {}
            M_SOS => {
                e.read_remaining(host);
                break;
            }
            _ => e.skip_variable(host, true),
        }
    }
    Ok(match e.buf {
        Some(b) => Value::str(b),
        None => Value::Bool(true),
    })
}

fn fn_iptcparse(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&deref(&args[0]));
    // php reads one past the end: the string's NUL terminator.
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let n = s.len();
    let mut inx = 0;
    while inx < n {
        if at(inx) == 0x1c && (at(inx + 1) == 0x01 || at(inx + 1) == 0x02) {
            break;
        }
        inx += 1;
    }
    let mut out: Option<PhpArray> = None;
    while inx < n {
        if at(inx) != 0x1c {
            break;
        }
        inx += 1;
        if inx + 4 >= n {
            break;
        }
        let dataset = at(inx);
        let recnum = at(inx + 1);
        inx += 2;
        let len = if at(inx) & 0x80 != 0 {
            if inx + 6 >= n {
                break;
            }
            let l = (at(inx + 2) as usize) << 24
                | (at(inx + 3) as usize) << 16
                | (at(inx + 4) as usize) << 8
                | at(inx + 5) as usize;
            inx += 6;
            l
        } else {
            let l = (at(inx) as usize) << 8 | at(inx + 1) as usize;
            inx += 2;
            l
        };
        if len > n || inx + len > n {
            break;
        }
        let key = ArrayKey::from_bytes(format!("{dataset}#{recnum:03}").as_bytes());
        let a = out.get_or_insert_with(PhpArray::new);
        let mut list = match a.get(&key) {
            Some(Value::Array(l)) => l.clone(),
            _ => PhpArray::new(),
        };
        list.push(Value::str(s[inx..inx + len].to_vec()));
        a.insert(key, Value::Array(list));
        inx += len;
    }
    Ok(out.map_or(Value::Bool(false), Value::Array))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[("iptcembed", fn_iptcembed), ("iptcparse", fn_iptcparse)],
    );
}
