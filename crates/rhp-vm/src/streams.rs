//! Stream resources: the open-stream table and the helpers behind the stream
//! builtins.

use std::collections::HashMap;
use std::rc::Rc;

use rhp_builtins::{self as builtins, Host};
use rhp_values::{Thrown, Value};

use crate::{deref, Vm};

/// php's memory-stream mode (`TEMP_STREAM_READONLY` / `_APPEND` / default).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MemMode {
    ReadWrite,
    ReadOnly,
    Append,
}

/// The open stream resources by resource id, one [`Stream`] record each:
/// closing a stream drops everything known about it.
#[derive(Default)]
pub(crate) struct StreamTable(HashMap<u32, Stream>);

/// An open stream resource: its backing state and what php keeps on the
/// `php_stream` beside it.
pub(crate) struct Stream {
    pub(crate) state: StreamState,
    /// The fopen() mode of a stream opened by `stream_open`.
    pub(crate) mode: Option<Vec<u8>>,
    /// (uri, mode) of a stream opened through a non-local wrapper
    /// (`phar://…`), for fstat / stream_get_meta_data.
    pub(crate) origin: Option<(Vec<u8>, Vec<u8>)>,
    /// (uri, mode) of a descriptor stream opened by name (`php://stdin`).
    pub(crate) fd_name: Option<(Vec<u8>, Vec<u8>)>,
    /// A `data:` (RFC 2397) stream's metadata, mode and uri.
    pub(crate) data_url: Option<(Value, Vec<u8>, Vec<u8>)>,
    /// The stream's context resource id (php's `stream->ctx`).
    pub(crate) ctx: Option<u32>,
    /// Opened on `php://output`: writes go through the output buffers like
    /// `echo` (unlike `php://stdout`).
    pub(crate) output: bool,
    /// Switched to non-blocking mode (stream_get_meta_data's `blocked`).
    pub(crate) nonblocking: bool,
}

impl StreamTable {
    pub(crate) fn insert(&mut self, id: u32, state: StreamState) {
        let s = Stream {
            state,
            mode: None,
            origin: None,
            fd_name: None,
            data_url: None,
            ctx: None,
            output: false,
            nonblocking: false,
        };
        self.0.insert(id, s);
    }

    pub(crate) fn remove(&mut self, id: &u32) -> Option<Stream> {
        self.0.remove(id)
    }

