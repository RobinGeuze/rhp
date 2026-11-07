//! An OS file-descriptor-backed stream (php's plain-files `stdio` stream opened
//! from an fd): the parent ends of `proc_open` pipes. The VM stores one of
//! these per stream resource and delegates `fread`/`fwrite`/`fgets`/`feof`/
//! `stream_set_blocking`/`stream_select` to it.
//!
//! Plain local files opened by `fopen` use the same type (`new_plain`): reads
//! then loop until satisfied, and writes / seeks keep php's logical position.
//!
//! Read semantics follow `main/streams/streams.c` + `plain_wrapper.c`: reads go
//! through a read-ahead buffer filled by a single `read(2)` per request (no
//! greedy looping for non-plain-file streams), `feof` is only true once a
//! `read(2)` returned 0, and a non-blocking read with no data available
//! (`EAGAIN`) yields an empty result rather than EOF.

use std::cell::{Cell, RefCell};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use crate::ext::openssl::xp_ssl::SslSock;
use crate::main::streams::errno;

/// php's default stream chunk size.
const CHUNK: usize = 8192;

thread_local! {
    static DIAGS: RefCell<Vec<Diag>> = const { RefCell::new(Vec::new()) };
    /// Whether `DIAGS` may be non-empty: checked after every builtin call.
    static HAS_DIAGS: Cell<bool> = const { Cell::new(false) };
}

/// A diagnostic raised inside a stream operation that has no host (the TLS
/// layer's warnings): the code that ran the operation reports it.
#[derive(Clone, Debug)]
pub struct Diag {
    pub level: i64,
    /// `php_error_docref` (prefixed with the running function) rather than
    /// `php_error`.
    pub docref: bool,
    pub msg: String,
}

/// Queue a diagnostic (see [`Diag`]).
pub fn push_diag(level: i64, docref: bool, msg: impl Into<String>) {
    let msg = msg.into();
    DIAGS.with(|d| d.borrow_mut().push(Diag { level, docref, msg }));
    HAS_DIAGS.with(|f| f.set(true));
}

/// php_stdiop_read's notice for a failed `read(2)` of `count` bytes.
pub fn read_failed(count: usize, err: i32) {
    push_diag(
        crate::E_NOTICE,
        true,
        format!(
            "Read of {count} bytes failed with errno={err} {}",
            super::strerror(err)
        ),
    );
}

/// php_stdiop_write's notice for a failed `write(2)` of `count` bytes.
pub fn write_failed(count: usize, err: i32) {
    push_diag(
        crate::E_NOTICE,
        true,
        format!(
            "Write of {count} bytes failed with errno={err} {}",
            super::strerror(err)
        ),
    );
}

/// A new request: drop what an earlier one queued and never reported
/// (a stream closed while its engine was dropped).
pub(crate) fn reset_state() {
    take_diags();
}

/// Whether diagnostics are queued (cheaper than [`take_diags`]).
#[inline]
pub fn has_diags() -> bool {
    HAS_DIAGS.with(|f| f.get())
}

/// The queued diagnostics, oldest first.
pub fn take_diags() -> Vec<Diag> {
    HAS_DIAGS.with(|f| f.set(false));
    DIAGS.with(|d| std::mem::take(&mut *d.borrow_mut()))
}

pub struct FdStream {
    /// The line ending fgets stops at.
    eol: super::Eol,
    /// The TLS layer of a socket stream (php's openssl socket ops). Declared
    /// before `fd`: it shuts the session down before the socket closes.
    pub(crate) tls: Option<Box<SslSock>>,
    fd: Option<OwnedFd>,
    /// Set once a `read(2)` returned 0 (or a hard error).
    pub(crate) eof: bool,
    /// Read-ahead buffer (bytes read from the fd but not yet consumed).
    rbuf: Vec<u8>,
    /// php's `stream->position`: the logical offset (reads and writes add to
    /// it; `-1` to start with on an unseekable fd, as php does).
    position: i64,
    /// php's `is_seekable` (not a FIFO / character device, and `lseek` works).
    seekable: bool,
    /// A stream opened through the plain-files wrapper (`fopen` of a local
    /// path): `fread` loops until `$length` bytes or EOF (php's "greedy" read
    /// for `file://`), instead of returning after one fill.
    greedy: bool,
    /// Set for a socket stream (php's `php_netstream_data_t`): reads and
    /// writes then follow `xp_socket.c` (poll with the read timeout, `recv` /
    /// `send` with `MSG_DONTWAIT`).
    pub(crate) sock: Option<SockInfo>,
    /// The failed `send(2)` of the last write (socket streams).
    /// (boxed: a rare error, kept off the stream's size)
    last_send_error: Option<Box<SendError>>,
    /// php's "dechunk" read filter (an http response with
    /// `Transfer-Encoding: chunked`).
    dechunk: Option<Dechunk>,
    /// The stream's context has a notifier: reads record [`NotifyEvent`]s
    /// for the code that ran them to deliver.
    pub notify: bool,
    pub(crate) events: Vec<NotifyEvent>,
    /// An ftp:// file stream's control connection (php's `wrapperthis`),
    /// dropped after the data socket closes.
    pub(crate) ftp_control: Option<Box<crate::ext::standard::ftp_fopen_wrapper::Control>>,
}

