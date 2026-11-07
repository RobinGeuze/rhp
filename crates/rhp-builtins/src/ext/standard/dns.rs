//! ext/standard/dns.c: `gethostname`, `gethostbyname`, `gethostbynamel`,
//! `gethostbyaddr`, the resolver functions (`dns_check_record`,
//! `dns_get_mx`, `dns_get_record` and their aliases) and the `DNS_*`
//! constants.

use std::sync::Mutex;

use rhp_values::{ArrayKey, PhpArray, Value};

use crate::zend::zpp::path_param;
use crate::{Host, Thrown};

/// `gethostbyaddr($ip)` (dns.c php_gethostbyaddr): inet_pton as AF_INET6, then
/// AF_INET, then getnameinfo(NI_NAMEREQD). The address itself when it has no
/// name; a warning and false when it is not an address.
fn fn_gethostbyaddr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ip = crate::zend::zpp::path_param(host, args, 0, "gethostbyaddr", "ip")?;
    let mut out = [0 as libc::c_char; 1025];
    let name = |sa: *const libc::sockaddr, len: usize, out: &mut [libc::c_char]| {
        // SAFETY: `sa` points at a zeroed sockaddr_in/_in6 of `len` bytes with
        // family and address set (as php fills it), and `out` is writable for
        // the length passed.
        let rc = unsafe {
            libc::getnameinfo(
                sa,
                len as libc::socklen_t,
                out.as_mut_ptr(),
                out.len() as libc::socklen_t,
                std::ptr::null_mut(),
                0,
                libc::NI_NAMEREQD,
            )
        };
        // SAFETY: getnameinfo NUL-terminated `out` on success.
        (rc == 0).then(|| {
            unsafe { std::ffi::CStr::from_ptr(out.as_ptr()) }
                .to_bytes()
                .to_vec()
        })
    };
    let found = if let Some(a) = crate::main::network::pton6(&ip) {
        // SAFETY: all-zero is a valid sockaddr_in6.
        let mut sa: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
        sa.sin6_family = libc::AF_INET6 as libc::sa_family_t;
        sa.sin6_addr.s6_addr = a;
        name(
            &sa as *const _ as *const libc::sockaddr,
            std::mem::size_of_val(&sa),
            &mut out,
        )
    } else if let Some(a) = crate::main::network::pton4(&ip) {
        // SAFETY: all-zero is a valid sockaddr_in.
        let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        sa.sin_family = libc::AF_INET as libc::sa_family_t;
        sa.sin_addr.s_addr = u32::from_ne_bytes(a);
        name(
            &sa as *const _ as *const libc::sockaddr,
            std::mem::size_of_val(&sa),
            &mut out,
        )
    } else {
        crate::warn(
            host,
            "gethostbyaddr",
            "Address is not a valid IPv4 or IPv6 address",
        )?;
        return Ok(Value::Bool(false));
    };
    Ok(Value::str(found.unwrap_or(ip)))
}

