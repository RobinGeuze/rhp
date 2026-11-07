//! ext/standard/url_scanner_ex.re: the output URL rewriter behind
//! `output_add_rewrite_var()` (main/output.c), with url_rewriter.tags and
//! url_rewriter.hosts. The scanner is php's re2c DFA (url_scanner_ex.c),
//! ported state by state, including where it stops for more input: what it
//! has not finished scanning waits in `buf` for the next chunk, and goes out
//! as is on a flush.

use std::cell::RefCell;

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, Value};

use crate::{Host, OutputHandlerCtx, Thrown};

const STATE_PLAIN: u8 = 0;
const STATE_TAG: u8 = 1;
const STATE_NEXT_ARG: u8 = 2;
const STATE_ARG: u8 = 3;
const STATE_BEFORE_VAL: u8 = 4;
const STATE_VAL: u8 = 5;

/// `url_adapt_state_ex_t`: the output rewriter's, or the session's
/// (`session`: trans-sid, which reads session.trans_sid_tags and
/// session.trans_sid_hosts).
#[derive(Default)]
struct State {
    session: bool,
    tag: Vec<u8>,
    arg: Vec<u8>,
    val: Vec<u8>,
    buf: Vec<u8>,
    result: Vec<u8>,
    form_app: Vec<u8>,
    url_app: Vec<u8>,
    active: bool,
    /// The attribute the current tag's URL is in (url_rewriter.tags).
    lookup_data: Vec<u8>,
    state: u8,
    attr_val: Vec<u8>,
    tag_form: bool,
    attr_action: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static SESSION_STATE: RefCell<State> = RefCell::new(State::default());
}

/// Per-request reset (RSHUTDOWN / RINIT).
pub(crate) fn reset_state() {
    STATE.with(|s| *s.borrow_mut() = State::default());
    SESSION_STATE.with(|s| *s.borrow_mut() = State::default());
}

fn with_state<T>(session: bool, f: impl FnOnce(&mut State) -> T) -> T {
    let st = if session { &SESSION_STATE } else { &STATE };
    st.with(|s| f(&mut s.borrow_mut()))
}

fn ini(name: &[u8]) -> Vec<u8> {
    crate::ini_value(name).flatten().unwrap_or_default()
}

/// php_strtok_r over `,`: the non-empty pieces.
fn tokens(v: &[u8]) -> impl Iterator<Item = &[u8]> {
    v.split(|&c| c == b',').filter(|t| !t.is_empty())
}

/// url_rewriter.tags (session.trans_sid_tags) as php_ini_on_update_tags
/// reads it: lowercased tag → attribute, the first entry for a tag kept.
fn lookup_tag(tag: &[u8], session: bool) -> Option<Vec<u8>> {
    let tags = ini(if session {
        b"session.trans_sid_tags"
    } else {
        b"url_rewriter.tags"
    });
    for t in tokens(&tags) {
        let Some(eq) = t.iter().position(|&c| c == b'=') else {
            continue;
        };
        if t[..eq].to_ascii_lowercase() == tag {
            // The value is a C string.
            let v = &t[eq + 1..];
            return Some(v[..v.iter().position(|&c| c == 0).unwrap_or(v.len())].to_vec());
        }
    }
    None
}

/// url_rewriter.hosts (session.trans_sid_hosts), lowercased.
fn allowed_hosts(session: bool) -> Vec<Vec<u8>> {
    let name: &[u8] = if session {
        b"session.trans_sid_hosts"
    } else {
        b"url_rewriter.hosts"
    };
    tokens(&ini(name)).map(|t| t.to_ascii_lowercase()).collect()
}

fn arg_separator() -> Vec<u8> {
    ini(b"arg_separator.output")
}