/// A notification a stream operation owes its context's notifier.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NotifyEvent {
    /// A socket read got this many bytes (`php_stream_notify_progress_increment`).
    Progress(usize),
    /// The eof flag changed (`php_stream_notify_completed`).
    Completed,
    /// A TLS server session exceeded its renegotiation limit and the
    /// context has a `reneg_limit_callback` to ask.
    RenegLimit,
}

/// The socket half of a socket stream (php's `php_netstream_data_t` plus the
/// stream ops name).
#[derive(Clone)]
pub struct SockInfo {
    /// php's ops label (`stream_get_meta_data()['stream_type']`):
    /// `tcp_socket/ssl`, `udp_socket`, `unix_socket`, `udg_socket`,
    /// `generic_socket`.
    pub stream_type: &'static str,
    /// The read/write timeout; `None` is php's `tv_sec == -1` (wait forever).
    pub timeout: Option<(i64, i64)>,
    /// php's `timeout_event` (`timed_out` in the meta data).
    pub timed_out: bool,
    /// php's `is_blocked`.
    pub blocked: bool,
    /// A listening server socket (`PHP_STREAM_FLAG_NO_IO`).
    pub no_io: bool,
    /// The address the stream was opened with (php's `orig_path`, the meta
    /// data `uri`); none for accepted sockets and socket pairs.
    pub uri: Option<Vec<u8>>,
    /// Opened persistent (`STREAM_CLIENT_PERSISTENT`, `pfsockopen`): the
    /// resource is a `persistent stream`.
    pub persistent: bool,
    /// The peer closed the connection, as seen by a liveness check (`feof`).
    peer_closed: std::cell::Cell<bool>,
    /// The timeout the transport was created with (openssl's
    /// `connect_timeout`, the client handshake's limit).
    pub connect_timeout: (i64, i64),
    /// The wrapper that opened the stream (`http`), with its data (the
    /// response headers) and the mode it was opened with.
    pub wrapper: Option<&'static str>,
    pub wrapper_data: Option<rhp_values::Value>,
    pub mode: Option<Vec<u8>>,
}

impl SockInfo {
    pub fn new(stream_type: &'static str, timeout: Option<(i64, i64)>) -> SockInfo {
        SockInfo {
            stream_type,
            timeout,
            timed_out: false,
            blocked: true,
            no_io: false,
            uri: None,
            persistent: false,
            peer_closed: std::cell::Cell::new(false),
            connect_timeout: (0, 0),
            wrapper: None,
            wrapper_data: None,
            mode: None,
        }
    }
}

/// A failed `send(2)` on a socket stream: php's
/// "Send of N bytes failed with errno=E <strerror>" notice.
#[derive(Clone, Copy, Debug)]
pub struct SendError {
    pub count: usize,
    pub errno: i32,
}

/// `poll(2)` one fd for `events`, waiting at most `timeout` (`None` =
/// forever): `>0` ready, `0` timed out, `<0` error (errno set).
pub fn poll_fd(fd: RawFd, events: libc::c_short, timeout: Option<(i64, i64)>) -> i32 {
    let ms = match timeout {
        None => -1,
        // php's `php_tvtoto`; a negative total (e.g. stream_set_timeout($s,
        // -5)) reaches poll(2) as negative: wait forever.
        Some((s, us)) => s
            .saturating_mul(1000)
            .saturating_add(us / 1000)
            .clamp(-1, i32::MAX as i64) as i32,
    };
    let mut p = libc::pollfd {
        fd,
        events,
        revents: 0,
    };
    unsafe { libc::poll(&mut p, 1, ms) }
}

/// Outcome of one `read(2)` into the read-ahead buffer.
enum Fill {
    Data,
    /// EOF, or (non-blocking) no data available right now.
    Nothing,
    /// A hard read error (php's fill fails: `fread` returns false).
    Error,
}

/// php's `detect_is_seekable()` for an fd's `fstat`: FIFOs never seek, nor
/// do character devices, except on Linux the seekable memory devices (major
/// 1: /dev/mem, kmem, null, port, zero, full).
fn detect_is_seekable(st: &libc::stat) -> bool {
    match st.st_mode & libc::S_IFMT {
        libc::S_IFIFO => false,
        #[cfg(target_os = "linux")]
        libc::S_IFCHR => {
            libc::major(st.st_rdev) == 1 && matches!(libc::minor(st.st_rdev), 1..=5 | 7)
        }
        #[cfg(not(target_os = "linux"))]
        libc::S_IFCHR => false,
        _ => true,
    }
}

