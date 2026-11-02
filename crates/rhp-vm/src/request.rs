//! The engine side of the SAPI layer: the output sink, lazy header sending
//! (`php_output_header` / `sapi_send_headers`), the response a request
//! builds, and the request input (superglobals, `php://input`).
//!
//! Without a [`Sapi`] installed the VM keeps its built-in php-cli behaviour
//! (output collected for [`Vm::output`] or passed through to stdout), with
//! php-cli's header semantics: nothing stored, `headers_sent()` true after
//! the first output byte.

use rhp_builtins::sapi::{self, RequestInfo, Response, Sapi, TreatData};
use rhp_builtins::{self as builtins, Host};
use rhp_values::{ArrayKey, PhpArray, Value};

use crate::{is_prelude_proto, Vm};

/// Per-request SAPI state kept on the [`Vm`].
#[derive(Default)]
pub(crate) struct SapiState {
    /// The installed server API (None: the built-in CLI behaviour).
    pub(crate) sapi: Option<Box<dyn Sapi>>,
    /// The response being built (`SG(sapi_headers)`).
    pub(crate) response: Response,
    /// The request's input, when a SAPI supplied one.
    pub(crate) request: Option<RequestInfo>,
    /// The message being displayed came from a builtin's
    /// `php_error_docref` (HTML-escaped under html_errors).
    pub(crate) docref_msg: bool,
}

impl Vm {
    /// Install the server API this request runs under: its name
    /// (`php_sapi_name()`, `PHP_SAPI`), ini defaults, header semantics and
    /// output sink. Call right after [`Vm::new`], before ini settings,
    /// compiling and running.
    pub fn set_sapi(&mut self, sapi: Box<dyn Sapi>) {
        let name = sapi.name().to_string();
        builtins::sapi::set_sapi(&name, &sapi.ini_defaults());
        self.constants
            .insert(b"PHP_SAPI".to_vec(), Value::str(name.into_bytes()));
        self.sapi.response.store = sapi.stores_headers();
        self.sapi.response.code = sapi.initial_response_code();
        self.sapi.sapi = Some(sapi);
    }

    /// Hand the engine the request's input (server variables, query string,
    /// cookies, body …). Superglobals are built from it when the script
    /// starts.
    pub fn set_request(&mut self, req: RequestInfo) {
        let r = &mut self.sapi.response;
        if req.response_code != 0 {
            r.code = req.response_code;
        }
        r.method = req.method.clone();
        r.proto_num = if req.proto_num == 0 {
            1000
        } else {
            req.proto_num
        };
        r.headers_only = req.method.as_deref() == Some(b"HEAD");
        self.sapi.request = Some(req);
    }

    /// php_request_startup's header step: `X-Powered-By` when expose_php.
    /// Call after ini settings are applied, before running the script.
    pub fn start_request(&mut self) {
        let expose = self.ini_get(b"expose_php").unwrap_or_default();
        if crate::ini_bool(&expose) && !self.sapi.response.sent {
            let line = format!("X-Powered-By: PHP/{}", builtins::PHP_VERSION);
            let _ = sapi::header_op(self, sapi::HeaderOp::Replace(line.as_bytes(), 0));
        }
        // php-cgi -q marks the headers sent after request startup (so the
        // X-Powered-By header above is still listed by headers_list()).
        if self.sapi.request.as_ref().is_some_and(|r| r.no_headers) {
            let r = &mut self.sapi.response;
            r.no_headers = true;
            r.sent = true;
        }
    }

    /// php_request_shutdown's tail: send the headers if no output did, then
    /// flush the SAPI. Call after [`Vm::run_main`].
    pub fn finish_request(&mut self) {
        self.sapi_send_headers();
        if let Some(s) = self.sapi.sapi.as_mut() {
            s.flush();
        }
    }

    /// Write bytes through the output layer (output buffers, then the
    /// SAPI), as `echo` does.
    pub fn write_output(&mut self, data: &[u8]) {
        builtins::Host::write_out(self, data);
    }

    /// The response as it stands (status, headers, whether they were sent).
    pub fn response(&self) -> &Response {
        &self.sapi.response
    }

    /// Mutable access to the response (a front end setting the status
    /// before the script runs, e.g. php-cgi's 404 "No input file").
    pub fn response_mut(&mut self) -> &mut Response {
        &mut self.sapi.response
    }

    /// The installed SAPI (to recover its state after the request).
    pub fn take_sapi(&mut self) -> Option<Box<dyn Sapi>> {
        self.sapi.sapi.take()
    }

    /// Where the currently executing user code is (file, line), if any.
    pub(crate) fn exec_location(&self) -> Option<(Vec<u8>, i64)> {
        let i = (0..self.stack.len())
            .rev()
            .find(|&i| !is_prelude_proto(&self.stack[i].proto))?;
        let file = self.current_file(i);
        if file.is_empty() {
            return None;
        }
        Some((file, self.current_line(i) as i64))
    }

