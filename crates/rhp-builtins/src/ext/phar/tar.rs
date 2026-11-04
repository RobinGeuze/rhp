//! Tar-based `PharData` archives (php-src `ext/phar/tar.c`): reading a
//! (gzip / bzip2 compressed) tar into its entries and writing one back with
//! phar's own header layout (`phar_tar_writeheaders_int`: ustar magic, mode /
//! size / mtime as 7/11/11 octal digits, uid/gid left empty, names over 100
//! bytes split into `prefix`), followed by the two zero blocks. A gzip'd
//! archive is deflated like php's `zlib.deflate` stream filter (window 31).

use rhp_values::{ArrayKey, PhpArray, Value};

use crate::{Host, Thrown};

/// One archive entry, in manifest order.
pub struct Entry {
    pub name: Vec<u8>,
    pub is_dir: bool,
    pub content: Vec<u8>,
    pub mtime: i64,
    pub mode: u32,
}

/// `Phar::GZ` / `Phar::BZ2`.
pub const GZ: i64 = 4096;
pub const BZ2: i64 = 8192;

fn octal(field: &[u8]) -> u64 {
    let mut n = 0u64;
    for &c in field {
        match c {
            b'0'..=b'7' => n = n * 8 + (c - b'0') as u64,
            b' ' | 0 if n == 0 => continue,
            _ => break,
        }
    }
    n
}

fn cstr(field: &[u8]) -> &[u8] {
    let end = field.iter().position(|&c| c == 0).unwrap_or(field.len());
    &field[..end]
}

/// Decompress (gzip / bzip2, detected by magic) and parse a tar. `None`
/// when the data is not a tar archive. Returns the compression used too.
pub fn read(data: &[u8]) -> Option<(i64, Vec<Entry>)> {
    let (comp, raw) = if data.starts_with(&[0x1f, 0x8b]) {
        (GZ, crate::ext::zlib::zraw::gunzip_transparent(data)?)
    } else if data.starts_with(b"BZh") {
        (BZ2, crate::ext::bz2::decompress(data, false).ok()?)
    } else {
        (0, data.to_vec())
    };
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut long_name: Option<Vec<u8>> = None;
    while let Some(h) = raw.get(pos..pos + 512) {
        if h.iter().all(|&b| b == 0) {
            break;
        }
        // Verify the header checksum (phar_is_tar).
        let stored = octal(&h[148..156]);
        let sum: u64 = h
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                if (148..156).contains(&i) {
                    b' ' as u64
                } else {
                    b as u64
                }
            })
            .sum();
        if stored != sum {
            return if out.is_empty() {
                None
            } else {
                Some((comp, out))
            };
        }
        let size = octal(&h[124..136]) as usize;
        let typeflag = h[156];
        let body_start = pos + 512;
        let body = raw.get(body_start..body_start + size)?.to_vec();
        pos = body_start + size.div_ceil(512) * 512;
        if typeflag == b'L' {
            // GNU long name for the next entry.
            long_name = Some(cstr(&body).to_vec());
            continue;
        }
        let mut name = match long_name.take() {
            Some(n) => n,
            None => {
                let base = cstr(&h[0..100]);
                let prefix = if &h[257..262] == b"ustar" {
                    cstr(&h[345..500])
                } else {
                    b""
                };
                if prefix.is_empty() {
                    base.to_vec()
                } else {
                    [prefix, b"/", base].concat()
                }
            }
        };
        if matches!(typeflag, b'g' | b'x') {
            continue; // pax headers
        }
        let is_dir = typeflag == b'5' || name.last() == Some(&b'/');
        while name.last() == Some(&b'/') {
            name.pop();
        }
        if name.is_empty() {
            continue;
        }
        out.push(Entry {
            name,
            is_dir,
            content: if is_dir { Vec::new() } else { body },
            mtime: octal(&h[136..148]) as i64,
            mode: (octal(&h[100..108]) & 0o777) as u32,
        });
    }
    Some((comp, out))
}

fn put_octal(field: &mut [u8], val: u64, digits: usize) {
    let mut v = val;
    for i in (0..digits).rev() {
        field[i] = b'0' + (v & 7) as u8;
        v >>= 3;
    }
}