fn fn_gethostname(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut buf = [0u8; 256];
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc != 0 {
        return Ok(Value::Bool(false));
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    Ok(Value::str(&buf[..end]))
}

/// php's MAXFQDNLEN.
const MAXFQDNLEN: usize = 255;

/// gethostbyname(3) returns static storage: one caller at a time.
static NETDB: Mutex<()> = Mutex::new(());

extern "C" {
    fn gethostbyname(name: *const libc::c_char) -> *mut libc::hostent;
}

/// php_network_gethostbyname: the IPv4 addresses of `name`, None when it
/// does not resolve.
fn host_addrs(name: &[u8]) -> Option<Vec<std::net::Ipv4Addr>> {
    let c = std::ffi::CString::new(name).ok()?;
    let _g = NETDB.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: `c` is NUL-terminated; the result (static storage, guarded
    // by NETDB) is read before the lock is released.
    unsafe {
        let hp = gethostbyname(c.as_ptr());
        if hp.is_null() || (*hp).h_addrtype != libc::AF_INET {
            return None;
        }
        let mut out = Vec::new();
        let mut p = (*hp).h_addr_list;
        // (h_addr_list entries may be misaligned on macOS: read unaligned.)
        while !p.is_null() {
            let a = std::ptr::read_unaligned(p);
            if a.is_null() {
                break;
            }
            let b = std::ptr::read_unaligned(a as *const [u8; 4]);
            out.push(std::net::Ipv4Addr::from(b));
            p = p.add(1);
        }
        Some(out)
    }
}

/// `gethostbyname`: the first IPv4 address, or the input unchanged.
fn fn_gethostbyname(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = path_param(host, args, 0, "gethostbyname", "hostname")?;
    if name.len() > MAXFQDNLEN {
        too_long(host, "gethostbyname")?;
        return Ok(Value::str(name));
    }
    match host_addrs(&name).and_then(|a| a.first().copied()) {
        Some(a) => Ok(Value::str(a.to_string().into_bytes())),
        None => Ok(Value::str(name)),
    }
}

/// `gethostbynamel`: every IPv4 address, or false.
fn fn_gethostbynamel(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = path_param(host, args, 0, "gethostbynamel", "hostname")?;
    if name.len() > MAXFQDNLEN {
        too_long(host, "gethostbynamel")?;
        return Ok(Value::Bool(false));
    }
    let Some(addrs) = host_addrs(&name) else {
        return Ok(Value::Bool(false));
    };
    let mut a = PhpArray::new();
    for ip in addrs {
        a.push(Value::str(ip.to_string().into_bytes()));
    }
    Ok(Value::Array(a))
}

fn too_long(host: &mut dyn Host, func: &str) -> Result<(), Thrown> {
    crate::warn(
        host,
        func,
        &format!("Host name cannot be longer than {MAXFQDNLEN} characters"),
    )
}

/// dns.h's `DNS_*` record type flags.
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    let i = Value::Int;
    vec![
        ("DNS_A", i(1)),
        ("DNS_NS", i(2)),
        ("DNS_CNAME", i(16)),
        ("DNS_SOA", i(32)),
        ("DNS_PTR", i(2048)),
        ("DNS_HINFO", i(4096)),
        ("DNS_CAA", i(8192)),
        ("DNS_MX", i(16384)),
        ("DNS_TXT", i(32768)),
        ("DNS_A6", i(16777216)),
        ("DNS_SRV", i(33554432)),
        ("DNS_NAPTR", i(67108864)),
        ("DNS_AAAA", i(134217728)),
        ("DNS_ANY", i(268435456)),
        ("DNS_ALL", i(251721779)),
    ]
}

// --- the resolver ------------------------------------------------------------

/// The C library's res_search / dn_expand / dn_skipname, found at run time
/// (libresolv names them res_9_* on macOS and __res_* in older glibc).
type SearchFn = unsafe extern "C" fn(
    *const libc::c_char,
    libc::c_int,
    libc::c_int,
    *mut u8,
    libc::c_int,
) -> libc::c_int;
type ExpandFn = unsafe extern "C" fn(
    *const u8,
    *const u8,
    *const u8,
    *mut libc::c_char,
    libc::c_int,
) -> libc::c_int;

struct Resolver {
    search: SearchFn,
    expand: ExpandFn,
}

fn resolver() -> Option<&'static Resolver> {
    static R: std::sync::OnceLock<Option<Resolver>> = std::sync::OnceLock::new();
    R.get_or_init(|| {
        let libs: &[&std::ffi::CStr] = &[
            c"libresolv.9.dylib",
            c"libresolv.dylib",
            c"libresolv.so.2",
            c"libresolv.so",
        ];
        // SAFETY: dlopen/dlsym with NUL-terminated names; the handles are
        // never closed, so the symbols stay valid.
        unsafe {
            let mut handles: Vec<*mut libc::c_void> = libs
                .iter()
                .map(|l| libc::dlopen(l.as_ptr(), libc::RTLD_LAZY))
                .filter(|h| !h.is_null())
                .collect();
            handles.push(libc::RTLD_DEFAULT);
            let find = |names: &[&std::ffi::CStr]| {
                handles.iter().find_map(|&h| {
                    names
                        .iter()
                        .map(|n| libc::dlsym(h, n.as_ptr()))
                        .find(|p| !p.is_null())
                })
            };
            let search = find(&[c"res_9_search", c"res_search", c"__res_search"])?;
            let expand = find(&[c"res_9_dn_expand", c"dn_expand", c"__dn_expand"])?;
            Some(Resolver {
                search: std::mem::transmute::<*mut libc::c_void, SearchFn>(search),
                expand: std::mem::transmute::<*mut libc::c_void, ExpandFn>(expand),
            })
        }
    })
    .as_ref()
}