    /// Unbuffered output on its way to the SAPI (`php_output_op`'s tail):
    /// the headers go out first, then the bytes reach the sink.
    pub(crate) fn sapi_write(&mut self, data: &[u8]) -> bool {
        if data.is_empty() {
            return true;
        }
        if !self.sapi.response.sent {
            self.php_output_header();
        }
        if self.sapi.response.output_disabled {
            return true;
        }
        match self.sapi.sapi.as_mut() {
            Some(s) => {
                s.ub_write(data);
                true
            }
            None => false,
        }
    }

    /// `php_output_header`: remember where output started, then send the
    /// headers (a HEAD request then discards the body).
    fn php_output_header(&mut self) {
        if self.sapi.response.output_start.is_none() {
            self.sapi.response.output_start = self.exec_location();
        }
        self.sapi_send_headers();
        if self.sapi.response.headers_only {
            self.sapi.response.output_disabled = true;
        }
    }

    /// `sapi_send_headers`: the default Content-type, the
    /// `header_register_callback` callback, then the SAPI's `send_headers`.
    pub(crate) fn sapi_send_headers(&mut self) {
        if self.sapi.response.sent || self.sapi.response.no_headers {
            return;
        }
        sapi::take_default_content_type(self);
        if let Some(cb) = self.sapi.response.callback.take() {
            if let Err(t) = self.call_callable_value(&cb, Vec::new()) {
                if self.pending_throw.is_none() {
                    self.pending_throw = Some(t);
                }
            }
        }
        self.sapi.response.sent = true;
        if let Some(s) = self.sapi.sapi.as_mut() {
            s.send_headers(&self.sapi.response);
        }
        self.sapi.response.status_line = None;
    }

    /// php_error_cb's bailout step for a fatal error: with display_errors
    /// off and nothing sent yet, a 200 becomes a 500.
    pub(crate) fn fatal_status(&mut self) {
        let display = self.ini_get(b"display_errors").unwrap_or_default();
        if !matches!(
            crate::display_errors_mode(&display),
            crate::DisplayErrors::Off
        ) {
            return;
        }
        let r = &self.sapi.response;
        if r.sent || r.code != 200 {
            return;
        }
        let _ = sapi::header_op(
            self,
            sapi::HeaderOp::Replace(b"HTTP/1.0 500 Internal Server Error", 0),
        );
    }

    /// The request body (`php://input`): empty without a request.
    pub(crate) fn request_body(&self) -> Vec<u8> {
        self.sapi
            .request
            .as_ref()
            .map(|r| r.body.clone())
            .unwrap_or_default()
    }

    /// Build the superglobals from the request (php_hash_environment and
    /// the `php_auto_globals_create_*` callbacks).
    pub(crate) fn seed_request_superglobals(&mut self) {
        let Some(req) = self.sapi.request.take() else {
            return;
        };
        let ini = |vm: &Self, n: &[u8]| vm.ini_get(n).unwrap_or_default();
        let order = ini(self, b"variables_order");
        let has = |c: u8| order.iter().any(|b| b.eq_ignore_ascii_case(&c));
        let seps = ini(self, b"arg_separator.input");
        let max_vars = builtins::c_atol(&ini(self, b"max_input_vars"));

        let get = if has(b'G') {
            parse(
                TreatData::Form,
                req.query_string.as_deref(),
                &seps,
                max_vars,
            )
        } else {
            PhpArray::new()
        };
        let mut files = PhpArray::new();
        let post = if has(b'P')
            && !self.sapi.response.sent
            && req
                .method
                .as_deref()
                .is_some_and(|m| m.eq_ignore_ascii_case(b"POST"))
        {
            self.read_post(&req, &seps, max_vars, &mut files)
        } else {
            PhpArray::new()
        };
        let cookie = if has(b'C') {
            parse(TreatData::Cookie, req.cookie.as_deref(), &seps, max_vars)
        } else {
            PhpArray::new()
        };
        let mut server = PhpArray::new();
        if has(b'S') {
            for (k, v) in &req.server {
                import_env_var(&mut server, k, v);
            }
            server.insert(
                ArrayKey::from_bytes(b"REQUEST_TIME_FLOAT"),
                Value::Float(req.request_time),
            );
            server.insert(
                ArrayKey::from_bytes(b"REQUEST_TIME"),
                Value::Int(req.request_time as i64),
            );
            if let Some(argv) = &req.argv {
                let a = argv_array(argv);
                server.insert(ArrayKey::from_bytes(b"argv"), Value::Array(a));
                server.insert(ArrayKey::from_bytes(b"argc"), Value::Int(argv.len() as i64));
            }
            check_http_proxy(&mut server, &req.env);
        }
        let mut env = PhpArray::new();
        if has(b'E') {
            for (k, v) in &req.env {
                import_env_var(&mut env, k, v);
            }
        }
        check_http_proxy(&mut env, &req.env);
        // $_REQUEST: request_order, else variables_order (G, P, C once each).
        let rorder = match self.ini_get_opt(b"request_order") {
            Some(o) => o,
            None => order.clone(),
        };
        let mut request = PhpArray::new();
        let mut done = [false; 3];
        for c in rorder.iter().map(u8::to_ascii_uppercase) {
            let (i, src) = match c {
                b'G' => (0, &get),
                b'P' => (1, &post),
                b'C' => (2, &cookie),
                _ => continue,
            };
            if !done[i] {
                autoglobal_merge(&mut request, src);
                done[i] = true;
            }
        }
        let set = |vm: &mut Vm, name: &[u8], v: Value| {
            *vm.global_cell(name).borrow_mut() = v;
        };
        set(self, b"_GET", Value::Array(get));
        set(self, b"_POST", Value::Array(post));
        set(self, b"_COOKIE", Value::Array(cookie));
        set(self, b"_FILES", Value::Array(files));
        set(self, b"_SERVER", Value::Array(server));
        set(self, b"_ENV", Value::Array(env));
        set(self, b"_REQUEST", Value::Array(request));
        if let Some(argv) = &req.argv {
            set(self, b"argv", Value::Array(argv_array(argv)));
            set(self, b"argc", Value::Int(argv.len() as i64));
        }
        self.sapi.request = Some(req);
    }