/// Serialize the entries the way phar_tar_flush does. `Err` names an entry
/// whose path is too long for the tar format.
pub fn write(entries: &[Entry]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for e in entries {
        let mut h = [0u8; 512];
        let name = &e.name;
        if name.len() > 100 {
            if name.len() > 256 {
                return Err(String::from_utf8_lossy(name).into_owned());
            }
            let mut b = name.len() - 101;
            while b < name.len() && name[b] != b'/' {
                b += 1;
            }
            if b >= name.len() || b > 155 {
                return Err(String::from_utf8_lossy(name).into_owned());
            }
            h[345..345 + b].copy_from_slice(&name[..b]);
            h[..name.len() - b - 1].copy_from_slice(&name[b + 1..]);
        } else {
            h[..name.len()].copy_from_slice(name);
        }
        put_octal(&mut h[100..108], e.mode as u64, 7);
        let size = if e.is_dir { 0 } else { e.content.len() };
        put_octal(&mut h[124..136], size as u64, 11);
        put_octal(&mut h[136..148], e.mtime.max(0) as u64, 11);
        h[156] = if e.is_dir { b'5' } else { b'0' };
        h[257..262].copy_from_slice(b"ustar");
        h[263..265].copy_from_slice(b"00");
        h[148..156].copy_from_slice(b"        ");
        let sum: u64 = h.iter().map(|&b| b as u64).sum();
        put_octal(&mut h[148..156], sum, 7);
        out.extend_from_slice(&h);
        if size > 0 {
            out.extend_from_slice(&e.content);
            out.resize(out.len() + (size.div_ceil(512) * 512 - size), 0);
        }
    }
    out.resize(out.len() + 1024, 0);
    Ok(out)
}

/// Compress a finished tar for `Phar::GZ` / `Phar::BZ2`.
pub fn compress(tar: Vec<u8>, comp: i64) -> Vec<u8> {
    match comp {
        GZ => crate::ext::zlib::zraw::deflate(&tar, -1, 31, 9, false).unwrap_or_default(),
        BZ2 => crate::ext::bz2::compress(&tar, 9, 0),
        _ => tar,
    }
}

fn entries_to_value(entries: Vec<Entry>) -> Value {
    let mut list = PhpArray::new();
    for e in entries {
        let mut a = PhpArray::new();
        a.insert(ArrayKey::from_bytes(b"name"), Value::str(e.name));
        a.insert(ArrayKey::from_bytes(b"dir"), Value::Bool(e.is_dir));
        a.insert(ArrayKey::from_bytes(b"content"), Value::str(e.content));
        a.insert(ArrayKey::from_bytes(b"mtime"), Value::Int(e.mtime));
        a.insert(ArrayKey::from_bytes(b"mode"), Value::Int(e.mode as i64));
        list.push(Value::Array(a));
    }
    Value::Array(list)
}

fn arg_bytes(args: &[Value], i: usize) -> Vec<u8> {
    rhp_values::conv::to_string_bytes(&super::deref(args.get(i).unwrap_or(&Value::Null)))
}

/// `__phardata_load(string $file): array|false` — `['comp' => int,
/// 'entries' => [['name','dir','content','mtime','mode'], …]]`.
pub fn fn_load(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = arg_bytes(args, 0);
    let Ok(data) = std::fs::read(crate::vcwd::os_path(&path)) else {
        return Ok(Value::Bool(false));
    };
    let Some((comp, entries)) = read(&data) else {
        return Ok(Value::Bool(false));
    };
    let mut r = PhpArray::new();
    r.insert(ArrayKey::from_bytes(b"comp"), Value::Int(comp));
    r.insert(ArrayKey::from_bytes(b"entries"), entries_to_value(entries));
    Ok(Value::Array(r))
}

/// `__phardata_save(string $file, array $entries, int $comp): string|true` —
/// true, or the error text.
pub fn fn_save(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = arg_bytes(args, 0);
    let comp = args
        .get(2)
        .map(|v| rhp_values::conv::to_int(&super::deref(v)))
        .unwrap_or(0);
    let mut entries = Vec::new();
    if let Value::Array(list) = super::deref(args.get(1).unwrap_or(&Value::Null)) {
        for (_, e) in list.iter() {
            let Value::Array(e) = super::deref(e) else {
                continue;
            };
            let get = |k: &[u8]| {
                e.get(&ArrayKey::from_bytes(k))
                    .map(super::deref)
                    .unwrap_or(Value::Null)
            };
            entries.push(Entry {
                name: rhp_values::conv::to_string_bytes(&get(b"name")),
                is_dir: rhp_values::conv::to_bool(&get(b"dir")),
                content: rhp_values::conv::to_string_bytes(&get(b"content")),
                mtime: rhp_values::conv::to_int(&get(b"mtime")),
                mode: rhp_values::conv::to_int(&get(b"mode")) as u32,
            });
        }
    }
    let tar = match write(&entries) {
        Ok(t) => t,
        Err(name) => {
            return Ok(Value::str(format!(
                "tar-based phar \"{}\" cannot be created, filename \"{name}\" is too long for tar file format",
                String::from_utf8_lossy(&path)
            )))
        }
    };
    let data = compress(tar, comp);
    match crate::vcwd::write_file(&path, &data) {
        Ok(()) => Ok(Value::Bool(true)),
        Err(e) => Ok(Value::str(format!(
            "unable to open new phar \"{}\" for writing: {e}",
            String::from_utf8_lossy(&path)
        ))),
    }
}
