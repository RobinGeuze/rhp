//! The embedding contract of [`run_request`]: what a server sees through
//! its [`Sapi`].

use std::cell::RefCell;
use std::rc::Rc;

use rhp_sapi::{run_request, Outcome, Request, RequestInfo, Response, Sapi, Script};

// The engine's memory accounting (memory_get_usage(), memory_limit).
#[global_allocator]
static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;

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
            cwd: None,
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
    // php's E_COMPILE_ERROR, displayed through the SAPI.
    let (ev, o) = run("break;", &[], get());
    assert_eq!(o, Outcome::Exit(255));
    assert!(
        ev.iter()
            .any(|e| e.contains("Fatal error: 'break' not in the 'loop' or 'switch' context")),
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

/// The body bytes, joined across `ub_write` calls.
fn body(ev: &[String]) -> String {
    ev.iter().filter_map(|e| e.strip_prefix("body:")).collect()
}

#[test]
fn authorization_yields_php_auth_vars() {
    let info = RequestInfo {
        authorization: Some(b"Basic dXNlcjpwYXNz".to_vec()),
        ..get()
    };
    let (ev, _) = run(
        "echo $_SERVER['PHP_AUTH_USER'], ':', $_SERVER['PHP_AUTH_PW'];",
        &[],
        info,
    );
    assert_eq!(body(&ev), "user:pass");
}

/// The threading model: one engine per OS thread, several threads at once.
/// Each worker builds its own (non-`Send`) SAPI; per-thread engine state
/// (the SAPI name, ini, globals, constants, output) must not leak across
/// threads or across the requests one thread runs in turn.
#[test]
fn concurrent_requests_on_worker_threads_are_isolated() {
    let code = "function f() { static $n = 0; return ++$n; } \
                $leak = ($leak ?? 0) + 1; \
                if (!defined('K')) { define('K', $_GET['t']); } \
                ini_set('precision', '3'); \
                echo $_GET['t'], '/', $_GET['i'], ' ', f(), ' ', $leak, ' ', K, ' ', \
                     ini_get('precision') === '3' ? 'p' : '?', ' ', php_sapi_name();";
    let workers: Vec<_> = (0..4)
        .map(|t| {
            std::thread::spawn(move || {
                let mut bodies = Vec::new();
                for i in 0..20 {
                    let info = RequestInfo {
                        query_string: Some(format!("t={t}&i={i}").into_bytes()),
                        ..get()
                    };
                    let (ev, o) = run(code, &[], info);
                    assert_eq!(o, Outcome::Exit(0));
                    bodies.push(body(&ev));
                }
                bodies
            })
        })
        .collect();
    for (t, w) in workers.into_iter().enumerate() {
        let bodies = w.join().expect("worker panicked");
        for (i, b) in bodies.iter().enumerate() {
            assert_eq!(b, &format!("{t}/{i} 1 1 {t} p embed-test"));
        }
    }
}

/// A POST whose body is longer than its declared `Content-Length` (an
/// embedding server delivering a chunked body: `read_post` hands over every
/// byte while `content_length` stays 0).
fn lying_post(content_type: &str, body: Vec<u8>) -> RequestInfo {
    RequestInfo {
        method: Some(b"POST".to_vec()),
        content_type: Some(content_type.as_bytes().to_vec()),
        body,
        content_length: Some(0),
        response_code: 200,
        ..Default::default()
    }
}

const LIMIT_INI: [(&str, &str); 3] = [
    ("post_max_size", "100K"),
    ("display_errors", "0"),
    ("log_errors", "1"),
];

/// `"a=" . str_repeat('a', 1 MiB)`.
fn big_form() -> Vec<u8> {
    let mut d = b"a=".to_vec();
    d.extend(std::iter::repeat_n(b'a', 1024 * 1024));
    d
}

/// The multipart body of the GHSA-2cv3-554h-jpmh phpt.
fn multipart(value: &[u8]) -> Vec<u8> {
    let mut d = b"--b\r\nContent-Disposition: form-data; name=\"a\"\r\nContent-Type: application/octet-stream\r\n\r\n".to_vec();
    d.extend_from_slice(value);
    d.extend_from_slice(b"\r\n\r\n--b--\r\n");
    d
}

/// GHSA-2cv3-554h-jpmh (tests/basic/GHSA-2cv3-554h-jpmh_urlencoded.phpt):
/// `sapi_read_standard_form_data` stops after the 256 KiB block that takes
/// the bytes read past post_max_size, warns, and `$_POST` holds what was
/// read.
#[test]
fn urlencoded_body_longer_than_content_length_stops_at_post_max_size() {
    let small = lying_post("application/x-www-form-urlencoded", b"a=hello".to_vec());
    let (ev, _) = run("var_dump(strlen($_POST['a']));", &LIMIT_INI, small);
    assert!(!ev.iter().any(|e| e.starts_with("log:")), "{ev:?}");
    assert_eq!(body(&ev), "int(5)\n");

    let (ev, _) = run(
        "var_dump(strlen($_POST['a'] ?? '') < 100*1024+0x40000, strlen($_POST['a']), strlen(file_get_contents('php://input')));",
        &LIMIT_INI,
        lying_post("application/x-www-form-urlencoded", big_form()),
    );
    assert!(
        ev.contains(&"log:PHP Warning:  PHP Request Startup: Actual POST length does not match Content-Length, and exceeds 102400 bytes in Unknown on line 0".to_string()),
        "{ev:?}"
    );
    assert_eq!(body(&ev), "bool(true)\nint(262142)\nint(262144)\n");
}

/// GHSA-2cv3-554h-jpmh (tests/basic/GHSA-2cv3-554h-jpmh_multipart.phpt):
/// the multipart reader stops filling its buffer once more than
/// post_max_size bytes were read, and warns through `sapi_error` (no
/// "PHP Request Startup" prefix).
#[test]
fn multipart_body_longer_than_content_length_stops_at_post_max_size() {
    let ct = "multipart/form-data; boundary=b";
    let (ev, _) = run(
        "var_dump(strlen($_POST['a']));",
        &LIMIT_INI,
        lying_post(ct, multipart(b"hello")),
    );
    assert!(!ev.iter().any(|e| e.starts_with("log:")), "{ev:?}");
    assert_eq!(body(&ev), "int(7)\n");

    let (ev, _) = run(
        "var_dump(strlen($_POST['a'] ?? '') < 100*1024);",
        &LIMIT_INI,
        lying_post(ct, multipart(&vec![b'a'; 1024 * 1024])),
    );
    assert!(
        ev.contains(&"log:PHP Warning:  Actual POST length does not match Content-Length, and exceeds 102400 bytes in Unknown on line 0".to_string()),
        "{ev:?}"
    );
    assert_eq!(body(&ev), "bool(true)\n");
}

/// request_parse_body() reads through the same readers: the urlencoded
/// warning names the function (`php_error_docref`), the multipart one
/// (`sapi_error`) does not.
#[test]
fn request_parse_body_stops_at_post_max_size() {
    let ini = [("enable_post_data_reading", "0"), ("display_errors", "1")];
    let (ev, _) = run(
        "[$p] = request_parse_body(['post_max_size' => '100K']); var_dump(strlen($p['a']));",
        &ini,
        lying_post("application/x-www-form-urlencoded", big_form()),
    );
    assert_eq!(
        body(&ev),
        "\nWarning: request_parse_body(): Actual POST length does not match Content-Length, and exceeds 102400 bytes in Command line code on line 1\nint(262142)\n"
    );

    let (ev, _) = run(
        "[$p] = request_parse_body(['post_max_size' => '100K']); var_dump(strlen($p['a'] ?? '') < 100*1024);",
        &ini,
        lying_post(
            "multipart/form-data; boundary=b",
            multipart(&vec![b'a'; 1024 * 1024]),
        ),
    );
    assert_eq!(
        body(&ev),
        "\nWarning: Actual POST length does not match Content-Length, and exceeds 102400 bytes in Command line code on line 1\nbool(true)\n"
    );
}
