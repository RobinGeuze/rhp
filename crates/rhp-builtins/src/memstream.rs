//! php's memory stream (`main/streams/memory.c`): `php://memory`,
//! `php://temp`, and the in-memory view the VM keeps of a file opened through
//! a non-`file://` wrapper.
//!
//! Two offsets are tracked, as in php: `fpos` is the memory stream's own
//! cursor (writes in append mode jump it to the end; a seek past the end is
//! allowed and a later write zero-fills the gap) and `position` is the stream
//! layer's logical offset that `ftell` reports (reads and writes add to it; a
//! failed seek leaves it at `-1`). `eof` is only set when a read is attempted
//! at the end, and cleared by a successful seek.

/// php's `TEMP_STREAM_*` mode, from the `fopen` mode string
/// (`php_stream_mode_from_str`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemMode {
    ReadOnly,
    Default,
    Append,
}

impl MemMode {
    pub fn from_fopen_mode(mode: &[u8]) -> MemMode {
        if mode.contains(&b'a') {
            MemMode::Append
        } else if mode.iter().any(|&b| b == b'w' || b == b'+') {
            MemMode::Default
        } else {
            MemMode::ReadOnly
        }
    }
}

pub struct MemStream {
    pub data: Vec<u8>,
    fpos: usize,
    position: i64,
    eof: bool,
    mode: MemMode,
    /// Whether a write changed `data` (a wrapper-backed view is then written
    /// back on flush / close).
    pub dirty: bool,
}

impl MemStream {
    pub fn new(data: Vec<u8>, mode: MemMode) -> Self {
        MemStream {
            data,
            fpos: 0,
            position: 0,
            eof: false,
            mode,
            dirty: false,
        }
    }

    /// `php_stream_memory_read`: at most `count` bytes from `fpos`; at the end
    /// it sets EOF and returns nothing.
    fn op_read(&mut self, count: usize) -> &[u8] {
        if self.fpos >= self.data.len() {
            self.eof = true;
            return &[];
        }
        let n = count.min(self.data.len() - self.fpos);
        let start = self.fpos;
        self.fpos += n;
        &self.data[start..start + n]
    }

    /// `fread($h, $len)` / `stream_get_contents` (`len < 0` = all): the stream
    /// layer reads memory streams greedily, so EOF is set whenever fewer than
    /// `len` bytes were left.
    pub fn read(&mut self, len: i64) -> Vec<u8> {
        let want = if len < 0 { usize::MAX } else { len as usize };
        let mut out = Vec::new();
        while out.len() < want {
            let chunk = self.op_read(want - out.len()).to_vec();
            if chunk.is_empty() {
                break;
            }
            out.extend_from_slice(&chunk);
        }
        self.position += out.len() as i64;
        out
    }

    /// `fgets($h[, $length])`: through the next newline (at most `maxlen - 1`
    /// bytes). Running into the end without a newline sets EOF (php's line
    /// reader tries one more fill there).
    pub fn gets(&mut self, maxlen: Option<usize>) -> Option<Vec<u8>> {
        if let Some(m) = maxlen {
            if m <= 1 {
                return None;
            }
        }
        if self.fpos >= self.data.len() {
            self.eof = true;
            return None;
        }
        let start = self.fpos;
        let rest = &self.data[start..];
        let cap = maxlen.map(|m| m - 1).unwrap_or(usize::MAX);
        let (n, hit_end) = match rest.iter().position(|&b| b == b'\n') {
            Some(i) if i < cap => (i + 1, false),
            _ if rest.len() >= cap => (cap, false),
            _ => (rest.len(), true),
        };
        if hit_end {
            self.eof = true;
        }
        self.fpos += n;
        self.position += n as i64;
        Some(self.data[start..start + n].to_vec())
    }

    /// `fwrite`: `None` (php: false) on a read-only stream.
    pub fn write(&mut self, buf: &[u8]) -> Option<usize> {
        if buf.is_empty() {
            return Some(0);
        }
        if self.mode == MemMode::ReadOnly {
            return None;
        }
        if self.mode == MemMode::Append {
            self.fpos = self.data.len();
        }
        let end = self.fpos + buf.len();
        if end > self.data.len() {
            // Zero-fills any gap left by a seek past the end.
            self.data.resize(end, 0);
        }
        self.data[self.fpos..end].copy_from_slice(buf);
        self.fpos = end;
        self.position += buf.len() as i64;
        self.dirty = true;
        Some(buf.len())
    }

    pub fn eof(&self) -> bool {
        self.eof
    }

    pub fn tell(&self) -> i64 {
        self.position
    }

    /// `fseek` (`_php_stream_seek` + `php_stream_memory_seek`).
    pub fn seek(&mut self, offset: i64, whence: i32) -> bool {
        let (offset, whence) = match whence {
            libc::SEEK_CUR => (self.position.saturating_add(offset), libc::SEEK_SET),
            libc::SEEK_SET if offset < 0 => return false,
            w => (offset, w),
        };
        let len = self.data.len() as i64;
        let target = match whence {
            libc::SEEK_SET => offset,
            libc::SEEK_END => len.saturating_add(offset),
            _ => {
                self.position = self.fpos as i64;
                return false;
            }
        };
        if target < 0 {
            self.fpos = 0;
            self.position = -1;
            return false;
        }
        self.fpos = target as usize;
        self.position = target;
        self.eof = false;
        true
    }

    /// The whole buffer (for writing a wrapper-backed view back).
    pub fn contents(&self) -> &[u8] {
        &self.data
    }
}
