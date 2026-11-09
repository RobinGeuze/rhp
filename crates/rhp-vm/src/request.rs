//! The engine side of the SAPI layer: the output sink, lazy header sending
//! (`php_output_header` / `sapi_send_headers`), the response a request
//! builds, and the request input (superglobals, `php://input`).

use rhp_builtins::main::rfc1867;
use rhp_builtins::main::rfc1867::{BodyState, Diag};
use rhp_builtins::main::sapi;
use rhp_builtins::main::sapi::{RequestInfo, Response, Sapi, TreatData};
use rhp_builtins::{self as builtins, Host};
use rhp_values::{ArrayKey, PhpArray, Thrown, Value};

use crate::Vm;

/// Per-request SAPI state kept on the [`Vm`].
pub(crate) struct SapiState {
    /// The server API the request runs under.
    pub(crate) sapi: Box<dyn Sapi>,
    /// The response being built (`SG(sapi_headers)`).
    pub(crate) response: Response,
    /// The request's input, when a SAPI supplied one.
    pub(crate) request: Option<RequestInfo>,
    /// Request startup is running (php's `during_request_startup`):
    /// diagnostics are reported in "Unknown" on line 0.
    pub(crate) startup: bool,
    /// The body exceeded `post_max_size`: `$_POST` stays empty.
    pub(crate) post_too_big: bool,
    /// php-cgi: module-startup diagnostics (level, message) shown after the
    /// response body.
    pub(crate) deferred_startup_diags: Vec<(i64, String)>,
    /// The exit status when the script ends without `exit`
    /// (`EG(exit_status)`): 255 after an E_ERROR at module startup, or
    /// after php-cli failed to write the output.
    pub(crate) exit_status: i32,
    /// Module startup failed (an E_CORE_ERROR): the request doesn't run.
    pub(crate) startup_failed: bool,
    /// What has been read of the request body, and this request's uploads
    /// (deleted when the request ends).
    pub(crate) body: BodyState,
    /// `fastcgi_finish_request()` completed the response: later output is
    /// discarded.
    pub(crate) response_ended: bool,
    /// A worker's request is being served (between
    /// `frankenphp_handle_request()` taking it and completing it); before
    /// the first one, the worker's own start-up counts as one.
    pub(crate) worker_in_request: bool,
}

impl SapiState {
    /// The state for a request under `sapi`, after its name and ini defaults
    /// were installed for this thread ([`builtins::main::sapi::set_sapi`]).
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
            startup: false,
            post_too_big: false,
            deferred_startup_diags: Vec::new(),
            exit_status: 0,
            startup_failed: false,
            body: BodyState::default(),
            response_ended: false,
            worker_in_request: true,
        }
    }
}

impl Vm {
    /// Hand the engine the request's input (server variables, query string,
    /// cookies, body …). Superglobals are built from it when the script
    /// starts.
    pub fn set_request(&mut self, mut req: RequestInfo) {
        if let Some(reader) = req.body_reader.take() {
            self.sapi.body.set_stream(reader);
        }
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
        builtins::zend::call_stack::init();
        builtins::zend::timeout::request_startup();
        // opcache's own log (its ini handlers' warnings at module startup).
        let (lines, dest) = builtins::take_accel_log();
        for line in lines {
            self.write_accel_log(&line, &dest);
        }
        // Warnings from applying the -d values (module startup): php-cli
        // shows them before anything else; php-cgi logs them at once
        // (stderr) but displays them only after the response body.
        let diags = builtins::take_startup_diagnostics();
        // php_error_cb: an E_ERROR before the module is up sets the exit
        // status but does not bail out; the script still runs.
        if diags.iter().any(|d| d.0 == 1) {
            self.sapi.exit_status = 255;
        }
        // An E_CORE_ERROR fails module startup: nothing runs.
        if diags.iter().any(|d| d.0 == rhp_builtins::E_CORE_ERROR) {
            self.sapi.startup_failed = true;
        }
        if builtins::main::sapi::is_cgi() {
            if self.startup_diags_shown() {
                self.sapi.deferred_startup_diags = diags;
            } else {
                for (level, msg) in diags {
                    self.module_startup_diag(level, &msg);
                }
            }
        } else {
            // Shown before the request starts: the headers are not sent by
            // them (sapi_activate starts the response afresh).
            let (sent, start) = (
                self.sapi.response.sent,
                self.sapi.response.output_start.clone(),
            );
            for (level, msg) in diags {
                self.module_startup_diag(level, &msg);
            }
            self.sapi.response.sent = sent;
            self.sapi.response.output_start = start;
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
            let _ = builtins::zend::virtual_cwd::append_file(dest, line.as_bytes(), 0o666);
        }
    }

