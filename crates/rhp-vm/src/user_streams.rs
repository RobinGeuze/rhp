//! Streams of user wrappers (`stream_wrapper_register`): the VM keeps the
//! [`UserStream`] in the stream table and lends it out for each operation,
//! so the wrapper's methods run with no table borrow held.

use std::rc::Rc;

use rhp_builtins::main::streams::userspace::{self, UserStream, UserWrapper};
use rhp_builtins::{self as builtins, Host};
use rhp_values::Value;

use crate::streams::{res_id, StreamState};
use crate::Vm;

impl Vm {
    /// Whether `res` is a stream a user wrapper opened.
    pub(crate) fn is_user_stream(&self, res: &Value) -> bool {
        res_id(res)
            .and_then(|id| self.streams.get(&id))
            .is_some_and(|s| matches!(s, StreamState::User(_)))
    }

    /// Run `f` on the user stream behind `res`: `None` when it is not one
    /// (or an operation on it is running already).
    #[inline]
    pub(crate) fn with_user<R>(
        &mut self,
        res: &Value,
        f: impl FnOnce(&mut UserStream, &mut Vm) -> R,
    ) -> Option<R> {
        let id = res_id(res)?;
        let rec = self.streams.record_mut(id)?;
        let StreamState::User(slot) = &mut rec.state else {
            return None;
        };
        let mut us = slot.take()?;
        let was = std::mem::replace(&mut rec.no_fclose, true);
        let r = f(&mut us, self);
        self.user_put(id, us, was);
        Some(r)
    }

    fn user_put(&mut self, id: u32, us: Box<UserStream>, no_fclose: bool) {
        match self.streams.record_mut(id) {
            Some(rec) => {
                rec.no_fclose = no_fclose;
                if let StreamState::User(slot) = &mut rec.state {
                    *slot = Some(us);
                }
            }
            // closed meanwhile (cannot happen while no_fclose is set)
            None => {
                let mut us = us;
                us.close(self);
            }
        }
    }

    pub(crate) fn host_user_wrapper(&self, protocol: &[u8]) -> Option<Rc<UserWrapper>> {
        if self.user_wrappers.is_empty() {
            return None;
        }
        userspace::as_user(self.wrappers.get(protocol)?)
    }

    pub(crate) fn host_register_wrapper(&mut self, protocol: &[u8], w: Rc<UserWrapper>) -> bool {
        let builtin_outside = matches!(protocol, b"php" | b"data" | b"http" | b"https")
            && !self.wrappers_off.contains_key(protocol);
        if self.wrappers.contains_key(protocol) || builtin_outside {
            return false;
        }
        self.wrappers.insert(protocol.to_vec(), w);
        self.user_wrappers.push(protocol.to_vec());
        true
    }

    pub(crate) fn host_user_stream_insert(
        &mut self,
        s: Box<UserStream>,
        ctx: Option<Value>,
    ) -> Value {
        let v = self.make_stream(StreamState::User(Some(s)), "stream");
        if let Some(cid) = ctx.as_ref().and_then(res_id) {
            if let Some(rec) = res_id(&v).and_then(|id| self.streams.record_mut(id)) {
                rec.ctx = Some(cid);
            }
        }
        v
    }

    pub(crate) fn host_user_stream_take(&mut self, res: &Value) -> Option<Box<UserStream>> {
        let id = res_id(res)?;
        let rec = self.streams.record_mut(id)?;
        let s = match &mut rec.state {
            StreamState::User(slot) => slot.take()?,
            _ => return None,
        };
        rec.no_fclose = true;
        Some(s)
    }

    pub(crate) fn host_user_stream_put(&mut self, res: &Value, s: Box<UserStream>) {
        let Some(id) = res_id(res) else { return };
        self.user_put(id, s, false);
    }