/// append_modified_url: `url` with `url_app` added to its query, when it is
/// a relative or http(s) URL to an allowed host.
fn append_modified_url(url: &[u8], dest: &mut Vec<u8>, url_app: &[u8], sep: &[u8], session: bool) {
    let Some(parts) = crate::ext::standard::url::parse(url) else {
        dest.extend_from_slice(url);
        return;
    };
    if parts.fragment.is_some() && url.first() == Some(&b'#') {
        dest.extend_from_slice(url);
        return;
    }
    if let Some(s) = &parts.scheme {
        if !s.eq_ignore_ascii_case(b"http") && !s.eq_ignore_ascii_case(b"https") {
            dest.extend_from_slice(url);
            return;
        }
    }
    if let Some(h) = &parts.host {
        if !allowed_hosts(session).contains(&h.to_ascii_lowercase()) {
            dest.extend_from_slice(url);
            return;
        }
    }
    if parts.path.is_none() && parts.query.is_none() && parts.fragment.is_none() {
        dest.extend_from_slice(url);
        dest.extend_from_slice(b"/?");
        dest.extend_from_slice(url_app);
        return;
    }
    // smart_str_appends: the parts as C strings.
    let cs =
        |b: &[u8]| -> Vec<u8> { b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())].to_vec() };
    if let Some(s) = &parts.scheme {
        dest.extend_from_slice(&cs(s));
        dest.extend_from_slice(b"://");
    } else if url.starts_with(b"//") {
        dest.extend_from_slice(b"//");
    }
    if let Some(u) = &parts.user {
        dest.extend_from_slice(&cs(u));
        if let Some(p) = &parts.pass {
            dest.extend_from_slice(&cs(p));
            dest.push(b':');
        }
        dest.push(b'@');
    }
    if let Some(h) = &parts.host {
        dest.extend_from_slice(&cs(h));
    }
    if parts.port != 0 {
        dest.push(b':');
        dest.extend_from_slice(parts.port.to_string().as_bytes());
    }
    if let Some(p) = &parts.path {
        dest.extend_from_slice(&cs(p));
    }
    dest.push(b'?');
    if let Some(q) = &parts.query {
        dest.extend_from_slice(&cs(q));
        dest.extend_from_slice(sep);
    }
    dest.extend_from_slice(url_app);
    if let Some(f) = &parts.fragment {
        dest.push(b'#');
        dest.extend_from_slice(&cs(f));
    }
}

impl State {
    /// tag_arg: the value, rewritten when it is the tag's URL attribute.
    fn tag_arg(&mut self, quotes: bool, ty: u8) {
        if quotes {
            self.result.push(ty);
        }
        if self.arg.eq_ignore_ascii_case(&self.lookup_data) {
            let mut out = std::mem::take(&mut self.result);
            let sep = arg_separator();
            append_modified_url(&self.val, &mut out, &self.url_app, &sep, self.session);
            self.result = out;
        } else {
            self.result.extend_from_slice(&self.val);
        }
        if quotes {
            self.result.push(ty);
        }
    }

    /// check_host_whitelist for a form's action.
    fn check_host_whitelist(&self, host: &mut dyn Host) -> bool {
        if self.attr_val.is_empty() {
            return true;
        }
        let Some(parts) = crate::ext::standard::url::parse(&self.attr_val) else {
            return false;
        };
        if let Some(s) = &parts.scheme {
            if !s.eq_ignore_ascii_case(b"http") && !s.eq_ignore_ascii_case(b"https") {
                return false;
            }
        }
        let Some(h) = &parts.host else {
            return true;
        };
        let allowed = allowed_hosts(self.session);
        if allowed.is_empty() && check_http_host(host, h) {
            return true;
        }
        // Compared as given (php looks the host up without lowercasing).
        allowed.contains(h)
    }

    /// handle_form: the hidden inputs after an allowed `<form>`.
    fn handle_form(&mut self, host: &mut dyn Host) {
        if !self.form_app.is_empty()
            && self.tag.eq_ignore_ascii_case(b"form")
            && self.check_host_whitelist(host)
        {
            let app = self.form_app.clone();
            self.result.extend_from_slice(&app);
        }
    }

    /// handle_tag: whether the tag is one url_rewriter.tags names.
    fn handle_tag(&mut self, tag: &[u8]) {
        self.tag = tag.to_ascii_lowercase();
        match lookup_tag(&self.tag, self.session) {
            Some(attr) => {
                self.lookup_data = attr;
                self.tag_form = self.tag == b"form";
                self.state = STATE_NEXT_ARG;
            }
            None => self.state = STATE_PLAIN,
        }
    }

    fn handle_arg(&mut self, arg: &[u8]) {
        self.arg = arg.to_vec();
        // strncasecmp over the attribute's length: a prefix of "action".
        let n = self.arg.len().min(6);
        self.attr_action = self.tag_form
            && self.arg[..n].eq_ignore_ascii_case(&b"action"[..n])
            && (self.arg.len() <= 6 || self.arg[6] == 0);
    }

