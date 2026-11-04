//! The engine side of the SAPI layer: the output sink, lazy header sending
//! (`php_output_header` / `sapi_send_headers`), the response a request
//! builds, and the request input (superglobals, `php://input`).

use rhp_builtins::rfc1867::{self, BodyState, Diag};
use rhp_builtins::sapi::{self, RequestInfo, Response, Sapi, TreatData};
use rhp_builtins::{self as builtins, Host};
use rhp_values::{ArrayKey, PhpArray, Value};

use crate::Vm;

/// Per-request SAPI state kept on the [`Vm`].
pub(crate) struct SapiState {
    /// The server API the request runs under.
    pub(crate) sapi: Box<dyn Sapi>,
    /// The response being built (`SG(sapi_headers)`).
    pub(crate) response: Response,
    /// The request's input, when a SAPI supplied one.
    pub(crate) request: Option<RequestInfo>,
    /// The message being displayed came from a builtin's
    /// `php_error_docref` (HTML-escaped under html_errors).
    pub(crate) docref_msg: bool,
    /// Request startup is running (php's `during_request_startup`):
    /// diagnostics are reported in "Unknown" on line 0.
    pub(crate) startup: bool,
    /// The body exceeded `post_max_size`: `$_POST` stays empty.
    pub(crate) post_too_big: bool,
    /// php-cgi: module-startup diagnostics (level, message) shown after the
    /// response body.
    pub(crate) deferred_startup_diags: Vec<(i64, String)>,
    /// The exit status when the script ends without `exit`: 255 after an
    /// E_ERROR at module startup (`EG(exit_status)`).
    pub(crate) startup_status: i32,
    /// What has been read of the request body, and this request's uploads
    /// (deleted when the request ends).
    pub(crate) body: BodyState,
}

impl SapiState {
    /// The state for a request under `sapi`, after its name and ini defaults
    /// were installed for this thread ([`builtins::sapi::set_sapi`]).
    pub(crate) fn new(sapi: Box<dyn Sapi>) -> SapiState {
        let response = Response {
            store: sapi.stores_headers(),
            code: sapi.initial_response_code(),
            ..Response::default()
        };
        SapiState {
            sapi,
            response,
            request: None,
            docref_msg: false,
            startup: false,
            post_too_big: false,
            deferred_startup_diags: Vec::new(),
            startup_status: 0,
            body: BodyState::default(),
        }
    }
}

impl Vm {
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
        // opcache's own log (its ini handlers' warnings at module startup).
        let (lines, dest) = builtins::take_accel_log();
        for line in lines {
            self.write_accel_log(&line, &dest);
        }
        // Warnings from applying the -d values (module startup): php-cli
        // shows them before anything else; php-cgi's reach the client only
        // after the response body.
        let diags = builtins::take_startup_diagnostics();
        // php_error_cb: an E_ERROR before the module is up sets the exit
        // status but does not bail out; the script still runs.
        if diags.iter().any(|d| d.0 == 1) {
            self.sapi.startup_status = 255;
        }
        if builtins::sapi::is_cgi() {
            self.sapi.deferred_startup_diags = diags;
        } else {
            for (level, msg) in diags {
                self.module_startup_diag(level, &msg);
            }
        }
        // sapi_activate reads the POST body before the headers are set up.
        self.check_post_size();
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

    /// A line of opcache's log, to stderr or the `opcache.error_log` file.
    pub(crate) fn write_accel_log(&mut self, line: &str, dest: &[u8]) {
        if dest.is_empty() {
            self.write_stderr(line.as_bytes());
        } else {
            let _ = builtins::vcwd::append_file(dest, line.as_bytes(), 0o666);
        }
    }

    /// A warning raised during request startup ("PHP Request Startup: …"
    /// in Unknown on line 0, as php's docref reports it with no function
    /// running).
    fn startup_warning(&mut self, msg: &str) {
        self.startup_diag(2, &format!("PHP Request Startup: {msg}"));
    }

    /// php_error_cb for a warning, or an ini handler's E_ERROR, raised
    /// before the module finished starting up: never recorded for
    /// `error_get_last()` nor passed to a handler; displayed when
    /// display_errors and display_startup_errors allow, otherwise always
    /// logged. An E_ERROR obeys error_reporting, carries the
    /// `fatal_error_backtraces` trace, and its docref message is escaped
    /// under html_errors (and escaped again when displayed, as in php).
    fn module_startup_diag(&mut self, level: i64, msg: &str) {
        let ini = |vm: &Self, n: &[u8]| vm.ini_get(n).unwrap_or_default();
        let shown = !matches!(
            crate::errors::display_errors_mode(&ini(self, b"display_errors")),
            crate::errors::DisplayErrors::Off
        ) && crate::ini_bool(&ini(self, b"display_startup_errors"));
        if level != rhp_builtins::E_ERROR {
            if shown {
                self.display_error(
                    rhp_builtins::E_WARNING,
                    "Warning",
                    msg.as_bytes(),
                    b"Unknown",
                    0,
                );
            } else {
                let line = format!("PHP Warning:  {msg} in Unknown on line 0\n");
                self.write_stderr(line.as_bytes());
            }
            return;
        }
        if builtins::php_fns::error_reporting_level(self) & 1 == 0 {
            return;
        }
        let mut msg = msg.as_bytes().to_vec();
        if crate::ini_bool(&ini(self, b"html_errors")) {
            msg = builtins::sapi::escape_html(&msg, &ini(self, b"default_charset"));
        }
        let trace: &[u8] = if crate::ini_bool(&ini(self, b"fatal_error_backtraces")) {
            b"#0 {main}"
        } else {
            b""
        };
        if shown {
            self.display_error_trace(1, "Fatal error", &msg, b"Unknown", 0, trace);
        } else {
            let mut line = b"PHP Fatal error:  ".to_vec();
            line.extend_from_slice(&msg);
            line.extend_from_slice(b" in Unknown on line 0\n");
            if !trace.is_empty() {
                line.extend_from_slice(b"Stack trace:\n");
                line.extend_from_slice(trace);
                line.push(b'\n');
            }
            self.write_stderr(&line);
        }
    }