    /// Whether module startup failed (php exits with status 1 before
    /// running anything).
    pub fn startup_failed(&self) -> bool {
        self.sapi.startup_failed
    }

    /// A warning raised during request startup ("PHP Request Startup: …"
    /// in Unknown on line 0, as php's docref reports it with no function
    /// running).
    fn startup_warning(&mut self, msg: &str) {
        self.startup_diag(2, &format!("PHP Request Startup: {msg}"));
    }

    /// The request body readers' diagnostics, raised during startup.
    fn body_diags(&mut self, diags: Vec<Diag>) {
        for d in diags {
            match d {
                Diag::Emit(m) | Diag::Warning(m) => self.startup_warning(&m),
                Diag::Notice(m) => self.startup_diag(8, &format!("PHP Request Startup: {m}")),
                Diag::Sapi(m) => self.startup_diag(2, &m),
            }
        }
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
        let shown = self.startup_diags_shown();
        if level != rhp_builtins::E_ERROR && level != rhp_builtins::E_CORE_ERROR {
            let (level, label) = if level == rhp_builtins::E_DEPRECATED {
                (level, "Deprecated")
            } else {
                (rhp_builtins::E_WARNING, "Warning")
            };
            if shown {
                self.display_error(level, label, msg.as_bytes(), b"Unknown", 0);
            } else {
                let line = format!("PHP {label}:  {msg} in Unknown on line 0\n");
                self.write_stderr(line.as_bytes());
            }
            return;
        }
        if builtins::zend::builtin_functions::error_reporting_level(self) & level == 0 {
            return;
        }
        let mut msg = msg.as_bytes().to_vec();
        if crate::ini_bool(&ini(self, b"html_errors")) {
            msg = builtins::main::sapi::escape_html(self, &msg);
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

    /// Whether module-startup diagnostics are displayed (else logged).
    fn startup_diags_shown(&self) -> bool {
        let ini = |vm: &Self, n: &[u8]| vm.ini_get(n).unwrap_or_default();
        !matches!(
            crate::errors::display_errors_mode(&ini(self, b"display_errors")),
            crate::errors::DisplayErrors::Off
        ) && crate::ini_bool(&ini(self, b"display_startup_errors"))
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
        let len = req.declared_length();
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
        let mut diags = Vec::new();
        let over = match self.sapi.request.as_ref() {
            Some(req) => self.sapi.body.read_standard(&req.body, max, &mut diags),
            None => false,
        };
        self.body_diags(diags);
        if over {
            self.startup_warning(&rfc1867::actual_length_exceeds(max));
        }
    }

    /// php_request_shutdown's tail: send the headers if no output did, then
    /// flush the SAPI. Call after [`Vm::run_main`].
    pub fn finish_request(&mut self) {
        builtins::zend::timeout::unset_timeout();
        // zend_ini_deactivate restores startup values changed at runtime
        // (re-running their handlers); php-cgi's deferred startup warnings
        // come out with them.
        for (level, msg) in std::mem::take(&mut self.sapi.deferred_startup_diags) {
            self.module_startup_diag(level, &msg);
        }
        // The module is up by now: the usual display / log rules apply.
        for (level, d) in builtins::take_shutdown_diagnostics() {
            let label = if level == rhp_builtins::E_DEPRECATED {
                "Deprecated"
            } else {
                "Warning"
            };
            self.display_error(level, label, d.as_bytes(), b"Unknown", 0);
        }
        self.sapi_send_headers();
        // sapi_deactivate: the SAPI's last flush (a failure only marks the
        // connection aborted now).
        if !self.sapi.response_ended && !self.sapi.sapi.flush() {
            builtins::main::set_connection_aborted();
        }
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
        if self.sapi.response_ended {
            return;
        }
        if !self.sapi.response.sent {
            self.php_output_header();
        }
        if !self.sapi.response.output_disabled && !self.sapi.sapi.ub_write(data) {
            if let Some(code) = self.sapi.sapi.aborted_exit_status() {
                self.sapi.exit_status = code;
            }
            self.handle_aborted_connection();
        }
    }

    /// `sapi_flush`: the SAPI's `flush`; a failure is a client abort.
    pub(crate) fn sapi_flush(&mut self) {
        if self.sapi.response_ended {
            return;
        }
        if self.sapi.sapi.flush_sends_headers() && !self.sapi.response.sent {
            self.sapi_send_headers();
        }
        if !self.sapi.sapi.flush() {
            self.handle_aborted_connection();
        }
    }

    /// fpm's `fastcgi_finish_request` (FrankenPHP's
    /// `frankenphp_finish_request`): php_output_end_all, php_header, then
    /// the SAPI completes the response. False when it already did.
    pub(crate) fn finish_response(&mut self) -> bool {
        if self.sapi.response_ended {
            return false;
        }
        self.output_end_all();
        self.sapi_send_headers();
        self.sapi.sapi.end_response();
        self.sapi.response_ended = true;
        true
    }

    /// FrankenPHP's `frankenphp_handle_request`: complete what the worker
    /// served so far, take its next request from the SAPI, set the
    /// request up (`frankenphp_worker_request_startup`: response, body,
    /// output layer, superglobals, the session's RINIT), call `handler`,
    /// and complete the request (`frankenphp_worker_request_shutdown`)
    /// before returning, so the client has its response while the worker
    /// runs on. The timer runs per request; waiting for one has none. An
    /// uncaught throwable from the handler is a fatal error that ends the
    /// worker (zend_exception_error + zend_bailout).
    pub(crate) fn worker_handle_request(&mut self, handler: &Value) -> Result<bool, Thrown> {
        if !self.sapi.sapi.worker() {
            let msg = "frankenphp_handle_request() called while not in worker mode";
            return Err(
                match builtins::Host::reflect_new_instance(
                    self,
                    b"RuntimeException",
                    vec![Value::str(msg)],
                    true,
                ) {
                    Ok(Value::Object(h)) => Thrown::Object(h),
                    Ok(_) => Thrown::error("RuntimeException", msg),
                    Err(t) => t,
                },
            );
        }
        self.worker_request_end()?;
        let Some(info) = self.sapi.sapi.next_request() else {
            return Ok(false);
        };
        self.worker_request_start(info)?;
        match self.call_callable_value(handler, Vec::new()) {
            Ok(_) => {}
            Err(Thrown::Exit(code)) => {
                self.worker_request_end()?;
                return Err(Thrown::Exit(code));
            }
            Err(t) if t.is_catchable() => {
                self.report_uncaught(t);
                return Err(Thrown::Exit(255));
            }
            Err(t) => return Err(t),
        }
        self.worker_request_end()?;
        Ok(true)
    }

    /// frankenphp_worker_request_startup for `info`.
    fn worker_request_start(&mut self, info: RequestInfo) -> Result<(), Thrown> {
        self.sapi.worker_in_request = true;
        self.sapi.response = Response {
            store: self.sapi.sapi.stores_headers(),
            code: self.sapi.sapi.initial_response_code(),
            ..Response::default()
        };
        self.sapi.response_ended = false;
        self.sapi.post_too_big = false;
        self.sapi.body = BodyState::default();
        builtins::main::reset_connection_status();
        self.set_request(info);
        builtins::zend::timeout::request_startup();
        self.check_post_size();
        let expose = self.ini_get(b"expose_php").unwrap_or_default();
        if crate::ini_bool(&expose) {
            let line = format!("X-Powered-By: PHP/{}", builtins::PHP_VERSION);
            let _ = sapi::header_op(self, sapi::HeaderOp::Replace(line.as_bytes(), 0));
        }
        self.output_reactivate();
        self.output_startup();
        self.seed_request_superglobals();
        builtins::zend::timeout::script_startup();
        self.sapi.startup = true;
        let r = builtins::request_activate(self);
        self.sapi.startup = false;
        r
    }

    /// frankenphp_worker_request_shutdown: flush the output buffers, the
    /// session's RSHUTDOWN, send the headers, complete the response,
    /// delete the uploads. Nothing when no request is being served.
    fn worker_request_end(&mut self) -> Result<(), Thrown> {
        if !std::mem::take(&mut self.sapi.worker_in_request) {
            return Ok(());
        }
        self.output_end_all();
        let r = builtins::request_deactivate(self);
        self.sapi_send_headers();
        if !self.sapi.response_ended {
            if !self.sapi.sapi.flush() {
                builtins::main::set_connection_aborted();
            }
            self.sapi.sapi.end_response();
            self.sapi.response_ended = true;
        }
        self.sapi.body.cleanup();
        builtins::zend::timeout::unset_timeout();
        r
    }

    /// php_handle_aborted_connection: the status becomes aborted, output
    /// stops, and unless `ignore_user_abort` is on the script bails out
    /// (shutdown functions and destructors still run, with output off).
    pub(crate) fn handle_aborted_connection(&mut self) {
        builtins::main::set_connection_aborted();
        self.output_set_disabled();
        let ignore = self.ini_get(b"ignore_user_abort").unwrap_or_default();
        if !builtins::ini_parse_bool(&ignore) && self.pending_throw.is_none() {
            self.pending_throw = Some(Thrown::Exit(self.sapi.exit_status));
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
        if !self.sapi.sapi.send_headers(&self.sapi.response) {
            // SAPI_HEADER_SEND_FAILED: not sent; php_output_header then
            // turns output off.
            self.sapi.response.sent = false;
            self.sapi.response.output_disabled = true;
        }
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
            } else if crate::ini_bool(&ini(self, b"register_argc_argv")) {
                // php_build_argv from the query string, split on '+'.
                self.raise_error(
                    builtins::E_DEPRECATED,
                    "Deriving $_SERVER['argv'] from the query string is deprecated. Configure register_argc_argv=0 to turn this message off",
                );
                let mut a = PhpArray::new();
                if let Some(q) = req.query_string.as_deref().filter(|q| !q.is_empty()) {
                    for part in q.split(|&b| b == b'+') {
                        a.push(Value::str(part));
                    }
                }
                let argc = a.len() as i64;
                server.insert(ArrayKey::from_bytes(b"argv"), Value::Array(a));
                server.insert(ArrayKey::from_bytes(b"argc"), Value::Int(argc));
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
        // ext/filter keeps the raw input (php_sapi_filter), then the
        // superglobals get filter.default's filter.
        let (mut get, mut post, mut cookie) = (get, post, cookie);
        let filtered_server = sapi::filtered_server_vars(&server);
        sapi::set_filter_input(&get, &post, &cookie, filtered_server, &env);
        for a in [&mut get, &mut post, &mut cookie] {
            sapi::apply_default_filter(self, a);
        }
        sapi::apply_default_filter_server(self, &mut server);
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
                    .unwrap_or_else(|| self.sapi.body.read_rest(&req.body));
                let (a, over) = sapi::post_form_vars(&data, max_vars);
                self.input_vars_exceeded(over, max_vars);
                post = a;
            }
            b"multipart/form-data" => {
                let lim = rfc1867::Limits::new(self, &rfc1867::BodyOptions::default());
                let len = req.declared_length();
                let read_before = self.sapi.body.read_bytes();
                let mut out = rfc1867::parse(
                    &mut self.sapi.body.reader(&req.body),
                    read_before,
                    &rfc1867::content_type_dup(ct),
                    len,
                    &lim,
                    &mut post,
                    files,
                );
                self.sapi.body.apply(&mut out);
                self.body_diags(std::mem::take(&mut out.diags));
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
    arr.insert(ArrayKey::from_bytes(k), Value::str(v));
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
            if let Some(Value::Array(da)) = dest.get_mut(&k) {
                autoglobal_merge(da, sa);
                continue;
            }
        }
        dest.insert(k.clone(), v.clone());
    }
}
