//! libc `inet_pton` / `inet_ntop`, which php calls for every textual IP
//! address. Results follow the platform, as php's do: macOS accepts leading
//! zeros and `%scope` suffixes and prints `::1.2.3.4`, glibc differs.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::os::fd::RawFd;

use crate::main::streams::errno;
use crate::main::streams::plain_wrapper::poll_fd;
use crate::main::streams::xp_socket::{unix_name, Tv, TIMEOUT_ERR};
use crate::{Host, Thrown};

// The libc crate doesn't bind these.
extern "C" {
    fn inet_pton(af: c_int, src: *const c_char, dst: *mut c_void) -> c_int;
    fn inet_ntop(
        af: c_int,
        src: *const c_void,
        dst: *mut c_char,
        size: libc::socklen_t,
    ) -> *const c_char;
}

/// `inet_pton(AF_INET, text)`: the address in network order. Text containing
/// a NUL byte fails (php callers cut or reject it first).
pub(crate) fn pton4(text: &[u8]) -> Option<[u8; 4]> {
    pton(libc::AF_INET, text)
}

/// `inet_pton(AF_INET6, text)`.
pub(crate) fn pton6(text: &[u8]) -> Option<[u8; 16]> {
    pton(libc::AF_INET6, text)
}

fn pton<const N: usize>(af: c_int, text: &[u8]) -> Option<[u8; N]> {
    let text = CString::new(text).ok()?;
    let mut out = [0u8; N];
    // SAFETY: `text` is NUL-terminated and `out` is the size of an in_addr
    // (N = 4) or in6_addr (N = 16), matching `af`.
    let rc = unsafe { inet_pton(af, text.as_ptr(), out.as_mut_ptr().cast()) };
    (rc == 1).then_some(out)
}

/// `inet_ntop` of a 4-byte (AF_INET) or 16-byte (AF_INET6) address.
pub(crate) fn ntop(addr: &[u8]) -> Option<Vec<u8>> {
    let af = match addr.len() {
        4 => libc::AF_INET,
        16 => libc::AF_INET6,
        _ => return None,
    };
    // INET6_ADDRSTRLEN.
    let mut buf = [0 as c_char; 46];
    // SAFETY: `addr` holds an address of `af`'s size and `buf` is writable
    // for the length passed.
    let p = unsafe {
        inet_ntop(
            af,
            addr.as_ptr().cast(),
            buf.as_mut_ptr(),
            buf.len() as libc::socklen_t,
        )
    };
    if p.is_null() {
        return None;
    }
    // SAFETY: inet_ntop NUL-terminated `buf` on success.
    Some(
        unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }
            .to_bytes()
            .to_vec(),
    )
}

// --- addresses ---------------------------------------------------------------

/// php's `parse_ip_address_ex`: `host:port` or `[v6]:port`.
pub(crate) fn parse_ip_address(s: &[u8]) -> Result<(Vec<u8>, i32), String> {
    if s.contains(&0) {
        return Err("The hostname must not contain null bytes".into());
    }
    let shown = String::from_utf8_lossy(s);
    if s.first() == Some(&b'[') && s.len() > 1 {
        let inner = &s[1..s.len() - 1];
        return match inner.iter().position(|&b| b == b']') {
            Some(p) if s.get(p + 2) == Some(&b':') => {
                Ok((s[1..p + 1].to_vec(), crate::c_atoi(&s[p + 3..])))
            }
            _ => Err(format!("Failed to parse IPv6 address \"{shown}\"")),
        };
    }
    let colon = if s.is_empty() {
        None
    } else {
        s[..s.len() - 1].iter().position(|&b| b == b':')
    };
    match colon {
        Some(c) => Ok((s[..c].to_vec(), crate::c_atoi(&s[c + 1..]))),
        None => Err(format!("Failed to parse address \"{shown}\"")),
    }
}

/// A resolved socket address.
pub(crate) struct Addr {
    pub(crate) storage: libc::sockaddr_storage,
    pub(crate) len: libc::socklen_t,
}

impl Addr {
    pub(crate) fn family(&self) -> i32 {
        self.storage.ss_family as i32
    }
    pub(crate) fn ptr(&self) -> *const libc::sockaddr {
        &self.storage as *const _ as *const libc::sockaddr
    }
    pub(crate) fn set_port(&mut self, port: u16) {
        unsafe {
            match self.family() {
                libc::AF_INET => {
                    let sin = &mut *(&mut self.storage as *mut _ as *mut libc::sockaddr_in);
                    sin.sin_port = port.to_be();
                    self.len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
                }
                libc::AF_INET6 => {
                    let sin6 = &mut *(&mut self.storage as *mut _ as *mut libc::sockaddr_in6);
                    sin6.sin6_port = port.to_be();
                    self.len = std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t;
                }
                _ => {}
            }
        }
    }
}