/// res_search state is process-global: one query at a time.
static RESOLVER: Mutex<()> = Mutex::new(());

/// php_dns_search with class C_IN: the answer packet, or None.
fn dns_search(name: &[u8], ty: u16) -> Option<Vec<u8>> {
    dns_search_errno(name, ty).ok()
}

/// [`dns_search`] with h_errno on failure (1 HOST_NOT_FOUND, 2 TRY_AGAIN,
/// 3 NO_RECOVERY, 4 NO_DATA).
fn dns_search_errno(name: &[u8], ty: u16) -> Result<Vec<u8>, i32> {
    let r = resolver().ok_or(3)?;
    let c = std::ffi::CString::new(name).map_err(|_| 1)?;
    let mut buf = vec![0u8; 65536];
    let _g = RESOLVER.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: `buf` is writable for the length passed; `c` NUL-terminated.
    let n = unsafe {
        (r.search)(
            c.as_ptr(),
            1,
            ty as libc::c_int,
            buf.as_mut_ptr(),
            buf.len() as libc::c_int,
        )
    };
    if n < 0 {
        return Err(h_errno());
    }
    buf.truncate((n as usize).min(buf.len()));
    Ok(buf)
}

/// The resolver's h_errno (a variable on macOS, a function in glibc).
fn h_errno() -> i32 {
    // SAFETY: dlsym lookups; the pointer read is the C library's int.
    unsafe {
        let f = libc::dlsym(libc::RTLD_DEFAULT, c"__h_errno_location".as_ptr());
        if !f.is_null() {
            let f: unsafe extern "C" fn() -> *mut libc::c_int = std::mem::transmute(f);
            return *f();
        }
        let v = libc::dlsym(libc::RTLD_DEFAULT, c"h_errno".as_ptr());
        if !v.is_null() {
            return *(v as *const libc::c_int);
        }
    }
    1
}

/// dn_expand: the name at `pos` and the bytes it takes.
fn dn_expand(msg: &[u8], pos: usize) -> Option<(Vec<u8>, usize)> {
    let r = resolver()?;
    if pos > msg.len() {
        return None;
    }
    let mut out = [0 as libc::c_char; 1024];
    // SAFETY: pointers into `msg` bounded by its end; `out` writable.
    let n = unsafe {
        (r.expand)(
            msg.as_ptr(),
            msg.as_ptr().add(msg.len()),
            msg.as_ptr().add(pos),
            out.as_mut_ptr(),
            (out.len() - 2) as libc::c_int,
        )
    };
    if n < 0 {
        return None;
    }
    // SAFETY: dn_expand NUL-terminated `out`.
    let name = unsafe { std::ffi::CStr::from_ptr(out.as_ptr()) }
        .to_bytes()
        .to_vec();
    Some((name, n as usize))
}

fn get16(m: &[u8], p: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*m.get(p)?, *m.get(p + 1)?]))
}

fn get32(m: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *m.get(p)?,
        *m.get(p + 1)?,
        *m.get(p + 2)?,
        *m.get(p + 3)?,
    ]))
}

const HFIXEDSZ: usize = 12;
const QFIXEDSZ: usize = 4;
const T_A: u16 = 1;
const T_NS: u16 = 2;
const T_CNAME: u16 = 5;
const T_SOA: u16 = 6;
const T_PTR: u16 = 12;
const T_HINFO: u16 = 13;
const T_MX: u16 = 15;
const T_TXT: u16 = 16;
const T_AAAA: u16 = 28;
const T_SRV: u16 = 33;
const T_NAPTR: u16 = 35;
const T_A6: u16 = 38;
const T_ANY: u16 = 255;
const T_CAA: u16 = 257;

/// The position after the question section, or None.
fn skip_questions(m: &[u8]) -> Option<usize> {
    let mut cp = HFIXEDSZ;
    for _ in 0..get16(m, 4)? {
        cp += dn_expand(m, cp)?.1 + QFIXEDSZ;
    }
    Some(cp)
}

