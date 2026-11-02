//! The embedding contract of [`run_request`]: what a server sees through
//! its [`Sapi`].

use std::cell::RefCell;
use std::rc::Rc;

use rhp_sapi::{run_request, Outcome, Request, RequestInfo, Response, Sapi, Script};

/// Records every call the engine makes, in order.
#[derive(Clone, Default)]
struct Recorder {
    events: Rc<RefCell<Vec<String>>>,
}

impl Sapi for Recorder {
    fn name(&self) -> &str {
        "embed-test"
    }

    fn ub_write(&mut self, data: &[u8]) {
        let s = String::from_utf8_lossy(data).into_owned();
        self.events.borrow_mut().push(format!("body:{s}"));
    }

    fn send_headers(&mut self, r: &Response) {
        self.events.borrow_mut().push(format!("headers:{}", r.code));
    }

    fn write_stderr(&mut self, data: &[u8]) {
        let s = String::from_utf8_lossy(data).into_owned();
        self.events.borrow_mut().push(format!("stderr:{s}"));
    }

    fn log_message(&mut self, msg: &[u8]) {
        let s = String::from_utf8_lossy(msg).into_owned();
        self.events.borrow_mut().push(format!("log:{s}"));
    }
}

fn run(code: &str, ini: &[(&str, &str)], info: RequestInfo) -> (Vec<String>, Outcome) {
    let rec = Recorder::default();
    let outcome = run_request(
        Box::new(rec.clone()),
        Request {
            script: Script::Code(code.to_string()),
            ini: ini
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            args: Vec::new(),
            info: Some(info),
            php_binary: None,
        },
    );
    let events = rec.events.borrow().clone();
    (events, outcome)
}

fn get() -> RequestInfo {
    RequestInfo {
        method: Some(b"GET".to_vec()),
        response_code: 200,
        ..Default::default()
    }
}

#[test]
fn headers_go_out_once_before_the_body() {
    let (ev, o) = run("http_response_code(201); echo 'a'; echo 'b';", &[], get());
    assert_eq!(o, Outcome::Exit(0));
    assert_eq!(ev, ["headers:201", "body:a", "body:b"]);
}

#[test]
fn headers_go_out_at_the_end_without_output() {
    let (ev, _) = run("header('Location: /x');", &[], get());
    assert_eq!(ev, ["headers:302"]);
}

#[test]
fn a_header_callback_throw_without_output_is_uncaught() {
    let (ev, o) = run(
        "header_register_callback(function () { throw new Exception('cb'); });",
        &[("display_errors", "0"), ("log_errors", "1")],
        get(),
    );
    assert_eq!(o, Outcome::Exit(255));
    assert!(
        ev.iter()
            .any(|e| e.starts_with("log:PHP Fatal error:  Uncaught Exception: cb")),
        "{ev:?}"
    );
}

#[test]
fn compile_errors_reach_the_sapi_not_the_process() {
    let (ev, o) = run("break;", &[], get());
    assert_eq!(o, Outcome::Exit(255));
    assert!(
        ev.iter().any(|e| e.starts_with("stderr:PHP Fatal error")),
        "{ev:?}"
    );
}

#[test]
fn cli_only_constants_are_absent() {
    let (ev, _) = run(
        "var_dump(defined('STDIN'), function_exists('cli_set_process_title'));",
        &[],
        get(),
    );
    assert_eq!(ev[1], "body:bool(false)\nbool(false)\n");
}

#[test]
fn post_bodies_over_post_max_size_are_not_parsed() {
    let info = RequestInfo {
        method: Some(b"POST".to_vec()),
        content_type: Some(b"application/x-www-form-urlencoded".to_vec()),
        body: b"a=1&b=2".to_vec(),
        response_code: 200,
        ..Default::default()
    };
    let (ev, _) = run(
        "var_dump($_POST, file_get_contents('php://input'));",
        &[
            ("post_max_size", "4"),
            ("display_errors", "0"),
            ("log_errors", "1"),
        ],
        info,
    );
    assert!(
        ev.contains(&"log:PHP Warning:  PHP Request Startup: POST Content-Length of 7 bytes exceeds the limit of 4 bytes in Unknown on line 0".to_string()),
        "{ev:?}"
    );
    assert_eq!(
        ev.last().unwrap(),
        "body:array(0) {\n}\nstring(7) \"a=1&b=2\"\n"
    );
}