impl FdStream {
    pub fn new(fd: OwnedFd) -> Self {
        // php's detect_is_seekable(): FIFOs and character devices never seek
        // (on Linux, the memory devices /dev/null, /dev/zero, ... do);
        // otherwise the position starts at the fd's current offset.
        let raw = fd.as_raw_fd();
        let mut seekable = unsafe {
            let mut st: libc::stat = std::mem::zeroed();
            libc::fstat(raw, &mut st) == 0 && detect_is_seekable(&st)
        };
        let mut position = -1;
        if seekable {
            let off = unsafe { libc::lseek(raw, 0, libc::SEEK_CUR) };
            if off < 0 {
                seekable = false;
            } else {
                position = off as i64;
            }
        }
        FdStream {
            eol: super::Eol::for_new_stream(),
            tls: None,
            fd: Some(fd),
            eof: false,
            rbuf: Vec::new(),
            position,
            seekable,
            greedy: false,
            sock: None,
            last_send_error: None,
            dechunk: None,
            notify: false,
            events: Vec::new(),
            ftp_control: None,
        }
    }

    /// A socket stream (`stream_socket_server` / `_client` / `_accept` /
    /// `_pair`, `fsockopen`): never seekable, position 0.
    pub fn new_socket(fd: OwnedFd, info: SockInfo) -> Self {
        FdStream {
            eol: super::Eol::for_new_stream(),
            tls: None,
            fd: Some(fd),
            eof: false,
            rbuf: Vec::new(),
            position: 0,
            seekable: false,
            greedy: false,
            sock: Some(info),
            last_send_error: None,
            dechunk: None,
            notify: false,
            events: Vec::new(),
            ftp_control: None,
        }
    }

    /// The socket state of a socket stream.
    pub fn sock(&self) -> Option<&SockInfo> {
        self.sock.as_ref()
    }

    /// The failed send of the last `write`, if it failed (taken).
    pub fn take_send_error(&mut self) -> Option<SendError> {
        self.last_send_error.take().map(|e| *e)
    }

    /// `stream_set_timeout` on a socket stream (php's READ_TIMEOUT option):
    /// false for any other stream.
    pub fn set_timeout(&mut self, timeout: Option<(i64, i64)>) -> bool {
        match self.sock.as_mut() {
            Some(s) => {
                s.timeout = timeout;
                s.timed_out = false;
                true
            }
            None => false,
        }
    }

    /// A stream over a local file opened by `fopen` (plain-files wrapper).
    pub fn new_plain(fd: OwnedFd) -> Self {
        let mut s = FdStream::new(fd);
        s.greedy = true;
        s
    }

    /// The underlying fd number (for `select(2)`), if still open.
    pub fn raw_fd(&self) -> Option<RawFd> {
        self.fd.as_ref().map(|f| f.as_raw_fd())
    }

    /// A `dup(2)` of the fd (for handing to a child process).
    pub fn dup_fd(&self) -> Option<OwnedFd> {
        let raw = self.raw_fd()?;
        let d = unsafe { libc::dup(raw) };
        (d >= 0).then(|| unsafe { OwnedFd::from_raw_fd(d) })
    }

    /// Does the read-ahead buffer hold unread data? (`stream_select` treats such
    /// a stream as immediately readable, like php's emulated read set.)
    pub fn has_buffered(&self) -> bool {
        // php's cast for select first moves what the TLS layer holds
        // decrypted (SSL_pending) into the read buffer.
        !self.rbuf.is_empty() || self.tls.as_ref().is_some_and(|t| t.pending() > 0)
    }

    /// php's `is_seekable` (`stream_get_meta_data()['seekable']`).
    pub fn is_seekable(&self) -> bool {
        self.seekable
    }

    /// Hand over the read-ahead (php's read buffer when a filter is added).
    pub fn take_buffered(&mut self) -> Vec<u8> {
        let n = self.rbuf.len();
        self.take(n)
    }

    /// Bytes read ahead but not consumed (php's `unread_bytes`).
    pub fn unread(&self) -> usize {
        self.rbuf.len()
    }

    /// One `read(2)` of up to `max` bytes appended to the read-ahead buffer
    /// (php's `php_stdiop_read`: EINTR retried, EAGAIN is "no data", EBADF
    /// leaves the EOF flag alone, any other error sets it).
    fn fill(&mut self, max: usize) -> Fill {
        self.fill_ex(max, false)
    }

    /// [`FdStream::fill`]; `has_buffered` is php's `stream->has_buffered_data`
    /// (this read already took bytes from the buffer), which makes a blocking
    /// socket read not wait. A stream with the dechunk filter reads through
    /// it, until `want` (at most a chunk) decoded bytes are buffered.
    fn fill_ex(&mut self, want: usize, has_buffered: bool) -> Fill {
        let old_eof = self.eof;
        let r = if self.dechunk.is_none() {
            self.fill_raw(want, has_buffered)
        } else {
            match self.fill_filtered(want, has_buffered) {
                Some(r) => return r,
                None => Fill::Error,
            }
        };
        if self.notify && old_eof != self.eof {
            self.events.push(NotifyEvent::Completed);
        }
        r
    }

