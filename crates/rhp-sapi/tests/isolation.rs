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

// Builtin per-request state (ini overrides, default timezone, precision,
// strtok / preg / json state, putenv) must not leak from one CLI request to
// the next on the same thread, and putenv must not touch the process
// environment.

const DIRTY: &[u8] = br#"<?php
ini_set('precision', 5);
ini_set('serialize_precision', 3);
ini_set('arg_separator.output', ';');
ini_set('allow_url_fopen', '0');
error_reporting(0);
date_default_timezone_set('America/New_York');
strtok('a b c', ' ');
preg_replace('/(?:\D+|<\d+>)*[!?]/', '', 'foobar foobar foobar');
echo preg_last_error(), ' ';
json_decode('{');
putenv('RHP_REQUEST_STATE_TEST=dirty');
echo error_reporting(), ' dirty';
"#;

const CHECK: &[u8] = br#"<?php
echo 1/3, ' ', json_encode(0.1), ' ', http_build_query(['a' => 1, 'b' => 2]), ' ',
    ini_get('precision'), ' ', error_reporting(), ' ',
    date_default_timezone_get(), ' ', var_export(strtok(' '), true), ' ',
    preg_last_error(), ' ', json_last_error(), ' ',
    var_export(getenv('RHP_REQUEST_STATE_TEST'), true);
"#;

const CLEAN: &str = "0.33333333333333 0.1 a=1&b=2 14 30719 UTC false 0 0 false";

fn run_source(source: &[u8]) -> String {
    let script = Script::Source {
        path: "t.php".to_string(),
        bytes: source.to_vec(),
    };
    let (_, out) = CaptureSapi::cli().run(Request::new(script));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn vm_state_does_not_leak_between_engines() {
    let clean = run_source(CHECK);
    assert_eq!(clean, CLEAN);
    assert_eq!(run_source(DIRTY), "2 0 dirty");
    assert_eq!(run_source(CHECK), clean);
    assert!(std::env::var_os("RHP_REQUEST_STATE_TEST").is_none());
}

#[test]
fn clean_state_matches_php_defaults() {
    // Exactly what `php -n` prints (error_reporting = E_ALL = 30719; the
    // phpt harness passes its 32767 with -d, which rhp honours).
    assert_eq!(run_source(CHECK), CLEAN);
}

/// php drops gmp's random state at the end of a request (its RSHUTDOWN): a
/// seed does not carry over to the next engine.
#[test]
fn gmp_random_state_does_not_leak() {
    let seeded = run_source(
        b"<?php gmp_random_seed(42); echo gmp_random_bits(100), ' ', gmp_random_bits(100);",
    );
    let (first, second) = seeded.split_once(' ').expect("two numbers");
    assert_eq!(
        run_source(b"<?php gmp_random_seed(42); echo gmp_random_bits(100);"),
        first
    );
    assert_ne!(run_source(b"<?php echo gmp_random_bits(100);"), second);
}

#[test]
fn built_in_classes_are_fresh_in_every_request() {
    // The first request links the prelude's classes, the second keeps a
    // copy of them for the thread, the third starts from that copy.
    let code = r#"
        var_dump(class_exists('LeakedAlias'));
        class_alias('ArrayIterator', 'LeakedAlias');
        // Hoisted: a copy of the last request's class would redeclare it.
        class LeakedSub extends RuntimeException {}
        $e = new LeakedSub('m', 3);
        echo get_class($e), ' ', $e->getMessage(), ' ', $e->getCode(), ' ',
            get_parent_class($e), "\n";
        echo implode(',', class_implements(new LeakedAlias([1]))), "\n";
        echo count(get_declared_classes()), ' ', count(get_declared_interfaces()), "\n";
    "#;
    let outputs: Vec<String> = (0..3)
        .map(|_| {
            let (c, o) = run(code, RequestInfo::default());
            assert_eq!(o, Outcome::Exit(0), "{}", body(&c));
            body(&c)
        })
        .collect();
    assert!(
        outputs[0].starts_with("bool(false)\nLeakedSub m 3 RuntimeException\n"),
        "{}",
        outputs[0]
    );
    assert_eq!(outputs[1], outputs[0]);
    assert_eq!(outputs[2], outputs[0]);
}