    fn handle_val(&mut self, tok: &[u8], quotes: bool, ty: u8) {
        let q = quotes as usize;
        self.val = tok[q..tok.len() - q].to_vec();
        if self.tag_form && self.attr_action {
            self.attr_val = self.val.clone();
        }
        self.tag_arg(quotes, ty);
    }

    /// xx_mainloop: scan `buf` + `new` from the saved state.
    fn mainloop(&mut self, host: &mut dyn Host, new: &[u8]) {
        let mut data = std::mem::take(&mut self.buf);
        data.extend_from_slice(new);
        let d = &data[..];
        let lim = d.len();
        let mut p = 0usize;
        let mut start;
        let alpha = |c: u8| c.is_ascii_alphabetic();
        let alphanamespace = |c: u8| c.is_ascii_alphabetic() || c == b':';
        let ws = |c: u8| matches!(c, b'\t' | b'\n' | 0x0b | b'\r' | b' ');
        // `continue 'scan` jumps to a state; `break 'scan` is YYFILL.
        'scan: loop {
            start = p;
            match self.state {
                STATE_PLAIN => {
                    if p >= lim {
                        break 'scan;
                    }
                    if d[p] == b'<' {
                        p += 1;
                        self.result.extend_from_slice(&d[start..p]);
                        self.state = STATE_TAG;
                        continue 'scan;
                    }
                    loop {
                        p += 1;
                        if p >= lim {
                            break 'scan;
                        }
                        if d[p] == b'<' {
                            break;
                        }
                    }
                    self.result.extend_from_slice(&d[start..p]);
                }
                STATE_TAG => {
                    if p >= lim {
                        break 'scan;
                    }
                    if !alphanamespace(d[p]) {
                        p += 1;
                        self.result.extend_from_slice(&d[start..p]);
                        self.state = STATE_PLAIN;
                        continue 'scan;
                    }
                    loop {
                        p += 1;
                        if p >= lim {
                            break 'scan;
                        }
                        if !alphanamespace(d[p]) {
                            break;
                        }
                    }
                    self.handle_tag(&d[start..p]);
                    self.result.extend_from_slice(&d[start..p]);
                }
                STATE_NEXT_ARG => {
                    if lim - p < 2 {
                        break 'scan;
                    }
                    let c = d[p];
                    if ws(c) {
                        loop {
                            p += 1;
                            if p >= lim {
                                break 'scan;
                            }
                            if !ws(d[p]) {
                                break;
                            }
                        }
                        self.result.extend_from_slice(&d[start..p]);
                    } else if c == b'>' || (c == b'/' && d[p + 1] == b'>') {
                        p += if c == b'/' { 2 } else { 1 };
                        self.result.extend_from_slice(&d[start..p]);
                        self.handle_form(host);
                        self.state = STATE_PLAIN;
                    } else if alpha(c) {
                        self.state = STATE_ARG;
                    } else {
                        p += 1;
                        self.result.extend_from_slice(&d[start..p]);
                        self.state = STATE_PLAIN;
                    }
                }
                STATE_ARG => {
                    if p >= lim {
                        break 'scan;
                    }
                    if !alpha(d[p]) {
                        p += 1;
                        self.result.extend_from_slice(&d[start..p]);
                        self.state = STATE_NEXT_ARG;
                        continue 'scan;
                    }
                    loop {
                        p += 1;
                        if p >= lim {
                            break 'scan;
                        }
                        if !(alpha(d[p]) || d[p] == b'-') {
                            break;
                        }
                    }
                    self.result.extend_from_slice(&d[start..p]);
                    self.handle_arg(&d[start..p]);
                    self.state = STATE_BEFORE_VAL;
                }
                STATE_BEFORE_VAL => {
                    if lim - p < 2 {
                        break 'scan;
                    }
                    // [ ]* "=" [ ]*, else back to the next attribute.
                    let mut q = p;
                    while q < lim && d[q] == b' ' {
                        q += 1;
                    }
                    if q >= lim {
                        // Spaces up to the end: re2c asks for more.
                        break 'scan;
                    }
                    if d[q] != b'=' {
                        self.state = STATE_NEXT_ARG;
                        continue 'scan;
                    }
                    p = q;
                    loop {
                        p += 1;
                        if p >= lim {
                            break 'scan;
                        }
                        if d[p] != b' ' {
                            break;
                        }
                    }
                    self.result.extend_from_slice(&d[start..p]);
                    self.state = STATE_VAL;
                }
                STATE_VAL => {
                    if lim - p < 2 {
                        break 'scan;
                    }
                    let c = d[p];
                    let unquoted =
                        |c: u8| !matches!(c, b' ' | b'\r' | b'\t' | b'\n' | b'>' | b'\'' | b'"');
                    if unquoted(c) {
                        loop {
                            p += 1;
                            if p >= lim {
                                break 'scan;
                            }
                            if !unquoted(d[p]) {
                                break;
                            }
                        }
                        let tok = d[start..p].to_vec();
                        self.handle_val(&tok, false, b' ');
                    } else if c == b'"' || c == b'\'' {
                        let mut q = p + 1;
                        if d[q] == b'>' {
                            p += 1;
                            self.result.extend_from_slice(&d[start..p]);
                        } else {
                            loop {
                                if q >= lim {
                                    break 'scan;
                                }
                                if d[q] == c || d[q] == b'>' {
                                    break;
                                }
                                q += 1;
                            }
                            if d[q] == c {
                                p = q + 1;
                                let tok = d[start..p].to_vec();
                                self.handle_val(&tok, true, c);
                            } else {
                                // A `>` inside the quotes: just the quote.
                                p += 1;
                                self.result.extend_from_slice(&d[start..p]);
                            }
                        }
                    } else {
                        p += 1;
                        self.result.extend_from_slice(&d[start..p]);
                    }
                    self.state = STATE_NEXT_ARG;
                }
                _ => unreachable!(),
            }
        }
        self.buf = d[start.min(lim)..].to_vec();
    }

    /// url_adapt_ext.
    fn adapt(&mut self, host: &mut dyn Host, src: &[u8], flush: bool) -> Vec<u8> {
        self.mainloop(host, src);
        let mut out = std::mem::take(&mut self.result);
        if flush {
            out.append(&mut self.buf);
            self.val.clear();
            self.attr_val.clear();
        }
        out
    }
}

