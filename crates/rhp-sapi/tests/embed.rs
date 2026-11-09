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
    /// The client goes away after this many body writes.
    writes_left: Option<usize>,
    /// Ask for php-fpm's functions ([`Sapi::server_functions`]).
    server: bool,
}

impl Sapi for Recorder {
    fn name(&self) -> &str {
        "embed-test"
    }

    fn ub_write(&mut self, data: &[u8]) -> bool {
        if let Some(n) = &mut self.writes_left {
            if *n == 0 {
                return false;
            }
            *n -= 1;
        }
        let s = String::from_utf8_lossy(data).into_owned();
        self.events.borrow_mut().push(format!("body:{s}"));
        true
    }

    fn send_headers(&mut self, r: &Response) -> bool {
        self.events.borrow_mut().push(format!("headers:{}", r.code));
        true
    }

    fn write_stderr(&mut self, data: &[u8]) {
        let s = String::from_utf8_lossy(data).into_owned();
        self.events.borrow_mut().push(format!("stderr:{s}"));
    }

    fn log_message(&mut self, msg: &[u8]) {
        let s = String::from_utf8_lossy(msg).into_owned();
        self.events.borrow_mut().push(format!("log:{s}"));
    }

    fn flush(&mut self) -> bool {
        if self.server {
            self.events.borrow_mut().push("flush".to_string());
        }
        true
    }

    fn server_functions(&self) -> bool {
        self.server
    }

    fn end_response(&mut self) {
        self.events.borrow_mut().push("end".to_string());
    }
}

fn run(code: &str, ini: &[(&str, &str)], info: RequestInfo) -> (Vec<String>, Outcome) {
    run_with(Recorder::default(), code, ini, info)
}

