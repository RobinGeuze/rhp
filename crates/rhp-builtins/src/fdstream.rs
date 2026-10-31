//! An OS file-descriptor-backed stream (php's plain-files `stdio` stream opened
//! from an fd): the parent ends of `proc_open` pipes. The VM stores one of
//! these per stream resource and delegates `fread`/`fwrite`/`fgets`/`feof`/
//! `stream_set_blocking`/`stream_select` to it.
//!
//! Read semantics follow `main/streams/streams.c` + `plain_wrapper.c`: reads go
//! through a read-ahead buffer filled by a single `read(2)` per request (no
//! greedy looping for non-plain-file streams), `feof` is only true once a
//! `read(2)` returned 0, and a non-blocking read with no data available
//! (`EAGAIN`) yields an empty result rather than EOF.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

/// php's default stream chunk size.
const CHUNK: usize = 8192;

pub struct FdStream {
    fd: Option<OwnedFd>,
    /// Set once a `read(2)` returned 0 (or a hard error).
    eof: bool,
    /// Read-ahead buffer (bytes read from the fd but not yet consumed).
    rbuf: Vec<u8>,
}

impl FdStream {
    pub fn new(fd: OwnedFd) -> Self {
        FdStream {
            fd: Some(fd),
            eof: false,
            rbuf: Vec::new(),
        }
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
        !self.rbuf.is_empty()
    }

    /// One `read(2)` of up to `max` bytes appended to the read-ahead buffer.
    /// Returns the number of bytes read (0 on EOF or on EAGAIN).
    fn fill(&mut self, max: usize) -> usize {
        let Some(raw) = self.raw_fd() else {
            self.eof = true;
            return 0;
        };
        let mut buf = vec![0u8; max.max(1)];
        loop {
            let n = unsafe { libc::read(raw, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n > 0 {
                self.rbuf.extend_from_slice(&buf[..n as usize]);
                return n as usize;
            }
            if n == 0 {
                self.eof = true;
                return 0;
            }
            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if err == libc::EINTR {
                continue;
            }
            if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                return 0;
            }
            if err != libc::EBADF {
                self.eof = true;
            }
            return 0;
        }
    }

    fn take(&mut self, n: usize) -> Vec<u8> {
        let n = n.min(self.rbuf.len());
        self.rbuf.drain(..n).collect()
    }

    /// `fread($h, $len)`: buffered bytes first, then at most one `read(2)`.
    pub fn read(&mut self, len: usize) -> Vec<u8> {
        if len == 0 {
            return Vec::new();
        }
        let mut out = self.take(len);
        if out.len() < len {
            let want = len - out.len();
            self.fill(want.max(CHUNK));
            out.extend(self.take(want));
        }
        out
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
            if self.fill(CHUNK) == 0 {
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
            if let Some(i) = self.rbuf.iter().position(|&b| b == b'\n') {
                let n = cap.map(|c| c.min(i + 1)).unwrap_or(i + 1);
                return Some(self.take(n));
            }
            if let Some(c) = cap {
                if self.rbuf.len() >= c {
                    return Some(self.take(c));
                }
            }
            if self.eof || self.fill(CHUNK) == 0 {
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
        self.rbuf.is_empty() && self.eof
    }

    /// `fwrite($h, $data)`: a single `write(2)`; `Some(0)` when a non-blocking
    /// pipe is full, `None` on a hard error.
    pub fn write(&mut self, data: &[u8]) -> Option<usize> {
        let raw = self.raw_fd()?;
        if data.is_empty() {
            return Some(0);
        }
        loop {
            let n = unsafe { libc::write(raw, data.as_ptr() as *const libc::c_void, data.len()) };
            if n >= 0 {
                return Some(n as usize);
            }
            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if err == libc::EINTR {
                continue;
            }
            if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                return Some(0);
            }
            return None;
        }
    }

    /// `stream_set_blocking($h, $mode)`: toggle `O_NONBLOCK` on the fd.
    pub fn set_blocking(&mut self, blocking: bool) -> bool {
        match self.raw_fd() {
            Some(raw) => set_fd_blocking(raw, blocking),
            None => false,
        }
    }

    /// Close the fd (`fclose`).
    pub fn close(&mut self) {
        self.fd = None;
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

/// A `dup(2)` of a raw fd (e.g. the process's own 0/1/2) as an owned fd.
pub fn dup_raw(raw: RawFd) -> Option<OwnedFd> {
    let d = unsafe { libc::dup(raw) };
    (d >= 0).then(|| unsafe { OwnedFd::from_raw_fd(d) })
}
