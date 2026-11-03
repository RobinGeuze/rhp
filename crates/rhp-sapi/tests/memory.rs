//! memory_limit under the embedding API: a request that runs out of memory
//! ends as php ends it (fatal error, shutdown functions, status 500), and
//! the next request on the same thread starts with a clean slate.

use std::cell::RefCell;
use std::rc::Rc;

use rhp_sapi::{run_request, Outcome, Request, RequestInfo, Response, Sapi, Script};

#[global_allocator]
static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;

#[derive(Clone, Default)]
struct Capture {
    body: Rc<RefCell<Vec<u8>>>,
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
        *self.code.borrow_mut() = r.code;
    }
}

fn run(code: &str, ini: &[(&str, &str)]) -> (String, i64, Outcome) {
    let cap = Capture::default();
    let outcome = run_request(
        Box::new(cap.clone()),
        Request {
            script: Script::Code(code.to_string()),
            ini: ini
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            args: Vec::new(),
            info: Some(RequestInfo {
                method: Some(b"GET".to_vec()),
                response_code: 200,
                ..Default::default()
            }),
            php_binary: None,
        },
    );
    let body = String::from_utf8_lossy(&cap.body.borrow()).into_owned();
    let code = *cap.code.borrow();
    (body, code, outcome)
}

const GREEDY: &str = r#"
    register_shutdown_function(function () {
        $e = error_get_last();
        echo "shutdown: ", $e['type'], " ", $e['message'], "\n";
    });
    echo "start\n";
    $a = [];
    while (true) {
        $a[] = str_repeat('x', 1024);
    }
"#;

const MODEST: &str = r#"
    echo ini_get('memory_limit'), "\n";
    var_dump(memory_get_usage() < 1024 * 1024, memory_get_peak_usage(true) <= 4 * 1024 * 1024);
    $s = str_repeat('y', 32 * 1024 * 1024);
    var_dump(strlen($s), error_get_last());
"#;

#[test]
fn exhausted_request_then_a_normal_one_on_the_same_thread() {
    let (body, code, outcome) = run(GREEDY, &[("memory_limit", "8M"), ("display_errors", "1")]);
    assert_eq!(outcome, Outcome::Exit(255));
    // display_errors=1: the error is shown, so the status stays 200.
    assert_eq!(code, 200);
    assert!(body.starts_with("start\n"), "{body}");
    assert!(
        body.contains(
            "\nFatal error: Allowed memory size of 8388608 bytes exhausted (tried to allocate "
        ),
        "{body}"
    );
    let tail = body.rsplit_once("shutdown: ").map(|(_, t)| t).unwrap_or("");
    assert!(
        tail.starts_with("1 Allowed memory size of 8388608 bytes exhausted (tried to allocate ")
            && tail.ends_with(" bytes)\n"),
        "{body}"
    );

    let (body, code, outcome) = run(MODEST, &[]);
    assert_eq!(outcome, Outcome::Exit(0));
    assert_eq!(code, 200);
    assert_eq!(body, "128M\nbool(true)\nbool(true)\nint(33554432)\nNULL\n");
}

#[test]
fn hidden_fatal_turns_the_response_into_a_500() {
    // No output before the error: the headers can still change.
    let quiet = GREEDY.replace("echo \"start\\n\";", "");
    let (body, code, outcome) = run(&quiet, &[("memory_limit", "4M"), ("display_errors", "0")]);
    assert_eq!(outcome, Outcome::Exit(255));
    assert_eq!(code, 500);
    assert!(
        body.starts_with("shutdown: 1 Allowed memory size of 4194304"),
        "{body}"
    );
}

#[test]
fn a_far_string_offset_ends_only_the_request() {
    // Past php's hard limit (twice memory_limit) php-cli exits; a server
    // SAPI ends the request with the fatal error instead.
    let code = r#"
        register_shutdown_function(function () { echo "shutdown\n"; });
        $s = '';
        $s[100 * 1024 * 1024] = 'x';
        echo "not reached\n";
    "#;
    let (body, _, outcome) = run(code, &[("memory_limit", "8M"), ("html_errors", "0")]);
    assert_eq!(outcome, Outcome::Exit(255));
    assert!(
        body.contains(
            "Fatal error: Hard memory limit of 16777216 bytes exhausted (tried to allocate "
        ),
        "{body}"
    );
    assert!(body.ends_with("shutdown\n"), "{body}");
    let (body, _, outcome) = run("echo memory_get_usage(true);", &[]);
    assert_eq!(outcome, Outcome::Exit(0));
    assert_eq!(body, "2097152");
}