fn run_with(
    rec: Recorder,
    code: &str,
    ini: &[(&str, &str)],
    info: RequestInfo,
) -> (Vec<String>, Outcome) {
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
            ini_files: None,
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
    let (ev, o) = run("break;", &[("html_errors", "0")], get());
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
/// `sapi_read_standard_form_data` stops after the 16 KiB block that takes
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
    assert_eq!(body(&ev), "bool(true)\nint(114686)\nint(114688)\n");
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
    let ini = [
        ("enable_post_data_reading", "0"),
        ("display_errors", "1"),
        ("html_errors", "0"),
    ];
    let (ev, _) = run(
        "[$p] = request_parse_body(['post_max_size' => '100K']); var_dump(strlen($p['a']));",
        &ini,
        lying_post("application/x-www-form-urlencoded", big_form()),
    );
    assert_eq!(
        body(&ev),
        "\nWarning: request_parse_body(): Actual POST length does not match Content-Length, and exceeds 102400 bytes in Command line code on line 1\nint(114686)\n"
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

/// A failed body write is a client abort (php_handle_aborted_connection):
/// the script stops at that write, shutdown functions still run with
/// output off, and they see the aborted status.
#[test]
fn a_failed_write_aborts_the_script() {
    let rec = Recorder {
        writes_left: Some(2),
        ..Recorder::default()
    };
    let code = "register_shutdown_function(function () { \
                error_log('shutdown ' . connection_status() . connection_aborted()); echo 'lost'; }); \
                for ($i = 0; $i < 5; $i++) { echo $i; error_log(\"i=$i\"); }";
    let (ev, o) = run_with(rec, code, &[("log_errors", "1")], get());
    assert_eq!(o, Outcome::Exit(0));
    assert_eq!(
        ev,
        [
            "headers:200",
            "body:0",
            "log:i=0",
            "body:1",
            "log:i=1",
            "log:shutdown 11"
        ]
    );
}

/// With `ignore_user_abort` the script runs on; output stays off.
#[test]
fn ignore_user_abort_keeps_the_script_running() {
    let rec = Recorder {
        writes_left: Some(1),
        ..Recorder::default()
    };
    let code = "ignore_user_abort(true); for ($i = 0; $i < 3; $i++) { echo $i; \
                error_log(\"i=$i \" . connection_status()); }";
    let (ev, o) = run_with(rec, code, &[("log_errors", "1")], get());
    assert_eq!(o, Outcome::Exit(0));
    assert_eq!(
        ev,
        [
            "headers:200",
            "body:0",
            "log:i=0 0",
            "log:i=1 1",
            "log:i=2 1"
        ]
    );
}

const TIMEOUT_INI: [(&str, &str); 2] = [("max_execution_time", "1"), ("html_errors", "0")];

/// max_execution_time under an embedding SAPI: each thread has its own
/// wall-clock timer (php's ZTS timers), so a request that runs out ends
/// with the fatal error while a request on another thread runs on.
#[test]
fn max_execution_time_is_per_thread() {
    let slow = std::thread::spawn(|| {
        // No limit: busy for 1.5 s while the other thread times out.
        run(
            "$t = microtime(true); while (microtime(true) - $t < 1.5) {} echo 'done';",
            &[("max_execution_time", "0")],
            get(),
        )
    });
    let (ev, o) = run("echo 'start'; while (true) {}", &TIMEOUT_INI, get());
    assert_eq!(o, Outcome::Exit(255));
    let b = body(&ev);
    assert!(b.starts_with("start\nFatal error: Maximum execution time of 1 second exceeded in Command line code on line 1"), "{b}");
    let (ev, o) = slow.join().expect("worker panicked");
    assert_eq!((body(&ev), o), ("done".to_string(), Outcome::Exit(0)));
}

/// A request that ends before its deadline leaves nothing armed for the
/// next one on the thread; set_time_limit re-arms from now.
#[test]
fn a_timer_ends_with_its_request() {
    let (ev, o) = run("echo 'quick';", &TIMEOUT_INI, get());
    assert_eq!((body(&ev), o), ("quick".to_string(), Outcome::Exit(0)));
    let (ev, o) = run(
        "$t = microtime(true); while (microtime(true) - $t < 1.3) {} echo 'ok';",
        &[("max_execution_time", "0")],
        get(),
    );
    assert_eq!((body(&ev), o), ("ok".to_string(), Outcome::Exit(0)));
    let (ev, o) = run(
        "set_time_limit(0); $t = microtime(true); while (microtime(true) - $t < 1.3) {} echo 'unlimited';",
        &TIMEOUT_INI,
        get(),
    );
    assert_eq!((body(&ev), o), ("unlimited".to_string(), Outcome::Exit(0)));
}

/// max_input_time covers request startup only: the script then runs
/// under max_execution_time (php_execute_script re-arms the timer). As in
/// php, a max_execution_time of 0 leaves the input timer running
/// (zend_set_timeout(0) doesn't disarm), so it fires naming 0 seconds.
#[test]
fn max_input_time_gives_way_to_max_execution_time() {
    let busy = "$t = microtime(true); while (microtime(true) - $t < 1.3) {} echo 'ok';";
    let (ev, o) = run(
        busy,
        &[("max_input_time", "1"), ("max_execution_time", "5")],
        get(),
    );
    assert_eq!((body(&ev), o), ("ok".to_string(), Outcome::Exit(0)));
    // (Request startup must take less than the input time, or the timer
    // fires before the script's start clears it.)
    let (ev, o) = run(
        "$t = microtime(true); while (microtime(true) - $t < 3.5) {} echo 'ok';",
        &[
            ("max_input_time", "2"),
            ("max_execution_time", "0"),
            ("html_errors", "0"),
        ],
        get(),
    );
    assert_eq!(o, Outcome::Exit(255));
    assert!(
        body(&ev).contains("Maximum execution time of 0 seconds"),
        "{ev:?}"
    );
    let (ev, o) = run(
        "while (true) {}",
        &[
            ("max_input_time", "60"),
            ("max_execution_time", "1"),
            ("html_errors", "0"),
        ],
        get(),
    );
    assert_eq!(o, Outcome::Exit(255));
    assert!(
        body(&ev).contains("Maximum execution time of 1 second"),
        "{ev:?}"
    );
}

fn server() -> Recorder {
    Recorder {
        server: true,
        ..Recorder::default()
    }
}

/// php-fpm's functions exist only for a SAPI that asks for them.
#[test]
fn server_functions_are_opt_in() {
    let code = "var_dump(function_exists('fastcgi_finish_request'), function_exists('getallheaders'), php_sapi_name());";
    let (ev, _) = run(code, &[], get());
    assert_eq!(
        body(&ev),
        "bool(false)\nbool(false)\nstring(10) \"embed-test\"\n"
    );
    let (ev, _) = run_with(server(), code, &[], get());
    assert_eq!(
        body(&ev),
        "bool(true)\nbool(true)\nstring(10) \"embed-test\"\n"
    );
}

/// fastcgi_finish_request(): the output buffers are flushed, the headers
/// sent and the response completed; the script and its shutdown functions
/// run on with their output discarded, and the final flush is skipped.
#[test]
fn fastcgi_finish_request_completes_the_response_early() {
    let code = "register_shutdown_function(function () { echo 'gone'; error_log('shutdown'); }); \
                ob_start(); echo 'a'; header('X-A: 1'); \
                $r = fastcgi_finish_request(); echo 'lost'; flush(); \
                error_log('after ' . var_export($r, true) . ' ' . var_export(fastcgi_finish_request(), true) . ' ' . ob_get_level());";
    let (ev, o) = run_with(server(), code, &[("log_errors", "1")], get());
    assert_eq!(o, Outcome::Exit(0));
    assert_eq!(
        ev,
        [
            "headers:200",
            "body:a",
            "end",
            "log:after true false 0",
            "log:shutdown"
        ]
    );
}

/// An embedding SAPI starts from php's defaults, not php-cli's.
#[test]
fn an_embedding_sapi_gets_php_defaults() {
    let (ev, _) = run(
        "echo ini_get('html_errors'), ini_get('implicit_flush'), ini_get('max_execution_time'), ini_get('display_errors');",
        &[],
        get(),
    );
    assert_eq!(body(&ev), "10301");
}

/// A streamed body (`RequestInfo::body_reader`) handing out a few bytes
/// per read and logging each read among the SAPI events.
struct Trickle {
    data: Vec<u8>,
    pos: usize,
    events: Rc<RefCell<Vec<String>>>,
}

impl std::io::Read for Trickle {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(7).min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        if n > 0 {
            self.events.borrow_mut().push("read".to_string());
        }
        Ok(n)
    }
}

/// `info` with its body streamed instead.
fn streamed(rec: &Recorder, mut info: RequestInfo) -> RequestInfo {
    info.body_reader = Some(Box::new(Trickle {
        data: std::mem::take(&mut info.body),
        pos: 0,
        events: rec.events.clone(),
    }));
    info
}

fn post(content_type: &str, body: &[u8]) -> RequestInfo {
    RequestInfo {
        method: Some(b"POST".to_vec()),
        content_type: Some(content_type.as_bytes().to_vec()),
        content_length: Some(body.len() as i64),
        body: body.to_vec(),
        response_code: 200,
        ..Default::default()
    }
}

/// A streamed body reads as the in-memory one: urlencoded, multipart
/// with an upload, and php://input.
#[test]
fn a_streamed_body_reads_like_a_buffered_one() {
    let mut upload = b"--b\r\nContent-Disposition: form-data; name=\"f\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\n".to_vec();
    upload.extend(std::iter::repeat_n(b'x', 20_000));
    upload.extend_from_slice(
        b"\r\n--b\r\nContent-Disposition: form-data; name=\"k\"\r\n\r\nv\r\n--b--\r\n",
    );
    let cases = [
        (
            post("application/x-www-form-urlencoded", b"a=1&b[]=2"),
            "echo json_encode($_POST), file_get_contents('php://input');",
        ),
        (
            post("multipart/form-data; boundary=b", &upload),
            "echo $_POST['k'], ' ', $_FILES['f']['size'], ' ', filesize($_FILES['f']['tmp_name']), ' ', strlen(file_get_contents('php://input'));",
        ),
        (
            post("text/plain", b"raw body"),
            "echo file_get_contents('php://input'), '|', file_get_contents('php://input');",
        ),
    ];
    for (info, code) in cases {
        let buffered = RequestInfo {
            body: info.body.clone(),
            ..post(
                std::str::from_utf8(info.content_type.as_deref().unwrap()).unwrap(),
                &info.body,
            )
        };
        let (want, _) = run(code, &[], buffered);
        let rec = Recorder::default();
        let info = streamed(&rec, info);
        let (got, o) = run_with(rec, code, &[], info);
        assert_eq!(o, Outcome::Exit(0));
        let got: Vec<_> = got.into_iter().filter(|e| e != "read").collect();
        assert_eq!(got, want);
    }
}

/// With enable_post_data_reading off nothing is read until the script
/// asks (php://input, request_parse_body()).
#[test]
fn a_streamed_body_is_read_when_the_script_asks() {
    let rec = Recorder::default();
    let info = streamed(&rec, post("application/x-www-form-urlencoded", b"a=1&b=2"));
    let (ev, _) = run_with(
        rec,
        "echo 'before'; [$p] = request_parse_body(); echo json_encode($p);",
        &[("enable_post_data_reading", "0")],
        info,
    );
    assert_eq!(
        ev,
        [
            "headers:200",
            "body:before",
            "read",
            "body:{\"a\":\"1\",\"b\":\"2\"}"
        ]
    );
}

/// The GHSA-2cv3-554h-jpmh limits hold for a streamed body that is longer
/// than its Content-Length.
#[test]
fn a_streamed_body_stops_at_post_max_size() {
    let rec = Recorder::default();
    let info = streamed(
        &rec,
        lying_post("application/x-www-form-urlencoded", big_form()),
    );
    let (ev, _) = run_with(rec, "var_dump(strlen($_POST['a']));", &LIMIT_INI, info);
    assert!(
        ev.contains(&"log:PHP Warning:  PHP Request Startup: Actual POST length does not match Content-Length, and exceeds 102400 bytes in Unknown on line 0".to_string()),
        "{ev:?}"
    );
    assert_eq!(body(&ev), "int(114686)\n");
}

/// A FrankenPHP-style worker: requests come from a queue; every SAPI call
/// is recorded.
struct Worker {
    events: Rc<RefCell<Vec<String>>>,
    queue: std::collections::VecDeque<RequestInfo>,
}

impl Sapi for Worker {
    fn name(&self) -> &str {
        "embed-worker"
    }

    fn ub_write(&mut self, data: &[u8]) -> bool {
        let s = String::from_utf8_lossy(data).into_owned();
        self.events.borrow_mut().push(format!("body:{s}"));
        true
    }

    fn send_headers(&mut self, r: &Response) -> bool {
        let own: Vec<String> = r
            .headers
            .iter()
            .map(|h| String::from_utf8_lossy(h).into_owned())
            .filter(|h| h.starts_with("X-"))
            .collect();
        self.events
            .borrow_mut()
            .push(format!("headers:{} {}", r.code, own.join(",")));
        true
    }

    fn log_message(&mut self, msg: &[u8]) {
        let s = String::from_utf8_lossy(msg).into_owned();
        self.events.borrow_mut().push(format!("log:{s}"));
    }

    fn ini_defaults(&self) -> Vec<(&'static str, &'static str)> {
        vec![("expose_php", "0"), ("html_errors", "0")]
    }

    fn server_functions(&self) -> bool {
        true
    }

    fn worker(&self) -> bool {
        true
    }

    fn next_request(&mut self) -> Option<RequestInfo> {
        self.events.borrow_mut().push("next".to_string());
        self.queue.pop_front()
    }

    fn end_response(&mut self) {
        self.events.borrow_mut().push("end".to_string());
    }
}

fn run_worker(code: &str, queue: Vec<RequestInfo>) -> (Vec<String>, Outcome) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sapi = Worker {
        events: events.clone(),
        queue: queue.into(),
    };
    let outcome = run_request(
        Box::new(sapi),
        Request {
            script: Script::Code(code.to_string()),
            ini: vec![("log_errors".into(), "1".into())],
            args: Vec::new(),
            info: Some(get()),
            php_binary: None,
            cwd: None,
            ini_files: None,
        },
    );
    let ev = events.borrow().clone();
    (ev, outcome)
}