/// check_http_host: `$_SERVER['HTTP_HOST']` (port off) is `target`.
fn check_http_host(host: &mut dyn Host, target: &[u8]) -> bool {
    let Some(Value::Array(server)) = host.global_var(b"_SERVER") else {
        return false;
    };
    let Some(Value::Str(h)) = server.get(&ArrayKey::from_bytes(b"HTTP_HOST")).map(deref) else {
        return false;
    };
    let h = h.as_bytes();
    let h = &h[..h
        .iter()
        .position(|&c| c == b':' || c == 0)
        .unwrap_or(h.len())];
    let t = &target[..target.iter().position(|&c| c == 0).unwrap_or(target.len())];
    h.eq_ignore_ascii_case(t)
}

/// php_url_scanner_output_handler, the "URL-Rewriter" output handler.
fn output_handler(host: &mut dyn Host, cx: &mut OutputHandlerCtx) -> bool {
    handler_impl(host, cx, false)
}

/// php_url_scanner_session_handler, the session's "URL-Rewriter".
fn session_handler(host: &mut dyn Host, cx: &mut OutputHandlerCtx) -> bool {
    handler_impl(host, cx, true)
}

fn handler_impl(host: &mut dyn Host, cx: &mut OutputHandlerCtx, session: bool) -> bool {
    // PHP_OUTPUT_HANDLER_FLUSH | PHP_OUTPUT_HANDLER_FINAL.
    let flush = cx.op & 0x0c != 0;
    let mut st = with_state(session, std::mem::take);
    cx.out = if !st.url_app.is_empty() {
        st.adapt(host, cx.input, flush)
    } else if !st.buf.is_empty() {
        let mut out = std::mem::take(&mut st.buf);
        out.extend_from_slice(cx.input);
        st.result.clear();
        out
    } else {
        cx.input.to_vec()
    };
    with_state(session, |s| *s = st);
    true
}

/// php_raw_url_encode.
fn raw_url_encode(s: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = Vec::with_capacity(s.len());
    for &b in s {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b),
            _ => out.extend_from_slice(&[b'%', HEX[(b >> 4) as usize], HEX[(b & 0xf) as usize]]),
        }
    }
    out
}

/// php_escape_html_entities_ex(ENT_QUOTES|ENT_SUBSTITUTE, no double
/// encoding, quiet) in the default charset.
fn escape_html(host: &mut dyn Host, s: &[u8]) -> Vec<u8> {
    crate::ext::standard::html::escape_quotes_substitute(host, s)
}

