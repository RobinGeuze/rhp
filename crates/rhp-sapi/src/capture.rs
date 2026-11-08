//! A SAPI that keeps a request's output in memory, for tests and tools.

use std::cell::RefCell;
use std::rc::Rc;

use crate::{run_request, CliSapi, Outcome, Request, Response, Sapi};

/// What a request under a [`CaptureSapi`] produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Captured {
    /// The body and `STDOUT` stream writes, in order (php-cli's stdout).
    pub stdout: Vec<u8>,
    /// `display_errors=stderr` output, the `STDERR` stream and engine
    /// refusals.
    pub stderr: Vec<u8>,
    /// `log_errors` / `error_log()` lines.
    pub log: Vec<Vec<u8>>,
    /// The response status when the headers went out.
    pub code: i64,
    /// The header lines when the headers went out (empty for php-cli,
    /// which stores none).
    pub headers: Vec<Vec<u8>>,
}

/// A SAPI that captures everything a request writes ([`Captured`]).
/// [`CaptureSapi::cli`] behaves as php-cli (the name `cli`, so `STDIN`,
/// `STDOUT` and `STDERR` exist; no header store), [`CaptureSapi::web`] as a
/// web server of the given name. Unlike php-cli it never ends the process
/// (the hard memory limit ends just the request).
///
/// Clones share what was captured, so a caller can keep one and hand
/// another to [`run_request`]; [`CaptureSapi::run`] does both.
#[derive(Clone)]
pub struct CaptureSapi {
    name: Rc<str>,
    ini: Vec<(&'static str, &'static str)>,
    entries: Vec<(&'static str, &'static str)>,
    stores_headers: bool,
    initial_code: i64,
    sink: Rc<RefCell<Captured>>,
}

impl CaptureSapi {
    /// php-cli's name, ini defaults and header semantics ([`CliSapi`]).
    pub fn cli() -> CaptureSapi {
        CaptureSapi {
            name: CliSapi.name().into(),
            ini: CliSapi.ini_defaults(),
            entries: CliSapi.ini_entries(),
            stores_headers: CliSapi.stores_headers(),
            initial_code: CliSapi.initial_response_code(),
            sink: Rc::default(),
        }
    }

    /// A web server named `name`: headers stored, status 200 to start with.
    pub fn web(name: &str) -> CaptureSapi {
        CaptureSapi {
            name: name.into(),
            ini: Vec::new(),
            entries: Vec::new(),
            stores_headers: true,
            initial_code: 200,
            sink: Rc::default(),
        }
    }

    /// Run `req` under this SAPI; its outcome and what it wrote.
    pub fn run(self, req: Request) -> (Outcome, Captured) {
        let sink = self.sink.clone();
        let outcome = run_request(Box::new(self), req);
        (outcome, sink.take())
    }

    /// What has been captured so far.
    pub fn captured(&self) -> Captured {
        self.sink.borrow().clone()
    }
}

impl Sapi for CaptureSapi {
    fn name(&self) -> &str {
        &self.name
    }

    fn ini_defaults(&self) -> Vec<(&'static str, &'static str)> {
        self.ini.clone()
    }

    fn ini_entries(&self) -> Vec<(&'static str, &'static str)> {
        self.entries.clone()
    }

    fn stores_headers(&self) -> bool {
        self.stores_headers
    }

    fn initial_response_code(&self) -> i64 {
        self.initial_code
    }

    fn ub_write(&mut self, data: &[u8]) -> bool {
        self.sink.borrow_mut().stdout.extend_from_slice(data);
        true
    }

    fn send_headers(&mut self, r: &Response) {
        let mut s = self.sink.borrow_mut();
        s.code = r.code;
        s.headers = r.headers.clone();
    }

    fn log_message(&mut self, msg: &[u8]) {
        self.sink.borrow_mut().log.push(msg.to_vec());
    }

    fn write_stderr(&mut self, data: &[u8]) {
        self.sink.borrow_mut().stderr.extend_from_slice(data);
    }

    fn write_raw_stdout(&mut self, data: &[u8]) {
        self.sink.borrow_mut().stdout.extend_from_slice(data);
    }
}
