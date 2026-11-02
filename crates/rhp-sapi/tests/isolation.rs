//! Requests run one after another on one thread must not see each other's
//! state: each [`run_request`] builds a fresh engine and resets the
//! builtins' thread-local state.

use std::cell::RefCell;
use std::rc::Rc;

use rhp_sapi::{run_request, Outcome, Request, RequestInfo, Response, Sapi, Script};

/// A SAPI capturing the response into shared buffers.
#[derive(Clone, Default)]
struct Capture {
    body: Rc<RefCell<Vec<u8>>>,
    headers: Rc<RefCell<Vec<Vec<u8>>>>,
    code: Rc<RefCell<i64>>,
}

impl Sapi for Capture {
    fn name(&self) -> &str {
        "embed-test"
    }

    fn ub_write(&mut self, data: &[u8]) {
        self.body.borrow_mut().extend_from_slice(data);
    }

    fn send_headers(&mut self, r: &Response) {
        *self.headers.borrow_mut() = r.headers.clone();
        *self.code.borrow_mut() = r.code;
    }
}

fn run(code: &str, info: RequestInfo) -> (Capture, Outcome) {
    let cap = Capture::default();
    let outcome = run_request(
        Box::new(cap.clone()),
        Request {
            script: Script::Code(code.to_string()),
            ini: Vec::new(),
            args: Vec::new(),
            info: Some(info),
            php_binary: None,
        },
    );
    (cap, outcome)
}

fn body(c: &Capture) -> String {
    String::from_utf8_lossy(&c.body.borrow()).into_owned()
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
    assert_eq!(*c1.code.borrow(), 404);
    assert!(c1.headers.borrow().iter().any(|h| h == b"X-First: 1"));

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
  string(23) "X-Powered-By: PHP/8.5.8"
}
"#;
    assert_eq!(body(&c2), want);
    assert_eq!(*c2.code.borrow(), 200);
    assert!(!c2.headers.borrow().iter().any(|h| h == b"X-First: 1"));
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
    struct Other(Rc<RefCell<Vec<u8>>>);
    impl Sapi for Other {
        fn name(&self) -> &str {
            "other"
        }
        fn ub_write(&mut self, data: &[u8]) {
            self.0.borrow_mut().extend_from_slice(data);
        }
    }
    let buf = Rc::new(RefCell::new(Vec::new()));
    run_request(
        Box::new(Other(buf.clone())),
        Request {
            script: Script::Code("echo php_sapi_name();".to_string()),
            ini: Vec::new(),
            args: Vec::new(),
            info: None,
            php_binary: None,
        },
    );
    assert_eq!(&buf.borrow()[..], b"other");
}
