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

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

/// php's default stream chunk size.
const CHUNK: usize = 8192;

pub struct FdStream {
    fd: Option<OwnedFd>,
    /// Set once a `read(2)` returned 0 (or a hard error).
    eof: bool,
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
}

/// Outcome of one `read(2)` into the read-ahead buffer.
enum Fill {
    Data,
    /// EOF, or (non-blocking) no data available right now.
    Nothing,
    /// A hard read error (php's fill fails: `fread` returns false).
    Error,
}

impl FdStream {
    pub fn new(fd: OwnedFd) -> Self {
        // php's detect_is_seekable(): FIFOs and character devices never seek;
        // otherwise the position starts at the fd's current offset.
        let raw = fd.as_raw_fd();
        let mut seekable = unsafe {
            let mut st: libc::stat = std::mem::zeroed();
            libc::fstat(raw, &mut st) == 0
                && (st.st_mode & libc::S_IFMT) != libc::S_IFIFO
                && (st.st_mode & libc::S_IFMT) != libc::S_IFCHR
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
            fd: Some(fd),
            eof: false,
            rbuf: Vec::new(),
            position,
            seekable,
            greedy: false,
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
        !self.rbuf.is_empty()
    }

    /// One `read(2)` of up to `max` bytes appended to the read-ahead buffer
    /// (php's `php_stdiop_read`: EINTR retried, EAGAIN is "no data", EBADF
    /// leaves the EOF flag alone, any other error sets it).
    fn fill(&mut self, max: usize) -> Fill {
        let Some(raw) = self.raw_fd() else {
            return Fill::Error;
        };
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
            if err != libc::EBADF {
                self.eof = true;
            }
            return Fill::Error;
        }
    }

    fn take(&mut self, n: usize) -> Vec<u8> {
        let n = n.min(self.rbuf.len());
        self.position += n as i64;
        self.rbuf.drain(..n).collect()
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
            match self.fill(CHUNK) {
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
            if let Some(i) = self.rbuf.iter().position(|&b| b == b'\n') {
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
        self.rbuf.is_empty() && self.eof
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

/// Open a local path the way php's plain-files wrapper does
/// (`php_stream_parse_fopen_modes` + `open(2)` with mode 0666): `r` read-only,
/// `w` create + truncate, `a` create + append, `x` exclusive create, `c`
/// create; `+` makes it read-write, `e` close-on-exec, `n` non-blocking. A
/// leading `file://` is stripped. `None` for an unknown mode or a failed open.
pub fn open_plain_file(path: &[u8], mode: &[u8]) -> Option<OwnedFd> {
    let mut flags = match mode.first()? {
        b'r' => 0,
        b'w' => libc::O_TRUNC | libc::O_CREAT,
        b'a' => libc::O_CREAT | libc::O_APPEND,
        b'x' => libc::O_CREAT | libc::O_EXCL,
        b'c' => libc::O_CREAT,
        _ => return None,
    };
    if mode.contains(&b'+') {
        flags |= libc::O_RDWR;
    } else if flags != 0 {
        flags |= libc::O_WRONLY;
    } else {
        flags |= libc::O_RDONLY;
    }
    if mode.contains(&b'e') {
        flags |= libc::O_CLOEXEC;
    }
    if mode.contains(&b'n') {
        flags |= libc::O_NONBLOCK;
    }
    let path = path.strip_prefix(b"file://").unwrap_or(path);
    let c = std::ffi::CString::new(path).ok()?;
    let fd = unsafe { libc::open(c.as_ptr(), flags, 0o666 as libc::c_uint) };
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
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

/// Whether `fd` is open write-only (`Some(true)`), read-only / read-write
/// (`Some(false)`), or unknown (`None`): `fcntl(F_GETFL)`.
pub fn fd_access_mode(fd: RawFd) -> Option<bool> {
    let fl = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if fl < 0 {
        return None;
    }
    Some(fl & libc::O_ACCMODE == libc::O_WRONLY)
}