/// Worker mode: the script boots once, keeps its state across requests,
/// and each request gets its own superglobals, headers, output buffers
/// and response, completed before the worker runs on.
#[test]
fn a_worker_serves_requests_with_one_bootstrapped_script() {
    let code = "$n = 0; echo 'boot'; \
        $handler = function () use (&$n) { $n++; ob_start(); header('X-N: ' . $n); \
            echo $_GET['q'] ?? '-', ' ', json_encode($_POST), ' ', $n, ' ', http_response_code(); \
            if ($n == 2) { http_response_code(404); } }; \
        while (frankenphp_handle_request($handler)) { error_log('terminate ' . $n . ' ' . (headers_sent() ? 'sent' : '?')); } \
        echo 'gone';";
    let q = RequestInfo {
        query_string: Some(b"q=one".to_vec()),
        ..get()
    };
    let p = post("application/x-www-form-urlencoded", b"a=b");
    let (ev, o) = run_worker(code, vec![q, p]);
    assert_eq!(o, Outcome::Exit(0));
    assert_eq!(
        ev,
        [
            "headers:200 ",
            "body:boot",
            "end",
            "next",
            "headers:200 X-N: 1",
            "body:one [] 1 200",
            "end",
            "log:terminate 1 sent",
            "next",
            "headers:404 X-N: 2",
            "body:- {\"a\":\"b\"} 2 200",
            "end",
            "log:terminate 2 sent",
            "next",
        ]
    );
}