    /// A diagnostic raised during request startup (in Unknown on line 0).
    fn startup_diag(&mut self, level: i64, msg: &str) {
        self.sapi.startup = true;
        self.raise_error(level, msg);
        self.sapi.startup = false;
    }

    /// `sapi_activate`'s `sapi_read_post_data`: a POST body of any type but
    /// multipart (whose parser reads it later, itself) is buffered by
    /// `sapi_read_standard_form_data`, unless it is over `post_max_size`:
    /// then it is not parsed into `$_POST` (`php://input` still reads it).
    fn check_post_size(&mut self) {
        let Some(req) = self.sapi.request.as_ref() else {
            return;
        };
        let Some(ct) = req.content_type.as_deref() else {
            return;
        };
        if req.method.as_deref() != Some(b"POST")
            || rfc1867::media_type(ct) == b"multipart/form-data"
        {
            return;
        }
        let len = req.content_length.unwrap_or(req.body.len() as i64);
        let reading = self
            .ini_get(b"enable_post_data_reading")
            .unwrap_or_default();
        if !crate::ini_bool(&reading) {
            return;
        }
        let max = builtins::ini_parse_quantity(&self.ini_get(b"post_max_size").unwrap_or_default());
        if max > 0 && len > max {
            self.sapi.post_too_big = true;
            self.startup_warning(&format!(
                "POST Content-Length of {len} bytes exceeds the limit of {max} bytes"
            ));
            return;
        }
        let over = match self.sapi.request.as_ref() {
            Some(req) => self.sapi.body.read_standard(&req.body, max),
            None => false,
        };
        if over {
            self.startup_warning(&rfc1867::actual_length_exceeds(max));
        }
    }

    /// php_request_shutdown's tail: send the headers if no output did, then
    /// flush the SAPI. Call after [`Vm::run_main`].
    pub fn finish_request(&mut self) {
        // zend_ini_deactivate restores startup values changed at runtime
        // (re-running their handlers); php-cgi's deferred startup warnings
        // come out with them.
        for (level, msg) in std::mem::take(&mut self.sapi.deferred_startup_diags) {
            self.module_startup_diag(level, &msg);
        }
        // The module is up by now: the usual display / log rules apply.
        for d in builtins::take_shutdown_diagnostics() {
            self.display_error(
                rhp_builtins::E_WARNING,
                "Warning",
                d.as_bytes(),
                b"Unknown",
                0,
            );
        }
        self.sapi_send_headers();
        self.sapi.sapi.flush();
        // sapi_deactivate_destroy: uploads not moved away are deleted.
        self.sapi.body.cleanup();
        rhp_values::mem::end_request();
    }

    /// Write bytes through the output layer (output buffers, then the
    /// SAPI), as `echo` does.
    pub fn write_output(&mut self, data: &[u8]) {
        builtins::Host::write_out(self, data);
    }

    /// Write to the request's error stream (the SAPI's `write_stderr`).
    pub fn write_error_stream(&mut self, data: &[u8]) {
        self.write_stderr(data);
    }

    /// Where the currently executing user code is (file, line), if any.
    pub(crate) fn exec_location(&self) -> Option<(Vec<u8>, i64)> {
        let i = (0..self.stack.len())
            .rev()
            .find(|&i| !self.stack[i].proto.is_internal())?;
        let file = self.current_file(i);
        if file.is_empty() {
            return None;
        }
        Some((file, self.current_line(i) as i64))
    }