    /// _php_stream_fill_read_buffer with read filters; `None` when a read
    /// fails with nothing buffered, the one way it notifies completion.
    fn fill_filtered(&mut self, want: usize, has_buffered: bool) -> Option<Fill> {
        let to_read_now = want.min(CHUNK);
        let mut got = false;
        while !self.eof && self.rbuf.len() < to_read_now {
            let before = self.rbuf.len();
            let r = self.fill_raw(CHUNK, has_buffered);
            if matches!(r, Fill::Error) && self.rbuf.is_empty() {
                return None;
            }
            let raw: Vec<u8> = self.rbuf.drain(before..).collect();
            if raw.is_empty() {
                break;
            }
            let out = self.dechunk.as_mut().expect("dechunk").feed(raw);
            got |= !out.is_empty();
            self.rbuf.extend_from_slice(&out);
        }
        Some(if got { Fill::Data } else { Fill::Nothing })
    }

    /// One read of the underlying descriptor (the stream ops' `read`).
    fn fill_raw(&mut self, max: usize, has_buffered: bool) -> Fill {
        let Some(raw) = self.raw_fd() else {
            return Fill::Error;
        };
        if self.sock.is_some() {
            return self.sock_fill(raw, max, has_buffered);
        }
        let mut buf = vec![0u8; max.max(1)];
        loop {
            let n = unsafe { libc::read(raw, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n > 0 {
                self.rbuf.extend_from_slice(&buf[..n as usize]);
                return Fill::Data;
            }
            if n == 0 {
                self.eof = true;
                return Fill::Nothing;
            }
            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if err == libc::EINTR {
                continue;
            }
            if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                return Fill::Nothing;
            }
            read_failed(buf.len(), err);
            if err != libc::EBADF {
                self.eof = true;
            }
            return Fill::Error;
        }
    }

    /// php's `php_sockop_read`: a blocking socket waits (poll) up to its
    /// timeout for data unless this read already has some; a timeout fails
    /// the read (without EOF); then one `recv`, where EAGAIN is "no data",
    /// 0 is EOF and any other error sets EOF.
    fn sock_fill(&mut self, raw: RawFd, max: usize, has_buffered: bool) -> Fill {
        if self.tls_active() {
            // php_openssl_sockop_read
            let mut buf = vec![0u8; max.max(1)];
            let n = crate::ext::openssl::xp_ssl::sockop_read(self, &mut buf, has_buffered);
            return match n {
                n if n > 0 => {
                    self.rbuf.extend_from_slice(&buf[..n as usize]);
                    if self.notify {
                        self.events.push(NotifyEvent::Progress(n as usize));
                    }
                    Fill::Data
                }
                0 => Fill::Nothing,
                _ => Fill::Error,
            };
        }
        let sock = self.sock.as_mut().expect("socket stream");
        let mut flags = 0;
        if sock.blocked {
            let dont_wait = has_buffered || sock.timeout == Some((0, 0));
            if dont_wait || sock.timeout.is_some() {
                flags = libc::MSG_DONTWAIT;
            }
            if !dont_wait {
                sock.timed_out = false;
                loop {
                    let r = poll_fd(raw, libc::POLLIN | libc::POLLPRI, sock.timeout);
                    if r == 0 {
                        sock.timed_out = true;
                    }
                    if r >= 0 || errno() != libc::EINTR {
                        break;
                    }
                }
                if sock.timed_out {
                    return if has_buffered {
                        Fill::Nothing
                    } else {
                        Fill::Error
                    };
                }
            }
        }
        let mut buf = vec![0u8; max.max(1)];
        let n = unsafe { libc::recv(raw, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), flags) };
        if n > 0 {
            self.rbuf.extend_from_slice(&buf[..n as usize]);
            if self.notify {
                self.events.push(NotifyEvent::Progress(n as usize));
            }
            return Fill::Data;
        }
        if n == 0 {
            self.eof = true;
            return Fill::Nothing;
        }
        let err = errno();
        if err == libc::EAGAIN || err == libc::EWOULDBLOCK || err == libc::EINTR {
            return Fill::Nothing;
        }
        self.eof = true;
        Fill::Error
    }

