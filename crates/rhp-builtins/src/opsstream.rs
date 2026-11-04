//! A stream whose I/O an extension implements, as php's `php_stream_ops`,
//! under a port of php's generic stream layer (main/streams/streams.c): the
//! read buffer, the logical position, the eof flag, buffered seeks and
//! forward-seek emulation. The VM keeps one per such stream resource (zip
//! entries) and forwards fread/fgets/fseek/fstat/fclose to it.

use crate::{Host, StatInfo};

/// php's default chunk size.
const CHUNK: usize = 8192;

/// `php_stream_ops`, for a stream with no write side worth having.
pub trait StreamOps {
    /// `ops->label`: the meta data's `stream_type`.
    fn label(&self) -> &'static str;
    /// `ops->read`: fill `buf`, return the byte count or -1. Sets `*eof` as
    /// php's ops set `stream->eof`; diagnostics go to `diags` (level,
    /// message without the function prefix).
    fn read(&mut self, buf: &mut [u8], eof: &mut bool, diags: &mut Vec<(i64, String)>) -> isize;
    /// `ops->write`.
    fn write(&mut self, data: &[u8]) -> isize;
    /// Whether the ops have a `seek`.
    fn can_seek(&self) -> bool {
        false
    }
    /// `ops->seek` to an absolute offset (`SEEK_SET`) or relative to the
    /// end: 0 and the new offset, or -1 and the offset after the failure.
    fn seek(&mut self, _offset: i64, _whence: i32) -> (i32, i64) {
        (-1, -1)
    }
    /// `ops->stat`.
    fn stat(&mut self) -> Option<StatInfo> {
        None
    }
    /// `ops->close(stream, 1)`. It may reach PHP (a zip archive closing
    /// runs its callbacks).
    fn close(&mut self, host: &mut dyn Host);
}

/// A `php_stream` over [`StreamOps`].
pub struct OpsStream {
    ops: Box<dyn StreamOps>,
    readbuf: Vec<u8>,
    readpos: usize,
    writepos: usize,
    /// `stream->position`.
    position: i64,
    /// `stream->eof`.
    eof: bool,
    /// The mode it was opened with.
    pub mode: Vec<u8>,
    /// `stream->orig_path` (the meta data's `uri`).
    pub uri: Vec<u8>,
    /// `wrapper->wops->label` for a stream a wrapper opened; none for one a
    /// builtin made (`ZipArchive::getStream`).
    pub wrapper_label: Option<&'static str>,
    /// Diagnostics the last operation raised, for the VM to report under
    /// the running builtin.
    pub diags: Vec<(i64, String)>,
}

impl OpsStream {
    pub fn new(ops: Box<dyn StreamOps>, mode: &[u8], uri: Vec<u8>) -> OpsStream {
        OpsStream {
            ops,
            readbuf: Vec::new(),
            readpos: 0,
            writepos: 0,
            position: 0,
            eof: false,
            mode: mode.to_vec(),
            uri,
            wrapper_label: None,
            diags: Vec::new(),
        }
    }