    pub(crate) fn contains_key(&self, id: &u32) -> bool {
        self.0.contains_key(id)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The backing state of stream `id`.
    pub(crate) fn get(&self, id: &u32) -> Option<&StreamState> {
        self.0.get(id).map(|s| &s.state)
    }

    pub(crate) fn get_mut(&mut self, id: &u32) -> Option<&mut StreamState> {
        self.0.get_mut(id).map(|s| &mut s.state)
    }

    /// The whole record of stream `id`.
    pub(crate) fn record(&self, id: u32) -> Option<&Stream> {
        self.0.get(&id)
    }

    pub(crate) fn record_mut(&mut self, id: u32) -> Option<&mut Stream> {
        self.0.get_mut(&id)
    }

    /// The context resource a stream stands for (its own, when attached),
    /// or `id` itself (a context resource, or a stream without one).
    pub(crate) fn context_of(&self, id: u32) -> u32 {
        self.record(id).and_then(|s| s.ctx).unwrap_or(id)
    }
}

/// The backing state of an open stream resource.
pub(crate) enum StreamState {
    /// Standard output / `php://output` — writes to the VM output sink.
    Stdout,
    /// Standard error — writes to the process stderr.
    Stderr,
    /// Standard input (reads return EOF for now).
    Stdin,
    /// An in-memory / preloaded-file buffer (`php://memory`, a file opened for
    /// reading): read from `pos`, writes append.
    /// `skew` is php's `stream->position` minus the buffer offset: they
    /// diverge after a failed seek (position -1) or a shrinking ftruncate,
    /// which moves only the memory stream's own offset.
    Mem {
        buf: Vec<u8>,
        pos: usize,
        skew: i64,
        /// php's `stream->eof`: set by a read that found no more data,
        /// cleared by a successful seek.
        eof: bool,
        /// The URI of a `php://temp` stream (a TEMP stream wrapping a
        /// MEMORY one); `None` for memory / wrapper-backed buffers.
        temp: Option<Vec<u8>>,
        /// php's `TEMP_STREAM_*` mode (read-only, append, read/write).
        mode: MemMode,
    },
    /// A file opened for writing: buffered, flushed to the wrapper on
    /// flush/close. `append` keeps existing content. (Non-local wrappers.)
    FileWrite { buf: Vec<u8>, url: Vec<u8> },
    /// A plain local file with a real descriptor.
    File(Box<builtins::main::streams::LocalFile>),
    /// A directory stream (`opendir`): the entries snapshot + read position.
    Dir {
        entries: Vec<Vec<u8>>,
        pos: usize,
        url: Vec<u8>,
    },
    /// An OS file descriptor stream: a `proc_open` pipe's parent end or
    /// `php://stdin` (a dup of fd 0).
    Fd(builtins::main::streams::plain_wrapper::FdStream),
    /// A stream on an extension's ops (zip entries).
    Ops(Box<builtins::main::streams::ops::OpsStream>),
}

impl Vm {
    /// `fgets` over any readable stream, optionally capped at `max` bytes.
    pub(crate) fn stream_gets_impl(&mut self, res: &Value, max: Option<usize>) -> Option<Vec<u8>> {
        let id = res_id(res)?;
        match self.streams.get_mut(&id)? {
            StreamState::Mem {
                buf,
                pos,
                eof,
                temp,
                ..
            } => {
                if *pos >= buf.len() {
                    *eof = true;
                    return None;
                }
                let start = *pos;
                let nl = buf[start..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map(|i| start + i + 1);
                let mut end = nl.unwrap_or(buf.len());
                let capped = max.is_some_and(|m| start + m < end);
                if let Some(m) = max {
                    end = end.min(start + m);
                }
                // A line that runs to the end of the data (no newline) makes
                // php look for more, which sets eof.
                if nl.is_none() && !capped {
                    *eof = true;
                }
                let line = buf[start..end].to_vec();
                *pos = end;
                if temp.is_some() && *pos >= buf.len() {
                    *eof = true;
                }
                Some(line)
            }
            StreamState::File(f) => f.gets(max),
            StreamState::Ops(s) => {
                let line = s.gets(max);
                self.raise_ops_stream_diags(id);
                line
            }
            // FdStream::gets takes php's `$length` (it reads `length - 1`).
            StreamState::Fd(f) => f.gets(max.map(|m| m + 1)),
            StreamState::Stdin => self.stdin_stream()?.gets(max.map(|m| m + 1)),
            _ => None,
        }
    }

    /// Free a stream's backing state (fclose / last reference dropped):
    /// closes the descriptor (releasing flock locks) and flushes buffered
    /// wrapper writes. An extension's close may call PHP and throw.
    pub(crate) fn free_stream(&mut self, id: u32) -> Result<bool, Thrown> {
        Ok(match self.streams.remove(&id).map(|s| s.state) {
            Some(StreamState::FileWrite { buf, url }) => self
                .wrapper_for(&url)
                .map(|w| w.put_contents(&url, &buf))
                .is_some(),
            Some(StreamState::Ops(mut s)) => {
                s.close(self)?;
                true
            }
            Some(_) => true,
            None => false,
        })
    }

    /// Free the streams whose last resource reference has been dropped,
    /// returning the first exception a close threw.
    pub(crate) fn reap_dropped_streams(&mut self) -> Result<(), Thrown> {
        let mut thrown = None;
        for id in rhp_values::take_dropped_streams() {
            // The CLI std streams live for the whole request.
            if matches!(
                self.streams.get(&id),
                Some(StreamState::Stdin | StreamState::Stdout | StreamState::Stderr)
            ) {
                continue;
            }
            if self.streams.contains_key(&id) {
                if let Err(t) = self.free_stream(id) {
                    thrown.get_or_insert(t);
                }
            } else if let Some(data) = self.native_resources.remove(&id) {
                builtins::native_resource_dtor(self, data);
            }
        }
        thrown.map_or(Ok(()), Err)
    }

    /// Deliver the notifications a read or write on stream `res` owes its
    /// context's notifier (progress, completed).
    pub(crate) fn deliver_stream_events(&mut self, res: &Value) {
        let Some(id) = res_id(res) else { return };
        let events = match self.streams.get_mut(&id) {
            Some(StreamState::Fd(f)) => f.take_events(),
            _ => return,
        };
        if events.is_empty() {
            return;
        }
        if let Err(e) = builtins::http::deliver(self, res, &events) {
            if self.pending_throw.is_none() {
                self.pending_throw = Some(e);
            }
        }
    }

    /// Raise the diagnostics stream operations queued (the TLS layer's
    /// warnings) as raised by builtin `func`.
    #[inline]
    pub(crate) fn flush_stream_diags(&mut self, func: &str) {
        if builtins::main::streams::plain_wrapper::has_diags() {
            self.raise_stream_diags(func);
        }
    }

    fn raise_stream_diags(&mut self, func: &str) {
        for d in builtins::main::streams::plain_wrapper::take_diags() {
            let msg = if d.docref {
                format!("{func}(): {}", d.msg)
            } else {
                d.msg
            };
            Host::raise_diagnostic(self, d.level, &msg);
        }
    }

    /// Allocate a stream resource backed by `state`, returning its resource Value.
    pub(crate) fn make_stream(&mut self, state: StreamState, type_name: &'static str) -> Value {
        let id = self.next_resource_id;
        self.next_resource_id += 1;
        self.streams.insert(id, state);
        Value::Resource(Rc::new(rhp_values::ResourceData { id, type_name }))
    }

    /// A wrapper's own opener (`zip://`), given the call's stream context
    /// (null: the default one). `None` when the wrapper has none.
    pub(crate) fn open_wrapper_stream(
        &mut self,
        path: &[u8],
        mode: &[u8],
        ctx: &Value,
    ) -> Option<Option<Value>> {
        let w = self.wrapper_for(path)?.clone();
        if !w.has_stream_opener() {
            return None;
        }
        let ctx = match ctx {
            Value::Resource(_) => ctx.clone(),
            _ => self.default_stream_context(),
        };
        let options = self.stream_context_options(&ctx);
        let mut diags = Vec::new();
        let opened = w.open_stream(path, mode, &options, &mut diags)?;
        self.raise_ops_diags(diags);
        Some(opened.map(|s| self.make_stream(StreamState::Ops(Box::new(s)), "stream")))
    }

    /// Report the diagnostics an ops stream's last operation raised, under
    /// the running builtin.
    pub(crate) fn raise_ops_stream_diags(&mut self, id: u32) {
        if let Some(StreamState::Ops(s)) = self.streams.get_mut(&id) {
            let diags = s.take_diags();
            self.raise_ops_diags(diags);
        }
    }

    fn raise_ops_diags(&mut self, diags: Vec<(i64, String)>) {
        if diags.is_empty() {
            return;
        }
        let func = self.native_calls.last().map_or("Unknown", |n| n.name);
        for (level, msg) in diags {
            self.raise_diagnostic(level, &format!("{func}(): {msg}"));
        }
    }

    /// The stream wrapper for a URL's scheme (default `file://`).
    pub(crate) fn wrapper_for(&self, url: &[u8]) -> Option<&Rc<dyn builtins::StreamWrapper>> {
        let (scheme, _) = builtins::main::streams::split_scheme(url);
        self.wrappers.get(scheme)
    }

    /// The `STDIN` reader (a dup of fd 0), created on first use.
    pub(crate) fn stdin_stream(
        &mut self,
    ) -> Option<&mut builtins::main::streams::plain_wrapper::FdStream> {
        if self.stdin_reader.is_none() {
            let fd = builtins::main::streams::plain_wrapper::dup_raw(0)?;
            self.stdin_reader = Some(builtins::main::streams::plain_wrapper::FdStream::new(fd));
        }
        self.stdin_reader.as_mut()
    }
}

/// The resource id of a (possibly referenced) resource value.
pub(crate) fn res_id(v: &Value) -> Option<u32> {
    match deref(v) {
        Value::Resource(r) => Some(r.id),
        _ => None,
    }
}