/// `dns_check_record($hostname, $type = "MX")` (alias `checkdnsrr`).
fn check_record(host: &mut dyn Host, args: &[Value], func: &str) -> Result<Value, Thrown> {
    let name = path_param(host, args, 0, func, "hostname")?;
    if name.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            format!("{func}(): Argument #1 ($hostname) must not be empty"),
        ));
    }
    let ty = match args.get(1) {
        None => T_MX,
        Some(_) => {
            let t = crate::zend::zpp::str_param(host, args, 1, func, "type")?.to_ascii_uppercase();
            match t.as_slice() {
                b"A" => T_A,
                b"NS" => T_NS,
                b"MX" => T_MX,
                b"PTR" => T_PTR,
                b"ANY" => T_ANY,
                b"SOA" => T_SOA,
                b"CAA" => T_CAA,
                b"TXT" => T_TXT,
                b"CNAME" => T_CNAME,
                b"AAAA" => T_AAAA,
                b"SRV" => T_SRV,
                b"NAPTR" => T_NAPTR,
                b"A6" => T_A6,
                _ => {
                    return Err(Thrown::error(
                        "ValueError",
                        format!("{func}(): Argument #2 ($type) must be a valid DNS record type"),
                    ))
                }
            }
        }
    };
    Ok(Value::Bool(
        dns_search(&name, ty).is_some_and(|a| get16(&a, 6).unwrap_or(0) != 0),
    ))
}

fn fn_dns_check_record(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    check_record(host, args, "dns_check_record")
}

fn fn_checkdnsrr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    check_record(host, args, "checkdnsrr")
}

/// The MX records of an answer: (host, weight), or None when it is
/// malformed.
fn mx_records(m: &[u8]) -> Option<Vec<(Vec<u8>, i64)>> {
    let mut cp = skip_questions(m)?;
    let mut out = Vec::new();
    let mut count = get16(m, 6)?;
    while count > 0 && cp < m.len() {
        count -= 1;
        cp += dn_expand(m, cp)?.1;
        let ty = get16(m, cp)?;
        cp += 2 + 2 + 4;
        let len = get16(m, cp)? as usize;
        cp += 2;
        if ty != T_MX {
            cp += len;
            continue;
        }
        let weight = get16(m, cp)? as i64;
        cp += 2;
        let (name, n) = dn_expand(m, cp)?;
        cp += n;
        out.push((name, weight));
    }
    Some(out)
}

/// `dns_get_mx($hostname, &$hosts, &$weights = null)` (alias `getmxrr`).
fn get_mx(host: &mut dyn Host, args: &[Value], func: &str) -> Result<Value, Thrown> {
    let name = path_param(host, args, 0, func, "hostname")?;
    let set = |host: &mut dyn Host, i: usize, a: PhpArray| {
        if let Some(Value::Ref(r)) = args.get(i) {
            let _ = host;
            *r.borrow_mut() = Value::Array(a);
        }
    };
    set(host, 1, PhpArray::new());
    if args.len() > 2 {
        set(host, 2, PhpArray::new());
    }
    let Some(recs) = dns_search(&name, T_MX).and_then(|a| mx_records(&a)) else {
        return Ok(Value::Bool(false));
    };
    let (mut hosts, mut weights) = (PhpArray::new(), PhpArray::new());
    for (h, w) in recs {
        hosts.push(Value::str(h));
        weights.push(Value::Int(w));
    }
    let found = !hosts.is_empty();
    set(host, 1, hosts);
    if args.len() > 2 {
        set(host, 2, weights);
    }
    Ok(Value::Bool(found))
}

fn fn_dns_get_mx(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    get_mx(host, args, "dns_get_mx")
}

fn fn_getmxrr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    get_mx(host, args, "getmxrr")
}

/// php's DNS_* flags in dns_get_record's query order.
const QUERY_ORDER: [(i64, u16); 13] = [
    (1, T_A),
    (2, T_NS),
    (16, T_CNAME),
    (32, T_SOA),
    (2048, T_PTR),
    (4096, T_HINFO),
    (16384, T_MX),
    (32768, T_TXT),
    (134217728, T_AAAA),
    (33554432, T_SRV),
    (67108864, T_NAPTR),
    (16777216, T_A6),
    (8192, T_CAA),
];
const PHP_DNS_ANY: i64 = 268435456;
const PHP_DNS_ALL: i64 = 251721779;

/// The text of an AAAA / A6 address group sequence, php's way (its own
/// `::` compression, not inet_ntop's).
struct V6 {
    out: Vec<u8>,
    have_break: bool,
    in_break: bool,
}

