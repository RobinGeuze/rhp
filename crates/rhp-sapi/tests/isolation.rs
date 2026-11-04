//! Requests run one after another on one thread must not see each other's
//! state: each [`run_request`] builds a fresh engine and resets the
//! builtins' thread-local state.

use rhp_sapi::{CaptureSapi, Captured, Outcome, Request, RequestInfo, Script};

// The engine's memory accounting (memory_get_usage(), memory_limit).
#[global_allocator]
static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;

fn run(code: &str, info: RequestInfo) -> (Captured, Outcome) {
    let req = Request {
        info: Some(info),
        ..Request::new(Script::Code(code.to_string()))
    };
    let (outcome, cap) = CaptureSapi::web("embed-test").run(req);
    (cap, outcome)
}

fn body(c: &Captured) -> String {
    String::from_utf8_lossy(&c.stdout).into_owned()
}

#[test]
fn sequential_requests_are_isolated() {
    let first = r#"
        ini_set('precision', '3');
        date_default_timezone_set('Asia/Tokyo');
        header('X-First: 1');
        http_response_code(404);
        setcookie('a', 'b');
        $GLOBALS['leak'] = 1;
        define('LEAKED', 1);
        function leaked_fn() {}
        class LeakedClass {}
        $f = fopen('php://memory', 'w+');
        mt_srand(1);
        echo $_GET['q'], ' ', php_sapi_name(), ' ', 1/3, "\n";
    "#;
    let info = RequestInfo {
        query_string: Some(b"q=one".to_vec()),
        method: Some(b"GET".to_vec()),
        response_code: 200,
        ..Default::default()
    };
    let (c1, o1) = run(first, info);
    assert_eq!(o1, Outcome::Exit(0));
    assert_eq!(body(&c1), "one embed-test 0.333\n");
    assert_eq!(c1.code, 404);
    assert!(c1.headers.iter().any(|h| h == b"X-First: 1"));

    let second = r#"
        var_dump(
            $_GET,
            ini_get('precision'),
            date_default_timezone_get(),
            isset($leak),
            defined('LEAKED'),
            function_exists('leaked_fn'),
            class_exists('LeakedClass'),
            http_response_code(),
            headers_list(),
        );
    "#;
    let info = RequestInfo {
        query_string: Some(b"r=two".to_vec()),
        method: Some(b"GET".to_vec()),
        response_code: 200,
        ..Default::default()
    };
    let (c2, o2) = run(second, info);
    assert_eq!(o2, Outcome::Exit(0));
    let powered = format!("X-Powered-By: PHP/{}", rhp_builtins::version::PHP_VERSION);
    let want = r#"array(1) {
  ["r"]=>
  string(3) "two"
}
string(2) "14"
string(3) "UTC"
bool(false)
bool(false)
bool(false)
bool(false)
int(200)
array(1) {
  [0]=>
  string(LEN) "POWERED"
}
"#
    .replace("LEN", &powered.len().to_string())
    .replace("POWERED", &powered);
    assert_eq!(body(&c2), want);
    assert_eq!(c2.code, 200);
    assert!(!c2.headers.iter().any(|h| h == b"X-First: 1"));
}

#[test]
fn sapi_name_follows_the_request() {
    let (c, _) = run(
        "echo PHP_SAPI, ' ', php_sapi_name();",
        RequestInfo::default(),
    );
    assert_eq!(body(&c), "embed-test embed-test");
    // A CLI-style request on the same thread afterwards (no SAPI request
    // input): the name is the new SAPI's again.
    let script = Script::Code("echo php_sapi_name();".to_string());
    let (_, out) = CaptureSapi::web("other").run(Request::new(script));
    assert_eq!(out.stdout, b"other");
}