/// php's `php_network_getaddresses`: getaddrinfo (AF_UNSPEC when the IPv6
/// stack works) for `socktype`. A failure warns right away (and is the
/// error text).
pub(crate) fn getaddresses(
    host: &mut dyn Host,
    fname: &str,
    name: &[u8],
    socktype: i32,
) -> Result<Vec<Addr>, (String, Result<(), Thrown>)> {
    resolve(name, socktype).map_err(|msg| {
        let w = crate::warn(host, fname, &msg);
        (msg, w)
    })
}

/// [`getaddresses`] without the warning: the error text on failure.
fn resolve(name: &[u8], socktype: i32) -> Result<Vec<Addr>, String> {
    let cname = CString::new(name.to_vec()).unwrap_or_default();
    let v6_ok = unsafe {
        let s = libc::socket(libc::AF_INET6, libc::SOCK_DGRAM, 0);
        if s >= 0 {
            libc::close(s);
        }
        s >= 0
    };
    let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    hints.ai_family = if v6_ok {
        libc::AF_UNSPEC
    } else {
        libc::AF_INET
    };
    hints.ai_socktype = socktype;
    let mut res: *mut libc::addrinfo = std::ptr::null_mut();
    let n = unsafe { libc::getaddrinfo(cname.as_ptr(), std::ptr::null(), &hints, &mut res) };
    if n != 0 || res.is_null() {
        let msg = if n != 0 {
            let e = unsafe { CStr::from_ptr(libc::gai_strerror(n)) };
            format!(
                "php_network_getaddresses: getaddrinfo for {} failed: {}",
                String::from_utf8_lossy(name),
                e.to_string_lossy()
            )
        } else {
            format!(
                "php_network_getaddresses: getaddrinfo for {} failed (null result pointer) errno={}",
                String::from_utf8_lossy(name),
                errno()
            )
        };
        return Err(msg);
    }
    let mut out = Vec::new();
    let mut p = res;
    while !p.is_null() {
        unsafe {
            let ai = &*p;
            let mut storage: libc::sockaddr_storage = std::mem::zeroed();
            let len = (ai.ai_addrlen as usize).min(std::mem::size_of::<libc::sockaddr_storage>());
            std::ptr::copy_nonoverlapping(
                ai.ai_addr as *const u8,
                &mut storage as *mut _ as *mut u8,
                len,
            );
            out.push(Addr {
                storage,
                len: len as libc::socklen_t,
            });
            p = ai.ai_next;
        }
    }
    unsafe { libc::freeaddrinfo(res) };
    Ok(out)
}

/// php's `php_network_populate_name_from_sockaddr` text form.
pub(crate) fn sockaddr_text(sa: &libc::sockaddr_storage, sl: libc::socklen_t) -> Option<Vec<u8>> {
    unsafe {
        match sa.ss_family as i32 {
            libc::AF_INET => {
                let sin = &*(sa as *const _ as *const libc::sockaddr_in);
                let ip = ntop(&sin.sin_addr.s_addr.to_ne_bytes()).unwrap_or_default();
                let mut out = ip;
                out.extend_from_slice(format!(":{}", u16::from_be(sin.sin_port)).as_bytes());
                Some(out)
            }
            libc::AF_INET6 => {
                let sin6 = &*(sa as *const _ as *const libc::sockaddr_in6);
                let ip = ntop(&sin6.sin6_addr.s6_addr)?;
                let mut out = b"[".to_vec();
                out.extend_from_slice(&ip);
                out.extend_from_slice(format!("]:{}", u16::from_be(sin6.sin6_port)).as_bytes());
                Some(out)
            }
            libc::AF_UNIX => {
                let un = &*(sa as *const _ as *const libc::sockaddr_un);
                // c_char is i8 or u8 depending on the target: `as u8` is an
                // unnecessary cast (a clippy error) where it is already u8.
                let path: Vec<u8> = un.sun_path.iter().map(|c| c.to_ne_bytes()[0]).collect();
                // php: `sl - sizeof(sa_family_t)` bytes of an abstract name
                // (on BSDs that includes one byte past the reported length).
                let hdr = std::mem::size_of::<libc::sa_family_t>();
                if path.first() == Some(&0) {
                    let len = (sl as usize).saturating_sub(hdr).min(path.len());
                    Some(path[..len].to_vec())
                } else {
                    let end = path.iter().position(|&b| b == 0).unwrap_or(path.len());
                    Some(unix_name(&path[..end]))
                }
            }
            _ => None,
        }
    }
}

pub(crate) fn sock_name(fd: RawFd, peer: bool) -> Option<Vec<u8>> {
    let mut sa: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut sl = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    let r = unsafe {
        let p = &mut sa as *mut _ as *mut libc::sockaddr;
        if peer {
            libc::getpeername(fd, p, &mut sl)
        } else {
            libc::getsockname(fd, p, &mut sl)
        }
    };
    if r != 0 {
        return None;
    }
    sockaddr_text(&sa, sl)
}

