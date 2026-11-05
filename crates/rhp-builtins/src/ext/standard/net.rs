//! ext/standard/net.c: `net_get_interfaces()` over getifaddrs(3).

use rhp_values::{ArrayKey, PhpArray, Value};

use crate::{Host, Thrown};

/// php_inet_ntop: an IPv4/IPv6 address as text (a numeric host's `%scope`
/// suffix dropped), None for other families.
fn inet_ntop(sa: *const libc::sockaddr) -> Option<Vec<u8>> {
    if sa.is_null() {
        return None;
    }
    // SAFETY: getifaddrs gives sockaddrs of the size their family implies.
    unsafe {
        match (*sa).sa_family as libc::c_int {
            libc::AF_INET => {
                let a = &*(sa as *const libc::sockaddr_in);
                let ip = std::net::Ipv4Addr::from(u32::from_be(a.sin_addr.s_addr));
                Some(ip.to_string().into_bytes())
            }
            libc::AF_INET6 => {
                let a = &*(sa as *const libc::sockaddr_in6);
                let ip = std::net::Ipv6Addr::from(a.sin6_addr.s6_addr);
                Some(ip.to_string().into_bytes())
            }
            _ => None,
        }
    }
}

fn key(s: &str) -> ArrayKey {
    ArrayKey::from_bytes(s.as_bytes())
}

/// iface_append_unicast.
fn unicast_entry(ifa: &libc::ifaddrs) -> Value {
    let flags = ifa.ifa_flags as libc::c_int;
    let mut u = PhpArray::new();
    u.insert(key("flags"), Value::Int(flags as i64));
    if !ifa.ifa_addr.is_null() {
        // SAFETY: checked non-null; getifaddrs owns it until freeifaddrs.
        let fam = unsafe { (*ifa.ifa_addr).sa_family };
        u.insert(key("family"), Value::Int(fam as i64));
        if let Some(h) = inet_ntop(ifa.ifa_addr) {
            u.insert(key("address"), Value::str(h));
        }
    }
    if let Some(h) = inet_ntop(ifa.ifa_netmask) {
        u.insert(key("netmask"), Value::str(h));
    }
    #[cfg(target_os = "linux")]
    let dst = ifa.ifa_ifu;
    #[cfg(not(target_os = "linux"))]
    let dst = ifa.ifa_dstaddr;
    if flags & libc::IFF_BROADCAST != 0 {
        if let Some(h) = inet_ntop(dst) {
            u.insert(key("broadcast"), Value::str(h));
        }
    }
    if flags & libc::IFF_POINTOPOINT != 0 {
        if let Some(h) = inet_ntop(dst) {
            u.insert(key("ptp"), Value::str(h));
        }
    }
    Value::Array(u)
}

fn fn_net_get_interfaces(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut addrs: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `addrs` with a list freed below.
    if unsafe { libc::getifaddrs(&mut addrs) } != 0 {
        let e = std::io::Error::last_os_error();
        let msg = format!(
            "getifaddrs() failed {}: {}",
            e.raw_os_error().unwrap_or(0),
            crate::main::streams::strerror(e.raw_os_error().unwrap_or(0))
        );
        host.emit_diagnostic(crate::E_WARNING, msg.as_bytes())?;
        return Ok(Value::Bool(false));
    }
    let mut out = PhpArray::new();
    let mut p = addrs;
    while !p.is_null() {
        // SAFETY: a node of the list getifaddrs returned.
        let ifa = unsafe { &*p };
        // SAFETY: ifa_name is a NUL-terminated string.
        let name = unsafe { std::ffi::CStr::from_ptr(ifa.ifa_name) }.to_bytes();
        let k = ArrayKey::from_bytes(name);
        let mut iface = match out.get(&k) {
            Some(Value::Array(a)) => a.clone(),
            _ => PhpArray::new(),
        };
        let mut uni = match iface.get(&key("unicast")) {
            Some(Value::Array(a)) => a.clone(),
            _ => PhpArray::new(),
        };
        uni.push(unicast_entry(ifa));
        iface.insert(key("unicast"), Value::Array(uni));
        if iface.get(&key("up")).is_none() {
            let up = ifa.ifa_flags as libc::c_int & libc::IFF_UP != 0;
            iface.insert(key("up"), Value::Bool(up));
        }
        out.insert(k, Value::Array(iface));
        p = ifa.ifa_next;
    }
    // SAFETY: the list from getifaddrs, no longer used.
    unsafe { libc::freeifaddrs(addrs) };
    Ok(Value::Array(out))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("net_get_interfaces", fn_net_get_interfaces)]);
}