    /// php's `PHP_STREAM_OPTION_CHECK_LIVENESS` with a zero timeout (what
    /// `feof` asks a socket): peek one byte without waiting; a clean
    /// shutdown (0) or a hard error means the peer is gone.
    fn sock_alive(&self) -> bool {
        let (Some(sock), Some(raw)) = (self.sock.as_ref(), self.raw_fd()) else {
            return true;
        };
        if sock.peer_closed.get() {
            return false;
        }
        if let Some(t) = self.tls.as_ref().filter(|t| t.active) {
            let alive = t.alive(raw, sock);
            if !alive {
                sock.peer_closed.set(true);
            }
            return alive;
        }
        if sock.no_io {
            // A listening socket: poll(0) for readability, then peek.
            if poll_fd(raw, libc::POLLIN | libc::POLLPRI, Some((0, 0))) <= 0 {
                return true;
            }
        }
        let mut b = 0u8;
        let r = unsafe {
            libc::recv(
                raw,
                &mut b as *mut u8 as *mut libc::c_void,
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        let alive = if r == 0 {
            false
        } else if r < 0 {
            let e = errno();
            e == libc::EWOULDBLOCK || e == libc::EAGAIN || e == libc::EMSGSIZE
        } else {
            true
        };
        if !alive {
            sock.peer_closed.set(true);
        }
        alive
    }

    fn take(&mut self, n: usize) -> Vec<u8> {
        let n = n.min(self.rbuf.len());
        self.position += n as i64;
        self.rbuf.drain(..n).collect()
    }

    /// Put bytes just read back in front of the read buffer (what php's
    /// `php_stream_get_record` leaves buffered when it returns nothing).
    pub fn push_back(&mut self, data: &[u8]) {
        self.rbuf.splice(0..0, data.iter().copied());
        self.position -= data.len() as i64;
    }

    /// `fread($h, $len)`: buffered bytes first, then reads of one chunk — a
    /// single fill for pipes / stdio (php returns after one fill for streams
    /// not opened through the plain-files wrapper), looping until `len` bytes
    /// or EOF for plain files. `None` (php: false) when the first read fails.
    pub fn read(&mut self, len: usize) -> Option<Vec<u8>> {
        if len == 0 {
            return Some(Vec::new());
        }
        let mut out = self.take(len);
        while out.len() < len {
            let had = !out.is_empty();
            let want = if self.dechunk.is_some() {
                len - out.len()
            } else {
                CHUNK
            };
            match self.fill_ex(want, had) {
                Fill::Data => {}
                Fill::Nothing => break,
                Fill::Error if out.is_empty() => return None,
                Fill::Error => break,
            }
            let want = len - out.len();
            out.extend(self.take(want));
            if !self.greedy {
                break;
            }
        }
        Some(out)
    }

    /// `stream_get_contents($h, $maxlen)`: read until EOF (or, non-blocking,
    /// until no more data is available right now); `maxlen < 0` = unlimited.
    pub fn read_all(&mut self, maxlen: i64) -> Vec<u8> {
        let limit = if maxlen < 0 {
            usize::MAX
        } else {
            maxlen as usize
        };
        let mut out = self.take(limit);
        while out.len() < limit && !self.eof {
            if !matches!(self.fill(CHUNK), Fill::Data) {
                break;
            }
            let want = limit - out.len();
            out.extend(self.take(want));
        }
        out
    }

    /// `fgets($h[, $length])`: up to and including the next newline, at most
    /// `maxlen - 1` bytes when a length is given. `None` at EOF with nothing
    /// read (or, non-blocking, when no data is available).
    pub fn gets(&mut self, maxlen: Option<usize>) -> Option<Vec<u8>> {
        let cap = maxlen.map(|m| m.saturating_sub(1));
        loop {
            if let Some(i) = self.eol.locate(&self.rbuf) {
                let n = cap.map(|c| c.min(i + 1)).unwrap_or(i + 1);
                return Some(self.take(n));
            }
            if let Some(c) = cap {
                if self.rbuf.len() >= c {
                    return Some(self.take(c));
                }
            }
            if self.eof || !matches!(self.fill(CHUNK), Fill::Data) {
                if self.rbuf.is_empty() {
                    return None;
                }
                let n = self.rbuf.len();
                return Some(self.take(cap.map(|c| c.min(n)).unwrap_or(n)));
            }
        }
    }

    /// `feof($h)`: false while buffered data remains; else the sticky EOF flag.
    pub fn eof(&self) -> bool {
        if !self.rbuf.is_empty() {
            return false;
        }
        // A socket is asked whether the peer is still there (php's
        // CHECK_LIVENESS in `_php_stream_eof`).
        self.eof || (self.sock.is_some() && !self.sock_alive())
    }

    /// `fwrite($h, $data)` (php's `_php_stream_write_buffer`): on a seekable
    /// fd, drop the read-ahead and move the fd to the logical position first;
    /// then `write(2)` until everything is written. `Some(0)` when a
    /// non-blocking pipe is full, `None` on a hard error before any byte.
    pub fn write(&mut self, data: &[u8]) -> Option<usize> {
        let raw = self.raw_fd()?;
        if data.is_empty() {
            return Some(0);
        }
        if self.tls_active() {
            return self.tls_write(data);
        }
        if self.sock.is_some() {
            return self.sock_write(raw, data);
        }
        if self.seekable && !self.rbuf.is_empty() {
            self.rbuf.clear();
            let r = unsafe { libc::lseek(raw, self.position as libc::off_t, libc::SEEK_SET) };
            if r >= 0 {
                self.position = r as i64;
            }
        }
        let mut done = 0usize;
        while done < data.len() {
            let rest = &data[done..];
            let n = unsafe { libc::write(raw, rest.as_ptr() as *const libc::c_void, rest.len()) };
            if n > 0 {
                done += n as usize;
                self.position += n as i64;
                continue;
            }
            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if n < 0 && err == libc::EINTR {
                continue;
            }
            if n == 0 || err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                return Some(done);
            }
            return if done == 0 { None } else { Some(done) };
        }
        Some(done)
    }

    /// `_php_stream_write_buffer` over `php_sockop_write`: `send` until all
    /// is written. A blocking socket waits (poll) up to its timeout when the
    /// send buffer is full; a non-blocking one returns the partial count. A
    /// failed send before any byte is `None` and leaves a [`SendError`] for
    /// the caller's notice.
    fn sock_write(&mut self, raw: RawFd, data: &[u8]) -> Option<usize> {
        let (blocked, timeout) = {
            let s = self.sock.as_ref().expect("socket stream");
            (s.blocked, s.timeout)
        };
        let mut done = 0usize;
        while done < data.len() {
            let rest = &data[done..];
            let flags = if blocked && timeout.is_some() {
                libc::MSG_DONTWAIT
            } else {
                0
            };
            let n = 'retry: loop {
                let n = unsafe {
                    libc::send(raw, rest.as_ptr() as *const libc::c_void, rest.len(), flags)
                };
                if n > 0 {
                    break n;
                }
                let mut err = errno();
                if err == libc::EINTR {
                    continue;
                }
                if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                    if !blocked {
                        // Not an error for a non-blocking stream: 0 written.
                        return Some(done);
                    }
                    let s = self.sock.as_mut().expect("socket stream");
                    s.timed_out = false;
                    loop {
                        let r = poll_fd(raw, libc::POLLOUT, timeout);
                        if r == 0 {
                            s.timed_out = true;
                            break;
                        }
                        if r > 0 {
                            continue 'retry; // writable now
                        }
                        err = errno();
                        if err != libc::EINTR {
                            break;
                        }
                    }
                }
                self.last_send_error = Some(Box::new(SendError {
                    count: rest.len(),
                    errno: err,
                }));
                return if done == 0 { None } else { Some(done) };
            };
            if self.notify {
                self.events.push(NotifyEvent::Progress(n as usize));
            }
            done += n as usize;
            self.position += n as i64;
        }
        Some(done)
    }

    /// `ftell($h)`: php's `stream->position` (may be `-1`: then false).
    pub fn tell(&self) -> i64 {
        self.position
    }

    /// `fseek($h, $offset, $whence)` (php's `_php_stream_seek`): a move inside
    /// the read-ahead buffer is served from it; a seekable fd is `lseek`ed
    /// (relative seeks resolved against the logical position) and the buffer
    /// dropped; an unseekable one emulates forward relative seeks by reading.
    pub fn seek(&mut self, offset: i64, whence: i32) -> bool {
        let avail = self.rbuf.len() as i64;
        match whence {
            libc::SEEK_CUR if offset > 0 && offset <= avail => {
                self.take(offset as usize);
                self.eof = false;
                return true;
            }
            libc::SEEK_SET if offset > self.position && offset <= self.position + avail => {
                self.take((offset - self.position) as usize);
                self.eof = false;
                return true;
            }
            _ => {}
        }
        if self.seekable {
            let Some(raw) = self.raw_fd() else {
                return false;
            };
            let (off, wh) = match whence {
                libc::SEEK_CUR => (self.position.saturating_add(offset), libc::SEEK_SET),
                libc::SEEK_SET if offset < 0 => return false,
                w => (offset, w),
            };
            let r = unsafe { libc::lseek(raw, off as libc::off_t, wh) };
            self.rbuf.clear();
            if r < 0 {
                return false;
            }
            self.position = r as i64;
            self.eof = false;
            return true;
        }
        if whence == libc::SEEK_CUR && offset >= 0 {
            let mut left = offset as usize;
            while left > 0 {
                match self.read(left.min(1024)) {
                    Some(v) if !v.is_empty() => left -= v.len(),
                    _ => return false,
                }
            }
            self.eof = false;
            return true;
        }
        false
    }

    /// php's `_php_stream_cast` buffer synchronization before handing the fd
    /// to someone else (e.g. a `proc_open` child): a seekable fd is moved back
    /// to the logical position and the read-ahead dropped.
    pub fn sync_for_cast(&mut self) {
        if !self.seekable {
            return;
        }
        if let Some(raw) = self.raw_fd() {
            unsafe { libc::lseek(raw, self.position as libc::off_t, libc::SEEK_SET) };
            self.rbuf.clear();
        }
    }

    /// `stream_set_blocking($h, $mode)`: toggle `O_NONBLOCK` on the fd.
    pub fn set_blocking(&mut self, blocking: bool) -> bool {
        let ok = match self.raw_fd() {
            Some(raw) => set_fd_blocking(raw, blocking),
            None => false,
        };
        if ok {
            if let Some(s) = self.sock.as_mut() {
                s.blocked = blocking;
            }
        }
        ok
    }

    /// php's `stream->eof` flag as `stream_get_meta_data` reports it (no
    /// liveness probe; a probe that saw the peer gone did set it).
    pub fn eof_flag(&self) -> bool {
        self.eof || self.sock.as_ref().is_some_and(|s| s.peer_closed.get())
    }

    /// Close the fd (`fclose`).
    pub fn close(&mut self) {
        self.tls = None;
        self.fd = None;
    }

    /// The TLS layer is set up and the handshake done (php's `ssl_active`).
    pub fn tls_active(&self) -> bool {
        self.tls.as_ref().is_some_and(|t| t.active)
    }

    /// `_php_stream_write_buffer` over `php_openssl_sockop_write`.
    fn tls_write(&mut self, data: &[u8]) -> Option<usize> {
        let mut done = 0usize;
        while done < data.len() {
            let n = crate::ext::openssl::xp_ssl::sockop_write(self, &data[done..]);
            if n <= 0 {
                return if done == 0 && n < 0 { None } else { Some(done) };
            }
            if self.notify {
                self.events.push(NotifyEvent::Progress(n as usize));
            }
            done += n as usize;
            self.position += n as i64;
        }
        Some(done)
    }

    /// The socket state of a socket stream, for changing it.
    pub fn sock_mut(&mut self) -> Option<&mut SockInfo> {
        self.sock.as_mut()
    }

    /// The `crypto` entry of `stream_get_meta_data` (an active TLS layer).
    pub fn crypto_meta(&self) -> Option<rhp_values::Value> {
        self.tls.as_ref().filter(|t| t.active).map(|t| t.meta())
    }

    /// The notifications owed since the last call.
    pub fn take_events(&mut self) -> Vec<NotifyEvent> {
        std::mem::take(&mut self.events)
    }

    /// The `reneg_limit_callback` said not to keep the connection:
    /// php's shutdown with eof.
    pub fn reneg_close(&mut self) {
        if let Some(raw) = self.raw_fd() {
            unsafe { libc::shutdown(raw, libc::SHUT_RDWR) };
        }
        self.eof = true;
    }

    /// Read the rest through php's "dechunk" filter.
    pub fn add_dechunk(&mut self) {
        let mut d = Dechunk::default();
        if !self.rbuf.is_empty() {
            let raw = std::mem::take(&mut self.rbuf);
            self.rbuf = d.feed(raw);
        }
        self.dechunk = Some(d);
    }

    /// Set php's `stream->position` (the http wrapper starts the body at 0).
    pub fn set_position(&mut self, pos: i64) {
        self.position = pos;
    }
}

