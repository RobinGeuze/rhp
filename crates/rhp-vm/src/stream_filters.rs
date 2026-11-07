//! Stream filter chains: php's read buffer and the filtered read / write
//! paths of main/streams/streams.c (`_php_stream_fill_read_buffer`,
//! `_php_stream_read`, `_php_stream_get_line`, `_php_stream_write_filtered`,
//! `_php_stream_flush`, `_php_stream_seek`) and filter.c's append / flush /
//! remove. A stream gets a [`FilterLayer`] when its first filter is added
//! and keeps it; streams without one take the unfiltered paths unchanged.

use rhp_builtins as builtins;
use rhp_builtins::main::streams::filter::{
    Brigade, Filter, Status, PSFS_FLAG_FLUSH_CLOSE, PSFS_FLAG_FLUSH_INC, PSFS_FLAG_NORMAL,
};
use rhp_values::{Thrown, Value};

use crate::streams::{res_id, StreamState};
use crate::Vm;

const SEEK_SET: i32 = 0;
const SEEK_CUR: i32 = 1;
const SEEK_END: i32 = 2;

/// php's `chunk_size`.
const CHUNK: usize = 8192;

/// A filter's `stream filter` resource: which stream it is on.
struct FilterRes {
    stream: u32,
}

/// The filters of a stream and the php state they work with.
#[derive(Default)]
pub(crate) struct FilterLayer {
    pub(crate) read: Vec<Filter>,
    pub(crate) write: Vec<Filter>,
    /// php's read buffer: `buf[pos..]` is unread.
    buf: Vec<u8>,
    pos: usize,
    /// php's `stream->eof`.
    eof: bool,
    /// php's `stream->position`.
    position: i64,
    /// php's `stream->fatal_error`: a read filter failed.
    fatal: bool,
}