pub(crate) fn now() -> std::time::Instant {
    std::time::Instant::now()
}

/// The instant a timeout `tv` from now ends (`None` when it is beyond what
/// the clock can represent: as good as forever).
pub(crate) fn deadline((s, us): (i64, i64)) -> Option<std::time::Instant> {
    let d = std::time::Duration::from_secs(s.max(0) as u64)
        + std::time::Duration::from_micros(us.max(0) as u64);
    now().checked_add(d)
}

/// php's `php_network_connect_socket`: a non-blocking `connect`, then (unless
/// asynchronous) a poll for completion within `timeout` and `SO_ERROR`; the
/// original blocking mode is restored for a synchronous connect.
/// `Ok(pending)` on success, `Err(errno)` on failure.
pub(crate) fn connect_socket(
    fd: RawFd,
    addr: &Addr,
    asynchronous: bool,
    timeout: Tv,
) -> Result<bool, i32> {
    let orig = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    unsafe { libc::fcntl(fd, libc::F_SETFL, orig | libc::O_NONBLOCK) };
    let n = unsafe { libc::connect(fd, addr.ptr(), addr.len) };
    let mut error = 0;
    if n != 0 {
        error = errno();
        if error != libc::EINPROGRESS {
            return Err(error);
        }
        if asynchronous {
            return Ok(true);
        }
        error = 0;
        let limit = timeout.and_then(deadline);
        let mut working = timeout;
        loop {
            let r = poll_fd(fd, libc::POLLIN | libc::POLLOUT, working);
            if r < 0 {
                if errno() == libc::EINTR {
                    if let Some(l) = limit {
                        let t = now();
                        if t >= l {
                            error = TIMEOUT_ERR;
                            break;
                        }
                        let left = l - t;
                        working = Some((left.as_secs() as i64, left.subsec_micros() as i64));
                    }
                    continue;
                }
            } else if r == 0 {
                error = TIMEOUT_ERR;
            } else {
                let mut len = std::mem::size_of::<i32>() as libc::socklen_t;
                unsafe {
                    libc::getsockopt(
                        fd,
                        libc::SOL_SOCKET,
                        libc::SO_ERROR,
                        &mut error as *mut i32 as *mut libc::c_void,
                        &mut len,
                    );
                }
            }
            break;
        }
    }
    if !asynchronous {
        unsafe { libc::fcntl(fd, libc::F_SETFL, orig) };
    }
    if error != 0 {
        return Err(error);
    }
    Ok(false)
}

/// php's `php_network_parse_network_address_with_port`: `[v6]:port` or
/// `host:port` (split at the first colon); a numeric address first, else the
/// first address the resolver gives (a resolver failure warns).
pub(crate) fn parse_network_address(
    host: &mut dyn Host,
    fname: &str,
    s: &[u8],
) -> Result<Option<Addr>, Thrown> {
    let (h, p): (&[u8], &[u8]) = if s.first() == Some(&b'[') {
        let Some(end) = s[1..].iter().position(|&b| b == b']').map(|e| e + 1) else {
            return Ok(None);
        };
        if s.get(end + 1) != Some(&b':') {
            return Ok(None);
        }
        (&s[1..end], &s[end + 2..])
    } else {
        let Some(c) = s.iter().position(|&b| b == b':') else {
            return Ok(None);
        };
        (&s[..c], &s[c + 1..])
    };
    // `estrndup` + C string calls: the host ends at its first NUL.
    let h = &h[..h.iter().position(|&b| b == 0).unwrap_or(h.len())];
    let port = crate::c_atoi(p) as u16;
    let text = String::from_utf8_lossy(h).into_owned();
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    if let Some(a) = pton6(h) {
        let sin6 = unsafe { &mut *(&mut storage as *mut _ as *mut libc::sockaddr_in6) };
        sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
        sin6.sin6_port = port.to_be();
        sin6.sin6_addr.s6_addr = a;
        return Ok(Some(Addr {
            storage,
            len: std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
        }));
    }
    if let Some(a) = pton4(h) {
        let sin = unsafe { &mut *(&mut storage as *mut _ as *mut libc::sockaddr_in) };
        sin.sin_family = libc::AF_INET as libc::sa_family_t;
        sin.sin_port = port.to_be();
        sin.sin_addr.s_addr = u32::from_ne_bytes(a);
        return Ok(Some(Addr {
            storage,
            len: std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        }));
    }
    match resolve(h, libc::SOCK_DGRAM) {
        Ok(addrs) => Ok(addrs
            .into_iter()
            .find(|a| matches!(a.family(), libc::AF_INET | libc::AF_INET6))
            .map(|mut a| {
                a.set_port(port);
                a
            })),
        Err(msg) => {
            // php_network_getaddresses warns itself, then the caller again.
            crate::warn(host, fname, &msg)?;
            crate::warn(host, fname, &format!("Failed to resolve `{text}': {msg}"))?;
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests;