    pub fn label(&self) -> &'static str {
        self.ops.label()
    }

    pub fn seekable(&self) -> bool {
        self.ops.can_seek()
    }

    /// `stream->eof` (the meta data's `eof`).
    pub fn eof_flag(&self) -> bool {
        self.eof
    }

    /// `stream->writepos - stream->readpos` (the meta data's `unread_bytes`).
    pub fn unread(&self) -> usize {
        self.writepos - self.readpos
    }

    /// `_php_stream_fill_read_buffer` without filters.
    fn fill(&mut self, size: usize) -> bool {
        if self.writepos - self.readpos < size {
            if self.readbuf.len() - self.writepos < CHUNK {
                if self.writepos > self.readpos {
                    self.readbuf.copy_within(self.readpos..self.writepos, 0);
                }
                self.writepos -= self.readpos;
                self.readpos = 0;
            }
            if self.readbuf.len() - self.writepos < CHUNK {
                let n = self.readbuf.len() + CHUNK;
                self.readbuf.resize(n, 0);
            }
            let wp = self.writepos;
            let got = self
                .ops
                .read(&mut self.readbuf[wp..], &mut self.eof, &mut self.diags);
            if got < 0 {
                return false;
            }
            self.writepos += got as usize;
        }
        true
    }

    /// `_php_stream_read`: at most `size` bytes, one fill (the stream is not
    /// a plain file or memory stream). `None` is a failed read with nothing
    /// read before it.
    pub fn read(&mut self, size: usize) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(size.min(CHUNK));
        let n = (self.writepos - self.readpos).min(size);
        out.extend_from_slice(&self.readbuf[self.readpos..self.readpos + n]);
        self.readpos += n;
        let size = size - n;
        if size > 0 {
            if self.fill(size) {
                let n = (self.writepos - self.readpos).min(size);
                out.extend_from_slice(&self.readbuf[self.readpos..self.readpos + n]);
                self.readpos += n;
            } else if out.is_empty() {
                return None;
            }
        }
        self.position += out.len() as i64;
        Some(out)
    }

    /// `_php_stream_copy_to_mem` with `PHP_STREAM_COPY_ALL`: read until a
    /// read returns nothing.
    pub fn read_all(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            match self.read(CHUNK) {
                Some(b) if !b.is_empty() => out.extend_from_slice(&b),
                _ => break,
            }
        }
        out
    }

    /// `_php_stream_get_line`: up to and including a newline, at most
    /// `max` bytes (`fgets`'s length minus one), else grow until a newline
    /// or the end. `None` when nothing was read.
    pub fn gets(&mut self, max: Option<usize>) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let mut left = max;
        if left == Some(0) {
            return None;
        }
        loop {
            let avail = self.writepos - self.readpos;
            if avail > 0 {
                let chunk = &self.readbuf[self.readpos..self.writepos];
                let (mut n, mut done) = match chunk.iter().position(|&b| b == b'\n') {
                    Some(i) => (i + 1, true),
                    None => (avail, false),
                };
                if let Some(l) = left {
                    if n >= l {
                        n = l;
                        done = true;
                    }
                }
                out.extend_from_slice(&chunk[..n]);
                self.position += n as i64;
                self.readpos += n;
                if let Some(l) = left.as_mut() {
                    *l -= n;
                }
                if done {
                    break;
                }
            } else if self.eof {
                break;
            } else {
                let toread = left.map_or(CHUNK, |l| l.min(CHUNK));
                self.fill(toread);
                if self.writepos == self.readpos {
                    break;
                }
            }
        }
        (!out.is_empty()).then_some(out)
    }

    /// `_php_stream_eof`: buffered data is never eof. The ops have no
    /// liveness check.
    pub fn eof(&self) -> bool {
        self.writepos == self.readpos && self.eof
    }

    /// `_php_stream_write` without filters.
    pub fn write(&mut self, data: &[u8]) -> Option<usize> {
        // A seekable stream's buffer is dropped (and the position kept) before
        // writing, as php does.
        if self.ops.can_seek() && self.readpos != self.writepos {
            self.readpos = 0;
            self.writepos = 0;
            let _ = self.ops.seek(self.position, libc::SEEK_SET);
        }
        let n = self.ops.write(data);
        if n < 0 {
            return None;
        }
        self.position += n as i64;
        Some(n as usize)
    }

    /// `_php_stream_seek`: 0 or -1. The "does not support seeking" warning
    /// is the caller's.
    pub fn seek(&mut self, offset: i64, whence: i32) -> i32 {
        let avail = (self.writepos - self.readpos) as i64;
        match whence {
            libc::SEEK_CUR if offset > 0 && offset <= avail => {
                self.readpos += offset as usize;
                self.position += offset;
                self.eof = false;
                return 0;
            }
            libc::SEEK_SET if offset > self.position && offset <= self.position + avail => {
                self.readpos += (offset - self.position) as usize;
                self.position = offset;
                self.eof = false;
                return 0;
            }
            _ => {}
        }
        if self.ops.can_seek() {
            let (offset, whence) = if whence == libc::SEEK_CUR {
                (self.position + offset, libc::SEEK_SET)
            } else {
                (offset, whence)
            };
            let (ret, pos) = self.ops.seek(offset, whence);
            self.position = pos;
            if ret == 0 {
                self.eof = false;
            }
            self.readpos = 0;
            self.writepos = 0;
            return ret;
        }
        // Forward relative seeks are emulated by reading.
        if whence == libc::SEEK_CUR && offset >= 0 {
            let mut left = offset;
            while left > 0 {
                match self.read(left.min(1024) as usize) {
                    Some(b) if !b.is_empty() => left -= b.len() as i64,
                    _ => return -1,
                }
            }
            self.eof = false;
            return 0;
        }
        -1
    }

    /// `_php_stream_tell`.
    pub fn tell(&self) -> i64 {
        self.position
    }

    /// `_php_stream_stat`.
    pub fn stat(&mut self) -> Option<StatInfo> {
        self.ops.stat()
    }

    /// `php_stream_free`'s `ops->close(stream, 1)`.
    pub fn close(&mut self, host: &mut dyn Host) {
        self.ops.close(host);
    }

    /// Take the diagnostics the last operation raised.
    pub fn take_diags(&mut self) -> Vec<(i64, String)> {
        std::mem::take(&mut self.diags)
    }
}