impl FilterLayer {
    fn avail(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn take(&mut self, n: usize) -> Vec<u8> {
        let n = n.min(self.avail());
        let out = self.buf[self.pos..self.pos + n].to_vec();
        self.pos += n;
        if self.pos == self.buf.len() {
            self.buf.clear();
            self.pos = 0;
        }
        out
    }

    fn push_data(&mut self, data: &[u8]) {
        if self.pos > 0 {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
        self.buf.extend_from_slice(data);
    }

    fn clear_buf(&mut self) {
        self.buf.clear();
        self.pos = 0;
    }
}

impl Vm {
    fn layer(&self, id: u32) -> Option<&FilterLayer> {
        self.streams.record(id)?.filters.as_deref()
    }

    fn layer_mut(&mut self, id: u32) -> Option<&mut FilterLayer> {
        self.streams.record_mut(id)?.filters.as_deref_mut()
    }

    /// The stream's resource as a filter sees it: null once its last
    /// reference is gone (the stream is being destroyed).
    fn filter_stream_value(&self, id: u32) -> Value {
        if self.closing_resources {
            return Value::Null;
        }
        self.resource_list
            .get(&id)
            .and_then(|ws| ws.iter().find_map(|w| w.upgrade()))
            .map_or(Value::Null, Value::Resource)
    }

    /// The name php's docref messages use for the running function.
    pub(crate) fn docref_function(&self) -> String {
        if let Some(n) = self.native_calls.last() {
            return n.name.to_string();
        }
        if let Some(k) = self.include_docref {
            if self.pending_throw.is_none() {
                return k.to_string();
            }
        }
        let f = self.stack.len().saturating_sub(1);
        match self.stack.get(f) {
            Some(fr) if !fr.proto.name.as_bytes().is_empty() && f > 0 => {
                String::from_utf8_lossy(fr.proto.name.as_bytes()).into_owned()
            }
            _ => "main".to_string(),
        }
    }

    /// Run a stream's write (else read) chain from filter `start` on `inb`:
    /// php's loop that stops at the first filter not passing data on.
    /// `flags` go to the first filter and the rest. Returns its status and,
    /// on PSFS_PASS_ON, the chain's output.
    fn run_chain(
        &mut self,
        id: u32,
        write: bool,
        start: usize,
        inb: Brigade,
        (flags_first, flags_rest): (i32, i32),
        mut consumed: Option<&mut usize>,
    ) -> (Status, Brigade) {
        let stream = self.filter_stream_value(id);
        let Some(rec) = self.streams.record_mut(id) else {
            return (Status::ErrFatal, Brigade::new());
        };
        let Some(layer) = rec.filters.as_deref_mut() else {
            return (Status::ErrFatal, Brigade::new());
        };
        let mut chain = std::mem::take(if write {
            &mut layer.write
        } else {
            &mut layer.read
        });
        let was = std::mem::replace(&mut rec.no_fclose, true);
        let mut inb = inb;
        let mut status = Status::PassOn;
        for (i, f) in chain.iter_mut().enumerate().skip(start) {
            let outb = Brigade::new();
            let flags = if i == start { flags_first } else { flags_rest };
            let c = if i == start {
                consumed.as_deref_mut()
            } else {
                None
            };
            status = f.ops.filter(self, &stream, &inb, &outb, c, flags);
            if status != Status::PassOn {
                break;
            }
            inb = outb;
        }
        match self.streams.record_mut(id) {
            Some(rec) => {
                rec.no_fclose = was;
                if let Some(layer) = rec.filters.as_deref_mut() {
                    let slot = if write {
                        &mut layer.write
                    } else {
                        &mut layer.read
                    };
                    // Filters added by the callbacks go after the chain.
                    let added = std::mem::take(slot);
                    chain.extend(added);
                    *slot = chain;
                }
            }
            None => {
                for mut f in chain {
                    f.ops.dtor(self);
                }
            }
        }
        (status, inb)
    }

    /// php's `stream->eof` of the backing stream after a read.
    fn raw_eof(&self, id: u32) -> bool {
        match self.streams.get(&id) {
            Some(StreamState::Mem { eof, .. }) => *eof,
            Some(StreamState::File(f)) => f.eof(),
            Some(StreamState::Fd(f)) => f.eof_flag(),
            Some(StreamState::Ops(s)) => s.eof(),
            Some(StreamState::Stdin) => self.stdin_reader.as_ref().is_some_and(|f| f.eof_flag()),
            _ => true,
        }
    }

    /// The backing stream's read op: one read of up to `n` bytes, setting
    /// its eof as php's ops do. `None` when the read failed.
    fn raw_read(&mut self, id: u32, n: usize) -> Option<Vec<u8>> {
        let plain_memory = self
            .streams
            .record(id)
            .is_some_and(|r| r.data_url.is_none() && r.origin.is_none());
        match self.streams.get_mut(&id)? {
            StreamState::Mem {
                buf,
                pos,
                eof,
                temp,
                ..
            } => {
                let avail = buf.len().saturating_sub(*pos);
                if plain_memory && temp.is_none() {
                    // php_stream_memory_read: eof when reading at the end.
                    if avail == 0 {
                        *eof = true;
                        return Some(Vec::new());
                    }
                } else if avail <= n {
                    // a temp stream reads its memory stream to the end
                    if avail < n || avail == 0 {
                        *eof = true;
                    }
                }
                let take = avail.min(n);
                let out = buf[*pos..*pos + take].to_vec();
                *pos += take;
                Some(out)
            }
            StreamState::File(f) => f.read_once(n),
            StreamState::Fd(f) => f.read(n),
            StreamState::Ops(s) => {
                let r = s.read(n);
                self.raise_ops_stream_diags(id);
                r
            }
            StreamState::Stdin => self.stdin_stream()?.read(n),
            _ => Some(Vec::new()),
        }
    }

    /// `_php_stream_fill_read_buffer` with read filters.
    fn fill_filtered(&mut self, id: u32, size: usize) -> bool {
        let to_read_now = size.min(CHUNK);
        loop {
            let Some(l) = self.layer(id) else {
                return false;
            };
            if l.eof || l.avail() >= to_read_now {
                break;
            }
            let avail = l.avail();
            let just = self.raw_read(id, CHUNK);
            let raw_eof = self.raw_eof(id);
            let Some(l) = self.layer_mut(id) else {
                return false;
            };
            if raw_eof {
                l.eof = true;
            }
            let eof = l.eof;
            let inb = Brigade::new();
            let got = just.as_ref().map_or(0, |d| d.len());
            let flags = match just {
                None if avail == 0 => return false,
                Some(d) if !d.is_empty() => {
                    inb.append_data(d);
                    if eof {
                        PSFS_FLAG_FLUSH_CLOSE
                    } else {
                        PSFS_FLAG_NORMAL
                    }
                }
                _ if eof => PSFS_FLAG_FLUSH_CLOSE,
                _ => PSFS_FLAG_FLUSH_INC,
            };
            let (status, out) = self.run_chain(id, false, 0, inb, (flags, flags), None);
            match status {
                Status::PassOn => {
                    let Some(l) = self.layer_mut(id) else {
                        return false;
                    };
                    for b in out.take_all() {
                        l.push_data(&b.borrow().buf);
                    }
                }
                Status::FeedMe => {}
                Status::ErrFatal => {
                    if let Some(l) = self.layer_mut(id) {
                        l.eof = true;
                        l.fatal = true;
                    }
                    return false;
                }
            }
            if got == 0 {
                break;
            }
        }
        true
    }

    /// Whether `_php_stream_read` keeps reading after a short read (plain
    /// files, memory and temp streams).
    fn greedy(&self, id: u32) -> bool {
        let rec = self.streams.record(id);
        match self.streams.get(&id) {
            Some(StreamState::File(_)) => true,
            Some(StreamState::Mem { .. }) => {
                rec.is_some_and(|r| r.data_url.is_none() && r.origin.is_none())
            }
            _ => false,
        }
    }

    /// `_php_stream_read` on a stream with a filter layer.
    fn read_layered(&mut self, res: &Value, id: u32, size: usize) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let greedy = self.greedy(id);
        loop {
            let l = self.layer_mut(id)?;
            let more = l.take(size - out.len());
            out.extend_from_slice(&more);
            if out.len() == size {
                break;
            }
            let filtered = !self.layer(id)?.read.is_empty();
            let got = if !filtered {
                match self.stream_read_unfiltered(res, (size - out.len()) as i64) {
                    Some(d) => {
                        let n = d.len();
                        out.extend_from_slice(&d);
                        n
                    }
                    None if out.is_empty() => return None,
                    None => 0,
                }
            } else {
                if !self.fill_filtered(id, size - out.len()) {
                    if out.is_empty() {
                        return None;
                    }
                    break;
                }
                let l = self.layer_mut(id)?;
                let more = l.take(size - out.len());
                out.extend_from_slice(&more);
                more.len()
            };
            if got == 0 || !greedy {
                break;
            }
        }
        if let Some(l) = self.layer_mut(id) {
            l.position += out.len() as i64;
        }
        Some(out)
    }

    /// `_php_stream_copy_to_mem` (everything) on a layered stream.
    fn read_all_layered(&mut self, res: &Value, id: u32) -> Option<Vec<u8>> {
        let size = match self.streams.get(&id) {
            Some(StreamState::Mem { buf, .. }) => buf.len() as i64,
            Some(StreamState::File(f)) => f.stat().map_or(0, |s| s.size as i64),
            _ => 0,
        };
        let position = self.layer(id).map_or(0, |l| l.position);
        let mut buflen = if size > 0 {
            (size - position).max(0) as usize + CHUNK
        } else {
            CHUNK
        };
        let mut out = Vec::new();
        loop {
            match self.read_layered(res, id, buflen - out.len()) {
                Some(d) if !d.is_empty() => out.extend_from_slice(&d),
                // a failed first read is an error (include fails)
                None if out.is_empty() => return None,
                _ => break,
            }
            if out.len() + CHUNK / 4 >= buflen {
                buflen += CHUNK;
            }
        }
        Some(out)
    }

    pub(crate) fn stream_read_filtered(&mut self, res: &Value, len: i64) -> Option<Vec<u8>> {
        let id = res_id(res)?;
        if self.layer(id).is_none() {
            return self.stream_read_unfiltered(res, len);
        }
        if len < 0 {
            Some(self.read_all_layered(res, id).unwrap_or_default())
        } else {
            self.read_layered(res, id, len as usize)
        }
    }

    /// `_php_stream_get_line` on a layered stream (`max`: at most this many
    /// bytes).
    pub(crate) fn gets_layered(
        &mut self,
        res: &Value,
        id: u32,
        max: Option<usize>,
    ) -> Option<Vec<u8>> {
        let l = self.layer(id)?;
        if l.read.is_empty() && l.avail() == 0 {
            let r = self.stream_gets_unlayered(res, max);
            if let (Some(l), Some(d)) = (self.layer_mut(id), &r) {
                l.position += d.len() as i64;
            }
            return r;
        }
        let mut out = Vec::new();
        loop {
            let l = self.layer(id)?;
            let avail = l.avail();
            if avail > 0 {
                let room = max.map_or(usize::MAX, |m| m - out.len());
                let l = self.layer_mut(id)?;
                let window = &l.buf[l.pos..l.pos + avail.min(room)];
                let (n, done) = match window.iter().position(|&c| c == b'\n') {
                    Some(i) => (i + 1, true),
                    None => (window.len(), false),
                };
                let part = l.take(n);
                l.position += part.len() as i64;
                out.extend_from_slice(&part);
                if done || max.is_some_and(|m| out.len() >= m) {
                    break;
                }
            } else if l.eof {
                break;
            } else {
                if l.read.is_empty() {
                    let more = self.stream_gets_unlayered(res, max.map(|m| m - out.len()));
                    if let (Some(d), Some(l)) = (more, self.layer_mut(id)) {
                        l.position += d.len() as i64;
                        out.extend_from_slice(&d);
                    }
                    break;
                }
                let toread = max.map_or(CHUNK, |m| (m - out.len()).min(CHUNK));
                // a fatal filter error loses the line read so far
                if !self.fill_filtered(id, toread) && self.layer(id)?.fatal {
                    return None;
                }
                if self.layer(id)?.avail() == 0 {
                    break;
                }
            }
        }
        if out.is_empty() {
            return None;
        }
        Some(out)
    }

    /// Whether reads of stream `id` go through its layer (read filters, or
    /// data left in php's buffer).
    pub(crate) fn layer_reads(&self, id: u32) -> bool {
        self.layer(id)
            .is_some_and(|l| !l.read.is_empty() || l.avail() > 0)
    }

    /// `php_stream_get_record` on a layered stream: `None` is false.
    pub(crate) fn get_record_layered(
        &mut self,
        id: u32,
        maxlen: usize,
        delim: &[u8],
    ) -> Option<Vec<u8>> {
        if maxlen == 0 {
            return None;
        }
        let search = |l: &FilterLayer, skip: usize| -> Option<usize> {
            let seek = l.avail().min(maxlen);
            if delim.is_empty() || seek <= skip {
                return None;
            }
            let hay = &l.buf[l.pos + skip..l.pos + seek];
            hay.windows(delim.len())
                .position(|w| w == delim)
                .map(|i| i + skip)
        };
        let mut found = search(self.layer(id)?, 0);
        let mut buffered = self.layer(id)?.avail();
        while found.is_none() && buffered < maxlen {
            let to_read_now = (maxlen - buffered).min(CHUNK);
            if self.layer(id)?.read.is_empty() {
                break;
            }
            if !self.fill_filtered(id, buffered + to_read_now) && self.layer(id)?.fatal {
                return None;
            }
            let l = self.layer(id)?;
            let just = l.avail() - buffered;
            if just == 0 {
                break;
            }
            if !delim.is_empty() {
                found = search(l, buffered.saturating_sub(delim.len() - 1));
                if found.is_some() {
                    break;
                }
            }
            buffered += just;
        }
        let l = self.layer_mut(id)?;
        let avail = l.avail();
        let len = match found {
            Some(n) => n,
            None if delim.is_empty() && avail >= maxlen => maxlen,
            None if avail < maxlen && !l.eof => return None,
            None if avail == 0 && l.eof => return None,
            None => avail.min(maxlen),
        };
        let out = l.take(len);
        l.position += len as i64;
        if found.is_some() {
            l.take(delim.len());
            l.position += delim.len() as i64;
        }
        Some(out)
    }

    pub(crate) fn stream_eof_filtered(&self, res: &Value) -> bool {
        if let Some(l) = res_id(res).and_then(|id| self.layer(id)) {
            if l.avail() > 0 {
                return false;
            }
            if l.eof {
                return true;
            }
        }
        self.stream_eof_unfiltered(res)
    }

    /// `_php_stream_write_filtered`: the bytes the first filter consumed,
    /// `None` on a fatal filter error.
    fn write_filtered(
        &mut self,
        res: &Value,
        id: u32,
        data: Option<&[u8]>,
        flags: i32,
    ) -> Option<usize> {
        let inb = Brigade::new();
        if let Some(d) = data {
            inb.append_data(d.to_vec());
        }
        let mut consumed = 0usize;
        let (status, out) = self.run_chain(id, true, 0, inb, (flags, flags), Some(&mut consumed));
        match status {
            Status::PassOn => {
                let mut failed = false;
                for b in out.take_all() {
                    let data = std::mem::take(&mut b.borrow_mut().buf);
                    if self.raw_write_counted(res, id, &data).is_none() {
                        failed = true;
                    }
                }
                (!failed).then_some(consumed)
            }
            Status::FeedMe => Some(consumed),
            Status::ErrFatal => None,
        }
    }

    /// A write to the backing stream, counted in php's position.
    fn raw_write_counted(&mut self, res: &Value, id: u32, data: &[u8]) -> Option<usize> {
        if data.is_empty() {
            return Some(0);
        }
        let n = self.stream_write_unfiltered(res, data);
        if let (Some(n), Some(l)) = (n, self.layer_mut(id)) {
            l.position += n as i64;
        }
        n
    }

    pub(crate) fn stream_write_filtered(&mut self, res: &Value, data: &[u8]) -> Option<usize> {
        let id = res_id(res)?;
        // One lookup on the unfiltered path. A stream without a write op
        // (data:) reports that before any filter runs.
        let layered = match self.streams.record_mut(id) {
            Some(r) => {
                r.mem_buffered = None;
                r.filters.is_some() && r.data_url.is_none()
            }
            None => false,
        };
        if !layered {
            return self.stream_write_unfiltered(res, data);
        }
        let l = self.layer(id)?;
        if data.is_empty() {
            return Some(0);
        }
        // Writing at php's position: drop what was read ahead.
        if l.avail() > 0 {
            let pos = l.position;
            if let Some(l) = self.layer_mut(id) {
                l.clear_buf();
            }
            self.stream_seek_unfiltered(res, pos, SEEK_SET);
        }
        if self.layer(id).is_some_and(|l| !l.write.is_empty()) {
            self.write_filtered(res, id, Some(data), PSFS_FLAG_NORMAL)
        } else {
            self.raw_write_counted(res, id, data)
        }
    }

    /// `_php_stream_flush`'s filter part.
    fn flush_write_chain(&mut self, res: &Value, id: u32, closing: bool) {
        // A stream without a write op (data:) is not flushed.
        let no_write = self
            .streams
            .record(id)
            .is_some_and(|r| r.data_url.is_some());
        if !no_write && self.layer(id).is_some_and(|l| !l.write.is_empty()) {
            let flags = if closing {
                PSFS_FLAG_FLUSH_CLOSE
            } else {
                PSFS_FLAG_FLUSH_INC
            };
            self.write_filtered(res, id, None, flags);
        }
    }

    pub(crate) fn stream_flush_filtered(&mut self, res: &Value) -> bool {
        if let Some(id) = res_id(res) {
            self.flush_write_chain(res, id, false);
        }
        self.stream_flush_unfiltered(res)
    }

    pub(crate) fn stream_seek_filtered(&mut self, res: &Value, offset: i64, whence: i32) -> bool {
        let Some(id) = res_id(res) else { return false };
        if let Some(r) = self.streams.record_mut(id) {
            r.mem_buffered = None;
        }
        let Some(l) = self.layer_mut(id) else {
            return self.stream_seek_unfiltered(res, offset, whence);
        };
        // Within the read buffer.
        let avail = l.avail() as i64;
        let skip = match whence {
            SEEK_CUR if offset > 0 && offset <= avail => Some(offset),
            SEEK_SET if offset > l.position && offset <= l.position + avail => {
                Some(offset - l.position)
            }
            _ => None,
        };
        if let Some(n) = skip {
            l.pos += n as usize;
            l.position += n;
            l.eof = false;
            l.fatal = false;
            return true;
        }
        let position = l.position;
        self.flush_write_chain(res, id, false);
        let target = match whence {
            SEEK_CUR => position.saturating_add(offset),
            SEEK_SET if offset < 0 => return false,
            _ => offset,
        };
        let w = if whence == SEEK_END {
            SEEK_END
        } else {
            SEEK_SET
        };
        let ok = self.stream_seek_unfiltered(res, target, w);
        let tell = self.stream_tell_unfiltered(res);
        if let Some(l) = self.layer_mut(id) {
            l.clear_buf();
            if ok {
                l.eof = false;
                l.fatal = false;
                if let Some(t) = tell {
                    l.position = t;
                }
            }
        }
        ok
    }

    pub(crate) fn stream_tell_filtered(&mut self, res: &Value) -> Option<i64> {
        match res_id(res).and_then(|id| self.layer(id)) {
            Some(l) => Some(l.position),
            None => self.stream_tell_unfiltered(res),
        }
    }

    pub(crate) fn stream_rewind_filtered(&mut self, res: &Value) -> bool {
        let id = res_id(res);
        if let Some(r) = id.and_then(|id| self.streams.record_mut(id)) {
            r.mem_buffered = None;
        }
        if id.and_then(|id| self.layer(id)).is_some() {
            return self.stream_seek_filtered(res, 0, SEEK_SET);
        }
        self.stream_rewind_unfiltered(res)
    }

    pub(crate) fn stream_push_back_filtered(&mut self, res: &Value, data: &[u8]) -> bool {
        let Some(id) = res_id(res) else { return false };
        match self.layer_mut(id) {
            Some(l) => {
                let at = l.pos;
                l.buf.splice(at..at, data.iter().copied());
                l.position -= data.len() as i64;
                true
            }
            None => self.stream_push_back_unfiltered(res, data),
        }
    }

    /// Give stream `id` its filter layer: php's read buffer takes over what
    /// the backing stream read ahead.
    fn ensure_layer(&mut self, res: &Value, id: u32) {
        if self.layer(id).is_some() {
            return;
        }
        let position = self.stream_tell_unfiltered(res).unwrap_or(0);
        let eof = self.raw_eof(id);
        let mem_end = self
            .streams
            .record_mut(id)
            .and_then(|r| r.mem_buffered.take());
        let buf = match self.streams.get_mut(&id) {
            Some(StreamState::File(f)) => f.take_buffered(),
            Some(StreamState::Fd(f)) => f.take_buffered(),
            Some(StreamState::Mem { buf, pos, .. }) => match mem_end {
                Some(end) if end > *pos && end <= buf.len() => {
                    let b = buf[*pos..end].to_vec();
                    *pos = end;
                    b
                }
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        self.filtered_streams += 1;
        if let Some(r) = self.streams.record_mut(id) {
            r.filters = Some(Box::new(FilterLayer {
                buf,
                eof,
                position,
                ..FilterLayer::default()
            }));
        }
    }

    /// php_stream_filter_append_ex's winding of buffered data through a
    /// filter just appended to the read chain. False on a fatal error.
    fn wind_prebuffered(&mut self, id: u32) -> bool {
        let Some(l) = self.layer_mut(id) else {
            return true;
        };
        if l.avail() == 0 {
            return true;
        }
        let data = l.buf[l.pos..].to_vec();
        let idx = l.read.len() - 1;
        let inb = Brigade::new();
        inb.append_data(data.clone());
        let mut consumed = 0usize;
        let (mut status, out) = self.run_chain_one(id, idx, inb, &mut consumed);
        if consumed > data.len() {
            status = Status::ErrFatal;
        }
        match status {
            Status::ErrFatal => {
                let msg = format!(
                    "{}(): Filter failed to process pre-buffered data",
                    self.docref_function()
                );
                builtins::main::streams::filter::raise(self, builtins::E_WARNING, msg.as_bytes());
                false
            }
            Status::FeedMe => {
                if let Some(l) = self.layer_mut(id) {
                    l.clear_buf();
                }
                true
            }
            Status::PassOn => {
                if let Some(l) = self.layer_mut(id) {
                    l.clear_buf();
                    for b in out.take_all() {
                        l.push_data(&b.borrow().buf);
                    }
                }
                true
            }
        }
    }

    /// Run only read filter `idx` (with consumed tracking).
    fn run_chain_one(
        &mut self,
        id: u32,
        idx: usize,
        inb: Brigade,
        consumed: &mut usize,
    ) -> (Status, Brigade) {
        // Hide the filters after `idx` while it runs.
        let rest = match self.layer_mut(id) {
            Some(l) => l.read.split_off(idx + 1),
            None => return (Status::ErrFatal, Brigade::new()),
        };
        let r = self.run_chain(
            id,
            false,
            idx,
            inb,
            (PSFS_FLAG_NORMAL, PSFS_FLAG_NORMAL),
            Some(consumed),
        );
        if let Some(l) = self.layer_mut(id) {
            l.read.extend(rest);
        }
        r
    }

    /// `_php_stream_filter_flush(filter, finish)`.
    fn filter_flush(&mut self, res: &Value, id: u32, write: bool, idx: usize) -> bool {
        let flags = PSFS_FLAG_FLUSH_CLOSE;
        let (status, out) = self.run_chain(
            id,
            write,
            idx,
            Brigade::new(),
            (flags, PSFS_FLAG_NORMAL),
            None,
        );
        match status {
            Status::FeedMe => true,
            Status::ErrFatal => false,
            Status::PassOn => {
                for b in out.take_all() {
                    let data = std::mem::take(&mut b.borrow_mut().buf);
                    if write {
                        self.raw_write_counted(res, id, &data);
                    } else if let Some(l) = self.layer_mut(id) {
                        l.push_data(&data);
                    }
                }
                true
            }
        }
    }

    /// Free a filter taken off its chain: its resource closes, then its dtor.
    fn free_filter(&mut self, mut f: Filter) {
        if let Some(rid) = f.res.take() {
            self.native_resources.remove(&rid);
        }
        f.ops.dtor(self);
    }

    /// Flush the write chain before a stream closes (php_stream_free).
    pub(crate) fn close_flush_filters(&mut self, id: u32) {
        if self.layer(id).is_some_and(|l| !l.write.is_empty()) {
            let res = self.filter_stream_value(id);
            // A stream being destroyed has no resource left: the writes
            // only need its id (a type name nothing tracks).
            let res = if matches!(res, Value::Null) {
                Value::Resource(std::rc::Rc::new(rhp_values::ResourceData {
                    id,
                    type_name: "stream-filtered",
                }))
            } else {
                res
            };
            self.flush_write_chain(&res, id, true);
        }
    }

    /// The filters of a closed stream: read chain then write chain.
    pub(crate) fn free_filters(&mut self, layer: Option<Box<FilterLayer>>) {
        let Some(mut layer) = layer else { return };
        self.filtered_streams = self.filtered_streams.saturating_sub(1);
        let read = std::mem::take(&mut layer.read);
        let write = std::mem::take(&mut layer.write);
        for f in read.into_iter().chain(write) {
            self.free_filter(f);
        }
    }

    /// Free the streams that still have filters at the end of the request
    /// (zend_close_rsrc_list, last first): their filters see a null stream.
    pub(crate) fn close_filtered_streams(&mut self) -> Result<(), Thrown> {
        let mut ids: Vec<u32> = self
            .streams
            .ids()
            .into_iter()
            .filter(|id| self.layer(*id).is_some())
            .collect();
        ids.sort_unstable();
        self.closing_resources = true;
        let mut r = Ok(());
        for id in ids.into_iter().rev() {
            if let Err(t) = self.free_stream(id) {
                r = r.and(Err(t));
            }
        }
        self.closing_resources = false;
        r
    }
}

impl Vm {
    pub(crate) fn host_stream_filter_add(
        &mut self,
        res: &Value,
        filter: Filter,
        write: bool,
        append: bool,
        register: bool,
    ) -> Option<Value> {
        let id = res_id(res)?;
        self.ensure_layer(res, id);
        let l = self.layer_mut(id)?;
        if write {
            if append {
                l.write.push(filter);
            } else {
                l.write.insert(0, filter);
            }
        } else if append {
            l.read.push(filter);
            if !self.wind_prebuffered(id) {
                let f = self.layer_mut(id)?.read.pop()?;
                self.free_filter(f);
                return None;
            }
        } else {
            l.read.insert(0, filter);
        }
        if !register {
            return Some(Value::Null);
        }
        let rid = self.next_resource_id;
        self.next_resource_id += 1;
        self.native_resources
            .insert(rid, Box::new(FilterRes { stream: id }));
        let l = self.layer_mut(id)?;
        let chain = if write { &mut l.write } else { &mut l.read };
        let f = if append {
            chain.last_mut()
        } else {
            chain.first_mut()
        }?;
        f.res = Some(rid);
        Some(self.listed_resource(rid, "stream filter"))
    }

    pub(crate) fn host_stream_filter_remove(&mut self, fres: &Value) -> Result<bool, Thrown> {
        let bad = || {
            Thrown::type_error(
                "stream_filter_remove(): supplied resource is not a valid stream filter resource",
            )
        };
        let Value::Resource(r) = fres else {
            return Err(bad());
        };
        if r.type_name != "stream filter" {
            return Err(bad());
        }
        let rid = r.id;
        let stream = self
            .native_resources
            .get(&rid)
            .and_then(|b| b.downcast_ref::<FilterRes>())
            .map(|f| f.stream)
            .ok_or_else(bad)?;
        let Some(l) = self.layer(stream) else {
            return Err(bad());
        };
        let (write, idx) = if let Some(i) = l.read.iter().position(|f| f.res == Some(rid)) {
            (false, i)
        } else if let Some(i) = l.write.iter().position(|f| f.res == Some(rid)) {
            (true, i)
        } else {
            return Err(bad());
        };
        let res = self.filter_stream_value(stream);
        if !self.filter_flush(&res, stream, write, idx) {
            let msg = format!(
                "{}(): Unable to flush filter, not removing",
                self.docref_function()
            );
            builtins::main::streams::filter::raise(self, builtins::E_WARNING, msg.as_bytes());
            return Ok(false);
        }
        let Some(l) = self.layer_mut(stream) else {
            return Ok(true);
        };
        let chain = if write { &mut l.write } else { &mut l.read };
        let Some(i) = chain.iter().position(|f| f.res == Some(rid)) else {
            return Ok(true);
        };
        let f = chain.remove(i);
        self.free_filter(f);
        Ok(true)
    }
}

impl Vm {
    /// php://filter/…/resource=URL (php_stream_url_wrap_php): the inner
    /// stream, with the listed filters appended. `rest` follows "php://".
    pub(crate) fn open_php_filter(
        &mut self,
        rest: &[u8],
        mode: &[u8],
        ctx: &Value,
    ) -> Option<Value> {
        let mut mode_rw = 0;
        if mode.contains(&b'r') || mode.contains(&b'+') {
            mode_rw |= 1;
        }
        if mode.contains(&b'w') || mode.contains(&b'+') || mode.contains(&b'a') {
            mode_rw |= 2;
        }
        let pathdup = &rest[6..];
        let Some(at) = pathdup.windows(10).position(|w| w == b"/resource=") else {
            self.pending_throw
                .get_or_insert_with(|| Thrown::error("Error", "No URL resource specified"));
            return None;
        };
        let Some(res) = builtins::Host::stream_open_ctx(self, &pathdup[at + 10..], mode, ctx)
        else {
            // The inner open reports nothing; the outer one fails.
            builtins::main::streams::note_wrapper_error("operation failed");
            return None;
        };
        // php cuts the spec at "/resource=" and tokenizes from its second
        // byte: with no spec ("php://filter/resource=…") the cut is at byte
        // 0, so the whole rest, URL included, is read as filter names.
        let spec = if at == 0 {
            pathdup[1..].to_vec()
        } else {
            pathdup[..at].to_vec()
        };
        for tok in spec.split(|&c| c == b'/').filter(|t| !t.is_empty()) {
            let (list, read, write) = if tok.len() >= 5 && tok[..5].eq_ignore_ascii_case(b"read=") {
                (&tok[5..], true, false)
            } else if tok.len() >= 6 && tok[..6].eq_ignore_ascii_case(b"write=") {
                (&tok[6..], false, true)
            } else {
                (tok, mode_rw & 1 != 0, mode_rw & 2 != 0)
            };
            for name in list.split(|&c| c == b'|').filter(|t| !t.is_empty()) {
                let name = builtins::main::php_variables::url_decode(name);
                for (on, write) in [(read, false), (write, true)] {
                    if !on {
                        continue;
                    }
                    match builtins::main::streams::filter::create(self, &name, None) {
                        Some(f) => {
                            self.host_stream_filter_add(&res, f, write, true, false);
                        }
                        None => {
                            let mut msg =
                                format!("{}(): Unable to create filter (", self.docref_function())
                                    .into_bytes();
                            msg.extend_from_slice(&name);
                            msg.push(b')');
                            builtins::main::streams::filter::raise(self, builtins::E_WARNING, &msg);
                        }
                    }
                }
            }
        }
        if self.pending_throw.is_some() {
            if let Some(id) = res_id(&res) {
                let _ = self.free_stream(id);
            }
            return None;
        }
        Some(res)
    }
}

impl Vm {
    /// An include's read of its whole stream: `None` when reading failed.
    pub(crate) fn stream_read_all_for_include(&mut self, res: &Value) -> Option<Vec<u8>> {
        let id = res_id(res)?;
        if self.layer(id).is_some() {
            self.read_all_layered(res, id)
        } else {
            self.stream_read_unfiltered(res, -1)
        }
    }
}

impl crate::streams::Stream {
    /// Note a line read starting on a memory stream: php's read buffer
    /// fills with a chunk from there. False for other streams.
    pub(crate) fn note_mem_line_read(&mut self) -> bool {
        let StreamState::Mem { buf, pos, .. } = &self.state else {
            return false;
        };
        if self.filters.is_none() {
            match self.mem_buffered {
                Some(end) if end > *pos => {}
                _ => self.mem_buffered = Some((*pos + CHUNK).min(buf.len())),
            }
        }
        true
    }

    /// After a line read on a memory stream: php read on in chunks while
    /// the line went past its buffer.
    pub(crate) fn extend_mem_line_read(&mut self) {
        let StreamState::Mem { buf, pos, .. } = &self.state else {
            return;
        };
        if let Some(end) = self.mem_buffered.as_mut() {
            while *end < *pos && *end < buf.len() {
                *end = (*end + CHUNK).min(buf.len());
            }
        }
    }
}