/// php's "dechunk" filter state (ext/standard/filters.c `php_dechunk`).
#[derive(Default)]
pub struct Dechunk {
    chunk_size: usize,
    state: ChunkState,
}

#[derive(Default, Clone, Copy, PartialEq)]
enum ChunkState {
    #[default]
    SizeStart,
    Size,
    SizeExt,
    SizeCr,
    SizeLf,
    Body,
    BodyCr,
    BodyLf,
    Trailer,
    Error,
}

impl Dechunk {
    /// Decode one bucket: `php_dechunk`.
    pub fn feed(&mut self, buf: Vec<u8>) -> Vec<u8> {
        use ChunkState::*;
        let end = buf.len();
        let mut p = 0usize;
        let mut out = Vec::with_capacity(end);
        // `php_dechunk` falls through from one state to the next; `st` is
        // the case being run.
        let mut st = self.state;
        while p < end {
            match st {
                SizeStart | Size => {
                    if st == SizeStart {
                        self.chunk_size = 0;
                    }
                    let mut state = st;
                    while p < end {
                        let c = buf[p];
                        let digit = match c {
                            b'0'..=b'9' => (c - b'0') as usize,
                            b'A'..=b'F' => (c - b'A' + 10) as usize,
                            b'a'..=b'f' => (c - b'a' + 10) as usize,
                            _ if state == SizeStart => {
                                state = Error;
                                break;
                            }
                            _ => {
                                state = SizeExt;
                                break;
                            }
                        };
                        if self.chunk_size > usize::MAX / 16 {
                            state = Error;
                            break;
                        }
                        self.chunk_size = self.chunk_size * 16 + digit;
                        state = Size;
                        p += 1;
                    }
                    self.state = state;
                    if state == Error {
                        st = Error;
                        continue;
                    } else if p == end {
                        return out;
                    }
                    st = SizeExt;
                }
                SizeExt => {
                    while p < end && buf[p] != b'\r' && buf[p] != b'\n' {
                        p += 1;
                    }
                    if p == end {
                        return out;
                    }
                    st = SizeCr;
                }
                SizeCr => {
                    if buf[p] == b'\r' {
                        p += 1;
                        if p == end {
                            self.state = SizeLf;
                            return out;
                        }
                    }
                    st = SizeLf;
                }
                SizeLf => {
                    if buf[p] == b'\n' {
                        p += 1;
                        if self.chunk_size == 0 {
                            self.state = Trailer;
                            st = Trailer;
                            continue;
                        } else if p == end {
                            self.state = Body;
                            return out;
                        }
                    } else {
                        self.state = Error;
                        st = Error;
                        continue;
                    }
                    st = Body;
                }
                Body => {
                    if end - p >= self.chunk_size {
                        out.extend_from_slice(&buf[p..p + self.chunk_size]);
                        p += self.chunk_size;
                        if p == end {
                            self.state = BodyCr;
                            return out;
                        }
                    } else {
                        out.extend_from_slice(&buf[p..end]);
                        self.chunk_size -= end - p;
                        self.state = Body;
                        return out;
                    }
                    st = BodyCr;
                }
                BodyCr => {
                    if buf[p] == b'\r' {
                        p += 1;
                        if p == end {
                            self.state = BodyLf;
                            return out;
                        }
                    }
                    st = BodyLf;
                }
                BodyLf => {
                    if buf[p] == b'\n' {
                        p += 1;
                        self.state = SizeStart;
                        st = SizeStart;
                    } else {
                        self.state = Error;
                        st = Error;
                    }
                }
                Trailer => {
                    p = end;
                }
                Error => {
                    out.extend_from_slice(&buf[p..end]);
                    return out;
                }
            }
        }
        out
    }
}

