//! ext/standard/dns.c: `gethostname`, `gethostbyname` and `gethostbyaddr`.

use rhp_values::Value;

use crate::zend::zpp::str_param;
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
    let found = if let Some(a) = crate::inet::pton6(&ip) {
        // SAFETY: all-zero is a valid sockaddr_in6.
        let mut sa: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
        sa.sin6_family = libc::AF_INET6 as libc::sa_family_t;
        sa.sin6_addr.s6_addr = a;
        name(
            &sa as *const _ as *const libc::sockaddr,
            std::mem::size_of_val(&sa),
            &mut out,
        )
    } else if let Some(a) = crate::inet::pton4(&ip) {
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
    Ok(Value::str(buf[..end].to_vec()))
}

/// `gethostbyname`: the first IPv4 address, or the input unchanged on failure.
fn fn_gethostbyname(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    use std::net::ToSocketAddrs;
    let name = str_param(host, args, 0, "gethostbyname", "hostname")?;
    if name.len() > 255 {
        return Ok(Value::Bool(false));
    }
    let s = String::from_utf8_lossy(&name).into_owned();
    if let Ok(addrs) = (s.as_str(), 0u16).to_socket_addrs() {
        for a in addrs {
            if let std::net::SocketAddr::V4(v4) = a {
                return Ok(Value::str(v4.ip().to_string().into_bytes()));
            }
        }
    }
    Ok(Value::str(name))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("gethostbyaddr", fn_gethostbyaddr),
            ("gethostname", fn_gethostname),
            ("gethostbyname", fn_gethostbyname),
        ],
    );
}