/// An uncaught exception in the handler is a fatal error that ends the
/// worker (FrankenPHP bails out); outside worker mode the function throws.
#[test]
fn a_worker_handler_exception_ends_the_worker() {
    let code = "try { while (frankenphp_handle_request(function () { throw new Exception('boom'); })) {} } \
        catch (Throwable $e) { echo 'caught'; }";
    let (ev, o) = run_worker(code, vec![get(), get()]);
    assert_eq!(o, Outcome::Exit(255));
    assert!(!ev.iter().any(|e| e == "body:caught"), "{ev:?}");
    assert_eq!(ev.iter().filter(|e| *e == "next").count(), 1, "{ev:?}");
    assert!(
        ev.iter()
            .any(|e| e.contains("PHP Fatal error:  Uncaught Exception: boom")),
        "{ev:?}"
    );

    let (ev, _) = run_with(
        server(),
        "try { frankenphp_handle_request(fn () => 1); } catch (RuntimeException $e) { echo $e->getMessage(); }",
        &[],
        get(),
    );
    assert_eq!(
        body(&ev),
        "frankenphp_handle_request() called while not in worker mode"
    );
}

/// Each worker request has its own session (the session's RSHUTDOWN
/// writes it when the request completes, RINIT starts the next fresh) and
/// its own uploads.
#[test]
fn a_worker_request_has_its_own_session_and_uploads() {
    let dir = std::env::temp_dir().join(format!("rhp-worker-sess-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let code = format!(
        "ini_set('session.save_path', '{}'); $files = []; \
         while (frankenphp_handle_request(function () use (&$files) {{ \
            session_id('abc'); session_start(); $_SESSION['n'] = ($_SESSION['n'] ?? 0) + 1; \
            echo session_status(), ' ', $_SESSION['n'], ' ', count($_FILES); \
            foreach ($_FILES as $f) {{ $files[] = $f['tmp_name']; echo ' ', is_uploaded_file($f['tmp_name']) ? 'up' : 'no'; }} \
         }})) {{ echo session_status(); error_log('after ' . session_status() . ' ' . implode(',', array_map('file_exists', $files))); }}",
        dir.display()
    );
    let upload = b"--b\r\nContent-Disposition: form-data; name=\"f\"; filename=\"a.txt\"\r\n\r\nxyz\r\n--b--\r\n";
    let (ev, o) = run_worker(
        &code,
        vec![post("multipart/form-data; boundary=b", upload), get()],
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(o, Outcome::Exit(0));
    // The body writes of each request joined, and the log lines.
    let mut seen: Vec<String> = Vec::new();
    for e in &ev {
        if let Some(b) = e.strip_prefix("body:") {
            match seen.last_mut().filter(|l| !l.starts_with("log:")) {
                Some(last) => last.push_str(b),
                None => seen.push(b.to_string()),
            }
        } else if e.starts_with("log:") {
            seen.push(e.clone());
        }
    }
    assert_eq!(seen, ["2 1 1 up", "log:after 1 ", "2 2 0", "log:after 1 "]);
}