    /// Unbuffered output on its way to the SAPI (`php_output_op`'s tail):
    /// the headers go out first, then the bytes reach the sink.
    pub(crate) fn sapi_write(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        if !self.sapi.response.sent {
            self.php_output_header();
        }
        if !self.sapi.response.output_disabled {
            self.sapi.sapi.ub_write(data);
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
        self.sapi.sapi.send_headers(&self.sapi.response);
        self.sapi.response.status_line = None;
    }

    /// php_error_cb's bailout step for a fatal error: with display_errors
    /// off and nothing sent yet, a 200 becomes a 500.
    pub(crate) fn fatal_status(&mut self) {
        let display = self.ini_get(b"display_errors").unwrap_or_default();
        if !matches!(
            crate::errors::display_errors_mode(&display),
            crate::errors::DisplayErrors::Off
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

    /// The request body (`php://input`): what the POST readers left of it
    /// (empty without a request).
    pub(crate) fn request_body(&self) -> Vec<u8> {
        self.sapi
            .request
            .as_ref()
            .map(|r| self.sapi.body.input(&r.body))
            .unwrap_or_default()
    }

    /// Build the superglobals from the request (php_hash_environment and
    /// the `php_auto_globals_create_*` callbacks).
    pub(crate) fn seed_request_superglobals(&mut self) {
        let Some(req) = self.sapi.request.take() else {
            return;
        };
        self.sapi.startup = true;
        self.seed_request_superglobals_from(&req);
        self.sapi.startup = false;
        self.sapi.request = Some(req);
    }

    /// "Input variables exceeded N" for one request input.
    fn input_vars_exceeded(&mut self, exceeded: bool, max_vars: i64) {
        if exceeded {
            self.startup_warning(&format!(
                "Input variables exceeded {max_vars}. To increase the limit change max_input_vars in php.ini."
            ));
        }
    }

    fn seed_request_superglobals_from(&mut self, req: &RequestInfo) {
        let ini = |vm: &Self, n: &[u8]| vm.ini_get(n).unwrap_or_default();
        let order = ini(self, b"variables_order");
        let has = |c: u8| order.iter().any(|b| b.eq_ignore_ascii_case(&c));
        let seps = ini(self, b"arg_separator.input");
        let max_vars = builtins::c_atol(&ini(self, b"max_input_vars"));

        let get = if has(b'G') {
            let (a, over) = parse(
                TreatData::Form,
                req.query_string.as_deref(),
                &seps,
                max_vars,
            );
            self.input_vars_exceeded(over, max_vars);
            a
        } else {
            PhpArray::new()
        };
        let mut files = PhpArray::new();
        // sapi_activate reads the body only for a method of exactly "POST";
        // php_auto_globals_create_post skips it once output (a startup
        // warning) sent the headers.
        let post = if has(b'P')
            && !self.sapi.post_too_big
            && !self.sapi.response.sent
            && req.method.as_deref() == Some(b"POST")
        {
            self.read_post(req, max_vars, &mut files)
        } else {
            PhpArray::new()
        };
        let cookie = if has(b'C') {
            let (a, over) = parse(TreatData::Cookie, req.cookie.as_deref(), &seps, max_vars);
            self.input_vars_exceeded(over, max_vars);
            a
        } else {
            PhpArray::new()
        };
        let mut server = PhpArray::new();
        if has(b'S') {
            for (k, v) in &req.server {
                import_env_var(&mut server, k, v);
            }
            for (k, v) in sapi::auth_server_vars(req.authorization.as_deref().unwrap_or_default()) {
                server.insert(ArrayKey::from_bytes(k), Value::str(v));
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
    }

    /// A directive's value, `None` when it is registered with a null value.
    fn ini_get_opt(&self, name: &[u8]) -> Option<Vec<u8>> {
        builtins::ini_value(name).flatten()
    }

    /// `sapi_read_post_data` + the post handlers: urlencoded bodies fill
    /// `$_POST`, multipart ones `$_POST` and `$_FILES` (`rfc1867`).
    fn read_post(&mut self, req: &RequestInfo, max_vars: i64, files: &mut PhpArray) -> PhpArray {
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
                // php_std_post_handler parses what the reader buffered.
                let data = self
                    .sapi
                    .body
                    .buffered()
                    .unwrap_or_else(|| self.sapi.body.remaining(&req.body).to_vec());
                let (a, over) = sapi::post_form_vars(&data, max_vars);
                self.input_vars_exceeded(over, max_vars);
                post = a;
            }
            b"multipart/form-data" => {
                let lim = rfc1867::Limits::new(self, &rfc1867::BodyOptions::default());
                let len = req.content_length.unwrap_or(req.body.len() as i64);
                let mut out = rfc1867::parse(
                    self.sapi.body.remaining(&req.body),
                    self.sapi.body.read_bytes(),
                    &rfc1867::content_type_dup(ct),
                    len,
                    &lim,
                    &mut post,
                    files,
                );
                self.sapi.body.apply(&mut out);
                for d in out.diags {
                    match d {
                        Diag::Emit(m) => self.startup_warning(&m),
                        Diag::Notice(m) => {
                            self.startup_diag(8, &format!("PHP Request Startup: {m}"))
                        }
                        Diag::Sapi(m) => self.startup_diag(2, &m),
                    }
                }
            }
            _ => {}
        }
        post
    }
}

fn parse(kind: TreatData, data: Option<&[u8]>, seps: &[u8], max_vars: i64) -> (PhpArray, bool) {
    match data {
        Some(d) if !d.is_empty() => sapi::treat_data(kind, d, seps, max_vars),
        _ => (PhpArray::new(), false),
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