    /// `php_stream_open_wrapper_ex` through a user wrapper, for fopen and
    /// the other openers: the resource, or `None` (the error is queued
    /// for the "Failed to open stream" warning).
    pub(crate) fn open_user_stream(
        &mut self,
        w: &Rc<UserWrapper>,
        path: &[u8],
        open_path: &[u8],
        mode: &[u8],
        options: i64,
        ctx: &Value,
    ) -> Option<Value> {
        // php_stream_context_from_zval(zcontext, 0): the default context
        let ctx = match ctx {
            Value::Resource(_) => Some(ctx.clone()),
            _ => Some(builtins::Host::default_stream_context(self)),
        };
        let opened = userspace::open(self, w, open_path, mode, options, ctx.as_ref())?;
        let mut s = opened.stream;
        s.orig_path = path.to_vec();
        s.opened_for_append(self);
        Some(self.host_user_stream_insert(s, ctx))
    }

    /// Close a user stream whose record was removed from the table.
    pub(crate) fn close_user_state(&mut self, s: Option<Box<UserStream>>) {
        if let Some(mut s) = s {
            s.close(self);
        }
    }

    /// The `stream_get_meta_data` fields of a user stream.
    pub(crate) fn user_stream_meta(&self, id: u32) -> Option<builtins::StreamMeta> {
        let StreamState::User(Some(s)) = self.streams.get(&id)? else {
            return None;
        };
        Some(builtins::StreamMeta {
            wrapper_type: "user-space",
            stream_type: s.label(),
            mode: s.mode.clone(),
            uri: s.orig_path.clone(),
            unread_bytes: s.unread() as i64,
            seekable: !s.no_seek,
            eof: s.eof,
            blocked: true,
            timed_out: false,
            crypto: None,
            wrapper_data: Some(s.obj.clone()),
            rfc2397: None,
        })
    }
}

/// What an include through a user wrapper came to.
pub(crate) enum UserInclude {
    /// Opened and read: the compiled name, the source and the stream to
    /// close once compiled.
    Opened {
        key: Vec<u8>,
        data: Vec<u8>,
        stream: Box<UserStream>,
    },
    /// The open failed: the reason for "Failed to open stream" (none:
    /// no such warning).
    Failed(Option<String>),
    /// An `_once` include of a file already included.
    AlreadyIncluded,
}

// php_streams.h open options
const STREAM_OPEN_FOR_ZEND_STREAM: i64 = 0x10000;
const STREAM_ASSUME_REALPATH: i64 = 0x4000;