    /// A directive's value, `None` when it is registered with a null value.
    fn ini_get_opt(&self, name: &[u8]) -> Option<Vec<u8>> {
        builtins::ini_value(name).flatten()
    }

    /// `sapi_read_post_data` + the post handlers: urlencoded bodies fill
    /// `$_POST`; multipart goes to the installed handler (none: empty).
    fn read_post(
        &mut self,
        req: &RequestInfo,
        seps: &[u8],
        max_vars: i64,
        files: &mut PhpArray,
    ) -> PhpArray {
        let reading = crate::ini_bool(
            &self
                .ini_get(b"enable_post_data_reading")
                .unwrap_or_default(),
        );
        let Some(ct) = req.content_type.as_deref().filter(|c| !c.is_empty()) else {
            return PhpArray::new();
        };
        if !reading {
            return PhpArray::new();
        }
        // The content type up to ';', ',' or ' ', lowercased.
        let end = ct
            .iter()
            .position(|&b| matches!(b, b';' | b',' | b' '))
            .unwrap_or(ct.len());
        let kind = ct[..end].to_ascii_lowercase();
        let mut post = PhpArray::new();
        match &kind[..] {
            b"application/x-www-form-urlencoded" => {
                post = parse(TreatData::Form, Some(&req.body), seps, max_vars);
            }
            b"multipart/form-data" => {
                if let Some(h) = sapi::multipart_handler() {
                    h(self, &req.body, ct, &mut post, files);
                }
            }
            _ => {}
        }
        post
    }
}

fn parse(kind: TreatData, data: Option<&[u8]>, seps: &[u8], max_vars: i64) -> PhpArray {
    match data {
        Some(d) if !d.is_empty() => sapi::treat_data(kind, d, seps, max_vars).0,
        _ => PhpArray::new(),
    }
}

fn argv_array(argv: &[Vec<u8>]) -> PhpArray {
    let mut a = PhpArray::new();
    for arg in argv {
        a.push(Value::str(arg.clone()));
    }
    a
}

/// `import_environment_variable`: names with ' ', '.' or '[' are skipped.
fn import_env_var(arr: &mut PhpArray, k: &[u8], v: &[u8]) {
    if k.is_empty() || k.iter().any(|&b| matches!(b, b' ' | b'.' | b'[')) {
        return;
    }
    arr.insert(ArrayKey::from_bytes(k), Value::str(v.to_vec()));
}

/// `check_http_proxy` (bug #72573): `HTTP_PROXY` only from the real
/// environment, never from a request header.
fn check_http_proxy(arr: &mut PhpArray, env: &[(Vec<u8>, Vec<u8>)]) {
    let key = ArrayKey::from_bytes(b"HTTP_PROXY");
    if arr.get(&key).is_none() {
        return;
    }
    match env.iter().find(|(k, _)| k == b"HTTP_PROXY") {
        Some((_, v)) => {
            arr.insert(key, Value::str(v.clone()));
        }
        None => {
            arr.remove(&key);
        }
    }
}

/// `php_autoglobal_merge`: nested arrays merge recursively, anything else
/// overwrites.
fn autoglobal_merge(dest: &mut PhpArray, src: &PhpArray) {
    for (k, v) in src.iter() {
        if let Value::Array(sa) = v {
            if let Some(Value::Array(da)) = dest.get_mut(k) {
                autoglobal_merge(da, sa);
                continue;
            }
        }
        dest.insert(k.clone(), v.clone());
    }
}
