//! Classic mode against worker mode on one script: N requests through
//! [`run_request`] each (a fresh engine per request, opcache kept), then
//! the same N served by one worker (`frankenphp_handle_request`). The
//! script is run with `$_SERVER['RHP_WORKER']` set in worker mode; it
//! must call `frankenphp_handle_request` in a loop then, and handle one
//! request otherwise. Prints the median request time of each.
//!
//! ```sh
//! cargo run --release -p rhp-sapi --example worker_bench -- <script> <requests>
//! ```

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use rhp_sapi::{run_request, Request, RequestInfo, Response, Sapi, Script};

#[global_allocator]
static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;

/// Collects the body; in worker mode hands out `left` requests and notes
/// when each one ends.
struct Bench {
    body: Rc<RefCell<Vec<u8>>>,
    worker: bool,
    left: usize,
    started: Rc<RefCell<Option<Instant>>>,
    times: Rc<RefCell<Vec<Duration>>>,
}

impl Sapi for Bench {
    fn name(&self) -> &str {
        "bench"
    }

    fn ub_write(&mut self, data: &[u8]) -> bool {
        self.body.borrow_mut().extend_from_slice(data);
        true
    }

    fn send_headers(&mut self, _r: &Response) -> bool {
        true
    }

    fn server_functions(&self) -> bool {
        true
    }

    fn worker(&self) -> bool {
        self.worker
    }

    fn next_request(&mut self) -> Option<RequestInfo> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        *self.started.borrow_mut() = Some(Instant::now());
        Some(info(false))
    }

    fn end_response(&mut self) {
        if let Some(t) = self.started.borrow_mut().take() {
            self.times.borrow_mut().push(t.elapsed());
        }
    }
}

fn info(worker: bool) -> RequestInfo {
    let mut server = vec![(b"REQUEST_URI".to_vec(), b"/".to_vec())];
    if worker {
        server.push((b"RHP_WORKER".to_vec(), b"1".to_vec()));
    }
    RequestInfo {
        server,
        method: Some(b"GET".to_vec()),
        response_code: 200,
        ..Default::default()
    }
}

fn request(script: &str, sapi: Bench, worker: bool) {
    run_request(
        Box::new(sapi),
        Request {
            script: Script::File(script.to_string()),
            ini: vec![("max_execution_time".into(), "0".into())],
            args: Vec::new(),
            info: Some(info(worker)),
            php_binary: None,
            cwd: None,
            ini_files: None,
        },
    );
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v.get(v.len() / 2).copied().unwrap_or_default()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let script = args.get(1).expect("script");
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
    let sapi = |worker, left| Bench {
        body: Rc::default(),
        worker,
        left,
        started: Rc::default(),
        times: Rc::default(),
    };
    let mut classic = Vec::new();
    for _ in 0..n {
        let t = Instant::now();
        request(script, sapi(false, 0), false);
        classic.push(t.elapsed());
    }
    let w = sapi(true, n);
    let times = w.times.clone();
    request(script, w, true);
    let worker = times.borrow().clone();
    println!(
        "classic: median {:?} per request; worker: median {:?} per request ({} requests)",
        median(classic),
        median(worker),
        n
    );
}