impl Vm {
    /// include / require through a user wrapper (php_stream_open_for_zend_ex
    /// and zend_stream_fixup): `None` when no user wrapper serves the path.
    pub(crate) fn user_include(
        &mut self,
        raw: &[u8],
        kind: rhp_bytecode::IncludeKind,
    ) -> Result<Option<UserInclude>, rhp_values::Thrown> {
        let (w, open_path, resolved) = match userspace::locate(self, raw) {
            Some((w, p)) => (w, p, false),
            None => match self.user_resolve_path(raw)? {
                Some((w, p)) => (w, p, true),
                None => return Ok(None),
            },
        };
        let fname = kind.name();
        // php_stream_locate_url_wrapper's URL protection
        if w.is_url {
            let on = |vm: &Self, n: &[u8]| crate::ini_bool(&vm.ini_get(n).unwrap_or_default());
            let off = if !on(self, b"allow_url_fopen") {
                Some("allow_url_fopen")
            } else if !on(self, b"allow_url_include") {
                Some("allow_url_include")
            } else {
                None
            };
            if let Some(off) = off {
                let n = raw.iter().position(|&b| b == b':').unwrap_or(0);
                let msg = format!(
                    "{fname}(): {}:// wrapper is disabled in the server configuration by {off}=0",
                    String::from_utf8_lossy(&raw[..n])
                );
                self.diag(builtins::E_WARNING, &msg)?;
                return Ok(Some(UserInclude::Failed(Some(
                    "no suitable wrapper could be found".into(),
                ))));
            }
        }
        let options = userspace::STREAM_OPEN_FOR_INCLUDE
            | STREAM_OPEN_FOR_ZEND_STREAM
            | if resolved {
                STREAM_ASSUME_REALPATH
            } else {
                userspace::USE_PATH
            };
        self.include_docref = Some(fname);
        let opened = userspace::open(self, &w, &open_path, b"rb", options, None);
        if let Some(t) = self.pending_throw.take() {
            self.include_docref = None;
            if let Some(o) = opened {
                self.close_user_state(Some(o.stream));
            }
            return Err(t);
        }
        let Some(opened) = opened else {
            self.include_docref = None;
            let reason = builtins::main::streams::take_wrapper_error()
                .unwrap_or_else(|| "operation failed".to_string());
            return Ok(Some(UserInclude::Failed(Some(reason))));
        };
        let mut s = opened.stream;
        s.orig_path = raw.to_vec();
        let key = opened.opened_path.unwrap_or_else(|| open_path.clone());
        if kind.is_once() && self.included_files.contains(&key) {
            self.include_docref = None;
            self.close_user_state(Some(s));
            return Ok(Some(UserInclude::AlreadyIncluded));
        }
        // php_stream_open_for_zend_ex: no read buffer; zend_stream_fixup
        // reads the size the stream reports, else in growing chunks.
        s.set_option(self, userspace::OPTION_READ_BUFFER, userspace::BUFFER_NONE, None);
        let size = match s.stat(self) {
            Some(st) => st.size as usize,
            None => 0,
        };
        let mut data = Vec::new();
        let mut failed = false;
        if size > 0 {
            loop {
                match s.read(self, size - data.len()) {
                    Some(b) if !b.is_empty() => data.extend_from_slice(&b),
                    Some(_) => break,
                    None => {
                        failed = true;
                        break;
                    }
                }
                if data.len() >= size {
                    // the next read returns 0 bytes
                    match s.read(self, 0) {
                        Some(_) => {}
                        None => failed = true,
                    }
                    break;
                }
            }
        } else {
            let mut remain = 4096;
            loop {
                match s.read(self, remain) {
                    Some(b) if !b.is_empty() => {
                        remain -= b.len();
                        data.extend_from_slice(&b);
                        if remain == 0 {
                            remain = data.len();
                        }
                    }
                    Some(_) => break,
                    None => {
                        failed = true;
                        break;
                    }
                }
            }
        }
        self.include_docref = None;
        if failed || self.pending_throw.is_some() {
            self.close_user_state(Some(s));
            if let Some(t) = self.pending_throw.take() {
                return Err(t);
            }
            return Ok(Some(UserInclude::Failed(None)));
        }
        Ok(Some(UserInclude::Opened {
            key,
            data,
            stream: s,
        }))
    }

    /// php_resolve_path over an include_path with user-wrapper entries: the
    /// first entry whose `url_stat` (quiet) succeeds. `None` when the path
    /// is not searched or a plain entry holds the file.
    fn user_resolve_path(
        &mut self,
        raw: &[u8],
    ) -> Result<Option<(Rc<UserWrapper>, Vec<u8>)>, rhp_values::Thrown> {
        if raw.is_empty()
            || raw[0] == b'/'
            || raw.starts_with(b"./")
            || raw.starts_with(b"../")
            || raw.windows(3).any(|w| w == b"://")
        {
            return Ok(None);
        }
        let ip = self.ini_get(b"include_path").unwrap_or_default();
        if !ip.windows(3).any(|w| w == b"://") {
            return Ok(None);
        }
        for dir in ip.split(|&c| c == b':') {
            if dir.is_empty() {
                continue;
            }
            let mut trypath = dir.to_vec();
            trypath.push(b'/');
            trypath.extend_from_slice(raw);
            if let Some((w, _)) = userspace::locate(self, &trypath) {
                if !dir.windows(3).any(|w| w == b"://") {
                    // a plain entry, while `file://` is a user wrapper
                    continue;
                }
                let st = userspace::url_stat(self, &w, &trypath, userspace::URL_STAT_QUIET, None);
                if let Some(t) = self.pending_throw.take() {
                    return Err(t);
                }
                if st.is_some() {
                    return Ok(Some((w, trypath)));
                }
                continue;
            }
            if dir.windows(3).any(|w| w == b"://") {
                continue;
            }
            let p = builtins::zend::virtual_cwd::os_path(&trypath);
            if std::fs::canonicalize(&p).is_ok() {
                return Ok(None);
            }
        }
        Ok(None)
    }
}