/// Toggle `O_NONBLOCK` on an fd; true on success.
pub fn set_fd_blocking(raw: RawFd, blocking: bool) -> bool {
    unsafe {
        let flags = libc::fcntl(raw, libc::F_GETFL);
        if flags < 0 {
            return false;
        }
        let nf = if blocking {
            flags & !libc::O_NONBLOCK
        } else {
            flags | libc::O_NONBLOCK
        };
        nf == flags || libc::fcntl(raw, libc::F_SETFL, nf) == 0
    }
}

/// `php://fd/<n>`: a dup() of descriptor `n` (`spec` is what follows
/// `fd/`), or `None` with the wrapper's error noted.
pub fn open_php_fd(spec: &[u8]) -> Option<OwnedFd> {
    // ZEND_STRTOL: leading space, a sign, digits, and nothing after
    let t = spec.iter().take_while(|c| c.is_ascii_whitespace()).count();
    let mut i = t;
    if matches!(spec.get(i), Some(b'+' | b'-')) {
        i += 1;
    }
    let digits = spec[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || i + digits != spec.len() {
        super::note_wrapper_error(
            "php://fd/ stream must be specified in the form php://fd/<orig fd>",
        );
        return None;
    }
    let n: i64 = std::str::from_utf8(&spec[t..])
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(if spec[t] == b'-' { i64::MIN } else { i64::MAX });
    let dtablesize = unsafe { libc::getdtablesize() };
    if n < 0 || n >= i64::from(dtablesize) {
        super::note_wrapper_error(format!(
            "The file descriptors must be non-negative numbers smaller than {dtablesize}"
        ));
        return None;
    }
    let fd = unsafe { libc::dup(n as RawFd) };
    if fd < 0 {
        let e = super::errno();
        super::note_wrapper_error(format!(
            "Error duping file descriptor {n}; possibly it doesn't exist: [{e}]: {}",
            super::strerror(e)
        ));
        return None;
    }
    Some(unsafe { std::os::fd::FromRawFd::from_raw_fd(fd) })
}

/// A `dup(2)` of a raw fd (e.g. the process's own 0/1/2) as an owned fd.
pub fn dup_raw(raw: RawFd) -> Option<OwnedFd> {
    let d = unsafe { libc::dup(raw) };
    (d >= 0).then(|| unsafe { OwnedFd::from_raw_fd(d) })
}

/// Whether `fd` is open write-only (`Some(true)`), read-only / read-write
/// (`Some(false)`), or unknown (`None`): `fcntl(F_GETFL)`.
pub fn fd_access_mode(fd: RawFd) -> Option<bool> {
    let fl = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if fl < 0 {
        return None;
    }
    Some(fl & libc::O_ACCMODE == libc::O_WRONLY)
}

#[cfg(test)]
mod tests;