impl V6 {
    fn group(&mut self, s: u16) {
        if s != 0 {
            if !self.out.is_empty() {
                self.in_break = false;
                self.out.push(b':');
            }
            self.out.extend_from_slice(format!("{s:x}").as_bytes());
        } else if !self.have_break {
            self.have_break = true;
            self.in_break = true;
            self.out.push(b':');
        } else if !self.in_break {
            self.out.extend_from_slice(b":0");
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.have_break && self.in_break {
            self.out.push(b':');
        }
        self.out
    }
}

/// php_parserr: one resource record at `cp`. Ok(next) with the record
/// (when stored and of a known type), Err(()) when the packet is bad.
#[allow(clippy::type_complexity)]
fn parse_rr(
    m: &[u8],
    cp: usize,
    want: u16,
    store: bool,
    raw: bool,
) -> Result<(usize, Option<PhpArray>), ()> {
    let k = |s: &str| ArrayKey::from_bytes(s.as_bytes());
    let (name, n) = dn_expand(m, cp).ok_or(())?;
    let mut cp = cp + n;
    if cp + 10 > m.len() {
        return Err(());
    }
    let ty = get16(m, cp).ok_or(())?;
    let ttl = get32(m, cp + 4).ok_or(())?;
    let dlen = get16(m, cp + 8).ok_or(())? as usize;
    cp += 10;
    if cp + dlen > m.len() || dlen == 0 {
        return Err(());
    }
    if (want != T_ANY && ty != want) || !store {
        return Ok((cp + dlen, None));
    }
    let mut a = PhpArray::new();
    a.insert(k("host"), Value::str(name));
    a.insert(k("class"), Value::str("IN"));
    a.insert(k("ttl"), Value::Int(ttl as i64));
    if raw {
        a.insert(k("type"), Value::Int(ty as i64));
        a.insert(k("data"), Value::str(&m[cp..cp + dlen]));
        return Ok((cp + dlen, Some(a)));
    }
    let need = |cp: usize, n: usize| if cp + n > m.len() { Err(()) } else { Ok(()) };
    let pstr = |cp: &mut usize| -> Result<Vec<u8>, ()> {
        need(*cp, 1)?;
        let n = m[*cp] as usize;
        *cp += 1;
        need(*cp, n)?;
        let s = m[*cp..*cp + n].to_vec();
        *cp += n;
        Ok(s)
    };
    let start = cp;
    match ty {
        T_A => {
            need(cp, 4)?;
            a.insert(k("type"), Value::str("A"));
            let ip = format!("{}.{}.{}.{}", m[cp], m[cp + 1], m[cp + 2], m[cp + 3]);
            a.insert(k("ip"), Value::str(ip.into_bytes()));
            cp += dlen;
        }
        T_MX | T_CNAME | T_NS | T_PTR => {
            if ty == T_MX {
                need(cp, 2)?;
                a.insert(k("type"), Value::str("MX"));
                a.insert(k("pri"), Value::Int(get16(m, cp).ok_or(())? as i64));
                cp += 2;
            }
            let t = match ty {
                T_CNAME => Some("CNAME"),
                T_NS => Some("NS"),
                T_PTR => Some("PTR"),
                _ => None,
            };
            if let Some(t) = t {
                a.insert(k("type"), Value::str(t));
            }
            let (target, n) = dn_expand(m, cp).ok_or(())?;
            cp += n;
            a.insert(k("target"), Value::str(target));
        }
        T_HINFO => {
            a.insert(k("type"), Value::str("HINFO"));
            let cpu = pstr(&mut cp)?;
            a.insert(k("cpu"), Value::str(cpu));
            let os = pstr(&mut cp)?;
            a.insert(k("os"), Value::str(os));
        }
        T_CAA => {
            a.insert(k("type"), Value::str("CAA"));
            need(cp, 1)?;
            a.insert(k("flags"), Value::Int(m[cp] as i64));
            cp += 1;
            let tag = pstr(&mut cp)?;
            let n = tag.len();
            a.insert(k("tag"), Value::str(tag));
            if dlen < n + 2 {
                return Err(());
            }
            let n = dlen - n - 2;
            need(cp, n)?;
            a.insert(k("value"), Value::str(&m[cp..cp + n]));
            cp += n;
        }
        T_TXT => {
            a.insert(k("type"), Value::str("TXT"));
            let d = &m[cp..cp + dlen];
            let (mut txt, mut entries) = (Vec::new(), PhpArray::new());
            let mut l1 = 0;
            while l1 < dlen {
                let mut n = d[l1] as usize;
                if l1 + n >= dlen {
                    n = dlen - (l1 + 1);
                }
                if n > 0 {
                    txt.extend_from_slice(&d[l1 + 1..l1 + 1 + n]);
                    entries.push(Value::str(&d[l1 + 1..l1 + 1 + n]));
                }
                l1 += n + 1;
            }
            cp += dlen;
            a.insert(k("txt"), Value::str(txt));
            a.insert(k("entries"), Value::Array(entries));
        }
        T_SOA => {
            a.insert(k("type"), Value::str("SOA"));
            for key in ["mname", "rname"] {
                let (n, l) = dn_expand(m, cp).ok_or(())?;
                cp += l;
                a.insert(k(key), Value::str(n));
            }
            need(cp, 20)?;
            for key in ["serial", "refresh", "retry", "expire", "minimum-ttl"] {
                a.insert(k(key), Value::Int(get32(m, cp).ok_or(())? as i64));
                cp += 4;
            }
        }
        T_AAAA => {
            need(cp, 16)?;
            let mut v6 = V6 {
                out: Vec::new(),
                have_break: false,
                in_break: false,
            };
            for _ in 0..8 {
                v6.group(get16(m, cp).ok_or(())?);
                cp += 2;
            }
            a.insert(k("type"), Value::str("AAAA"));
            a.insert(k("ipv6"), Value::str(v6.finish()));
        }
        T_A6 => {
            a.insert(k("type"), Value::str("A6"));
            need(cp, 1)?;
            let n = m[cp] as usize;
            cp += 1;
            a.insert(k("masklen"), Value::Int(n as i64));
            let mut v6 = V6 {
                out: Vec::new(),
                have_break: false,
                in_break: false,
            };
            if n > 15 {
                v6.have_break = true;
                v6.in_break = true;
                v6.out.push(b':');
            }
            if n % 16 > 8 {
                // A partial group (php prints it without advancing).
                need(cp, 1)?;
                let c = m[cp];
                if c != 0 {
                    // (php prints the byte without moving past it: what
                    // follows overwrites it.)
                    if !v6.out.is_empty() {
                        v6.in_break = false;
                        v6.out.push(b':');
                    }
                } else {
                    v6.group(0);
                }
                cp += 1;
            }
            for _ in (n + 8) / 16..8 {
                need(cp, 2)?;
                v6.group(get16(m, cp).ok_or(())?);
                cp += 2;
            }
            a.insert(k("ipv6"), Value::str(v6.finish()));
            if cp < start + dlen {
                let (chain, l) = dn_expand(m, cp).ok_or(())?;
                cp += l;
                a.insert(k("chain"), Value::str(chain));
            }
        }
        T_SRV => {
            need(cp, 6)?;
            a.insert(k("type"), Value::str("SRV"));
            for key in ["pri", "weight", "port"] {
                a.insert(k(key), Value::Int(get16(m, cp).ok_or(())? as i64));
                cp += 2;
            }
            let (target, n) = dn_expand(m, cp).ok_or(())?;
            cp += n;
            a.insert(k("target"), Value::str(target));
        }
        T_NAPTR => {
            need(cp, 4)?;
            a.insert(k("type"), Value::str("NAPTR"));
            for key in ["order", "pref"] {
                a.insert(k(key), Value::Int(get16(m, cp).ok_or(())? as i64));
                cp += 2;
            }
            for key in ["flags", "services", "regex"] {
                let s = pstr(&mut cp)?;
                a.insert(k(key), Value::str(s));
            }
            let (r, n) = dn_expand(m, cp).ok_or(())?;
            cp += n;
            a.insert(k("replacement"), Value::str(r));
        }
        _ => return Ok((cp + dlen, None)),
    }
    Ok((cp, Some(a)))
}

/// `dns_get_record($hostname, $type = DNS_ANY, &$authoritative_name_servers
/// = null, &$additional_records = null, $raw = false)`.
fn fn_dns_get_record(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    const F: &str = "dns_get_record";
    let name = path_param(host, args, 0, F, "hostname")?;
    let type_param = args.get(1).map_or(PHP_DNS_ANY, |v| {
        rhp_values::conv::to_int(&rhp_values::ops::deref(v))
    });
    let raw = args
        .get(4)
        .is_some_and(|v| rhp_values::conv::to_bool(&rhp_values::ops::deref(v)));
    let set = |i: usize, a: PhpArray| {
        if let Some(Value::Ref(r)) = args.get(i) {
            *r.borrow_mut() = Value::Array(a);
        }
    };
    let (want_auth, want_addtl) = (args.len() > 2, args.len() > 3);
    if want_auth {
        set(2, PhpArray::new());
    }
    if want_addtl {
        set(3, PhpArray::new());
    }
    if !raw {
        if type_param & !PHP_DNS_ALL != 0 && type_param != PHP_DNS_ANY {
            return Err(Thrown::error(
                "ValueError",
                format!("{F}(): Argument #2 ($type) must be a DNS_* constant"),
            ));
        }
    } else if !(1..=0xFFFF).contains(&type_param) {
        return Err(Thrown::error(
            "ValueError",
            format!("{F}(): Argument #2 ($type) must be between 1 and 65535 when argument #5 ($raw) is true"),
        ));
    }
    // php's loop: each requested type in order (or DNS_T_ANY / the raw
    // type), then, with $additional_records, one more ANY query whose
    // answers are not stored again.
    let mut queries: Vec<(u16, bool)> = Vec::new();
    if raw {
        queries.push((type_param as u16, true));
    } else if type_param == PHP_DNS_ANY {
        queries.push((T_ANY, true));
    } else {
        for (flag, t) in QUERY_ORDER {
            if type_param & flag != 0 {
                queries.push((t, true));
            }
        }
    }
    if want_addtl && type_param != PHP_DNS_ANY {
        queries.push((T_ANY, false));
    }
    let (mut out, mut auth, mut addtl) = (PhpArray::new(), PhpArray::new(), PhpArray::new());
    for (t, store) in queries {
        let m = match dns_search_errno(&name, t) {
            Ok(m) => m,
            // NO_DATA, HOST_NOT_FOUND: the next type.
            Err(4) | Err(1) => continue,
            Err(e) => {
                let msg = match e {
                    3 => "An unexpected server failure occurred.",
                    2 => "A temporary server error occurred.",
                    _ => "DNS Query failed",
                };
                crate::warn(host, F, msg)?;
                return Ok(Value::Bool(false));
            }
        };
        let Some(mut cp) = skip_questions(&m) else {
            crate::warn(host, F, "Unable to parse DNS data received")?;
            return Ok(Value::Bool(false));
        };
        let mut an = get16(&m, 6).unwrap_or(0);
        let mut ns = get16(&m, 8).unwrap_or(0);
        let mut ar = get16(&m, 10).unwrap_or(0);
        // php_parserr returning NULL stops every section.
        let mut alive = true;
        let mut step = |cp: &mut usize, want: u16, store: bool, dst: &mut PhpArray| {
            if !alive || *cp >= m.len() {
                return false;
            }
            match parse_rr(&m, *cp, want, store, raw) {
                Ok((n, rec)) => {
                    *cp = n;
                    if let Some(r) = rec {
                        dst.push(Value::Array(r));
                    }
                }
                Err(()) => alive = false,
            }
            true
        };
        while an > 0 {
            an -= 1;
            if !step(&mut cp, t, store, &mut out) {
                break;
            }
        }
        if want_auth || want_addtl {
            while ns > 0 {
                ns -= 1;
                if !step(&mut cp, T_ANY, want_auth, &mut auth) {
                    break;
                }
            }
        }
        if want_addtl {
            while ar > 0 {
                ar -= 1;
                if !step(&mut cp, T_ANY, true, &mut addtl) {
                    break;
                }
            }
        }
    }
    if want_auth {
        set(2, auth);
    }
    if want_addtl {
        set(3, addtl);
    }
    Ok(Value::Array(out))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("gethostbyaddr", fn_gethostbyaddr),
            ("gethostname", fn_gethostname),
            ("gethostbyname", fn_gethostbyname),
            ("gethostbynamel", fn_gethostbynamel),
            ("dns_check_record", fn_dns_check_record),
            ("checkdnsrr", fn_checkdnsrr),
            ("dns_get_mx", fn_dns_get_mx),
            ("getmxrr", fn_getmxrr),
            ("dns_get_record", fn_dns_get_record),
        ],
    );
}