/// php_url_scanner_add_var.
fn fn_output_add_rewrite_var(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = conv::to_string_bytes(&deref(&args[0]));
    let value = conv::to_string_bytes(&deref(&args[1]));
    // What the encoded copies and the grown buffers take (php allocates
    // them all, and runs out of memory as they grow).
    let (url_len, form_len) = with_state(false, |s| (s.url_app.len(), s.form_app.len()));
    let need = (name.len() + value.len()) * 3 + url_len + form_len;
    if !rhp_values::mem::reserve(need) || !rhp_values::mem::reserve(need * 2) {
        return Ok(Value::Null);
    }
    add_var(host, &name, &value, false)?;
    Ok(Value::Bool(true))
}

/// php_url_scanner_add_session_var (encoded).
pub(crate) fn add_session_var(
    host: &mut dyn Host,
    name: &[u8],
    value: &[u8],
) -> Result<(), Thrown> {
    add_var(host, name, value, true)
}

/// php_url_scanner_add_var_impl (encoded).
fn add_var(host: &mut dyn Host, name: &[u8], value: &[u8], session: bool) -> Result<(), Thrown> {
    let sname = raw_url_encode(name);
    let svalue = raw_url_encode(value);
    let hname = escape_html(host, name);
    let hvalue = escape_html(host, value);
    let sep = arg_separator();
    let start = with_state(session, |s| {
        let start = !s.active;
        if start {
            *s = State {
                active: true,
                session,
                ..State::default()
            };
        }
        if !s.url_app.is_empty() {
            s.url_app.extend_from_slice(&sep);
        }
        s.url_app.extend_from_slice(&sname);
        s.url_app.push(b'=');
        s.url_app.extend_from_slice(&svalue);
        s.form_app
            .extend_from_slice(b"<input type=\"hidden\" name=\"");
        s.form_app.extend_from_slice(&hname);
        s.form_app.extend_from_slice(b"\" value=\"");
        s.form_app.extend_from_slice(&hvalue);
        s.form_app.extend_from_slice(b"\" />");
        start
    });
    if start {
        let func = if session {
            session_handler
        } else {
            output_handler
        };
        host.output_control(crate::host::ObCall::StartInternal {
            name: "URL-Rewriter",
            func,
        })?;
    }
    Ok(())
}

/// php_url_scanner_reset_var_impl (encoded) for the session's variable:
/// `name=…` out of url_app and its hidden input out of form_app.
pub(crate) fn reset_session_var(host: &mut dyn Host, name: &[u8]) {
    if with_state(true, |s| s.url_app.is_empty()) {
        return;
    }
    let sname = raw_url_encode(name);
    let hname = escape_html(host, name);
    let sep = arg_separator();
    with_state(true, |s| {
        let mut url_app = sname;
        url_app.push(b'=');
        let mut form_app = b"<input type=\"hidden\" name=\"".to_vec();
        form_app.extend_from_slice(&hname);
        form_app.extend_from_slice(b"\" value=\"");
        let Some(start) = find(&s.url_app, &url_app) else {
            return;
        };
        let mut start = start;
        let limit = s.url_app.len();
        let mut end = start + url_app.len();
        let mut sep_removed = false;
        while end < limit {
            if !sep.is_empty() && s.url_app[end..].starts_with(&sep) {
                end += sep.len();
                sep_removed = true;
                break;
            }
            end += 1;
        }
        if s.url_app.len() == end - start {
            s.url_app.clear();
            s.form_app.clear();
            return;
        }
        // The separator before it goes when none followed.
        if !sep_removed && start >= sep.len() && s.url_app[start - sep.len()..start] == sep[..] {
            start -= sep.len();
        }
        s.url_app.drain(start..end);
        let Some(fstart) = find(&s.form_app, &form_app) else {
            s.url_app.clear();
            s.form_app.clear();
            return;
        };
        start = fstart;
        let limit = s.form_app.len();
        let mut end = start + form_app.len();
        while end < limit {
            if s.form_app[end] == b'>' {
                end += 1;
                break;
            }
            end += 1;
        }
        s.form_app.drain(start..end);
    });
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// php_url_scanner_reset_vars.
fn fn_output_reset_rewrite_vars(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    with_state(false, |s| {
        s.form_app.clear();
        s.url_app.clear();
    });
    Ok(Value::Bool(true))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("output_add_rewrite_var", fn_output_add_rewrite_var),
            ("output_reset_rewrite_vars", fn_output_reset_rewrite_vars),
        ],
    );
}
