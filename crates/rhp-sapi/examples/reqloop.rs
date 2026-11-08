//! Run one front controller N times per worker thread through
//! [`run_request`], FPM-style: a fresh engine per request, the thread's
//! compiled-script cache (opcache) and realpath cache kept across them.
//! Prints the first request's time and the median of the rest.
//!
//! ```sh
//! cargo run --release -p rhp-sapi --example reqloop -- \
//!     <script> <uri> <requests> <threads> [ini=value ...]
//! ```

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use rhp_sapi::{run_request, Request, RequestInfo, Sapi, Script};

// As an embedding server installs it (memory_get_usage(), memory_limit).
#[global_allocator]
static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;

#[derive(Clone, Default)]
struct Body(Rc<RefCell<Vec<u8>>>, Rc<RefCell<i64>>);

impl Sapi for Body {
    fn name(&self) -> &str {
        "cgi-fcgi"
    }

    fn ub_write(&mut self, data: &[u8]) -> bool {
        self.0.borrow_mut().extend_from_slice(data);
        true
    }

    fn send_headers(&mut self, r: &rhp_builtins::main::sapi::Response) -> bool {
        *self.1.borrow_mut() = r.code;
        true
    }
}

fn one(script: &str, uri: &str, ini: &[(String, String)]) -> (i64, Vec<u8>) {
    let body = Body::default();
    let root = std::path::Path::new(script)
        .parent()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let qs = uri.split_once('?').map_or("", |x| x.1);
    let server: Vec<(Vec<u8>, Vec<u8>)> = [
        ("REDIRECT_STATUS", "1"),
        ("SCRIPT_FILENAME", script),
        ("SCRIPT_NAME", "/index.php"),
        ("REQUEST_URI", uri),
        ("QUERY_STRING", qs),
        ("REQUEST_METHOD", "GET"),
        ("HTTP_HOST", "localhost"),
        ("SERVER_NAME", "localhost"),
        ("SERVER_PORT", "80"),
        ("SERVER_PROTOCOL", "HTTP/1.1"),
        ("REMOTE_ADDR", "127.0.0.1"),
        ("DOCUMENT_ROOT", &root),
        ("PHP_SELF", "/index.php"),
    ]
    .iter()
    .map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec()))
    .collect();
    let mut ini = ini.to_vec();
    ini.push(("opcache.enable".into(), "1".into()));
    ini.push(("opcache.file_update_protection".into(), "0".into()));
    run_request(
        Box::new(body.clone()),
        Request {
            script: Script::File(script.to_string()),
            ini,
            args: Vec::new(),
            info: Some(RequestInfo {
                env: server.clone(),
                server,
                method: Some(b"GET".to_vec()),
                query_string: Some(qs.as_bytes().to_vec()),
                response_code: 200,
                proto_num: 1001,
                ..Default::default()
            }),
            php_binary: None,
            cwd: Some(root.into_bytes()),
            ini_files: None,
        },
    );
    let code = *body.1.borrow();
    let out = body.0.borrow().clone();
    (code, out)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 5 {
        eprintln!("usage: reqloop <script> <uri> <requests> <threads> [ini=value ...]");
        std::process::exit(2);
    }
    let (script, uri) = (a[1].clone(), a[2].clone());
    let n: usize = a[3].parse().expect("requests");
    let threads: usize = a[4].parse().expect("threads");
    let ini: Vec<(String, String)> = a[5..]
        .iter()
        .map(|s| {
            let (k, v) = s.split_once('=').expect("ini=value");
            (k.into(), v.into())
        })
        .collect();
    let start = Instant::now();
    let workers: Vec<_> = (0..threads)
        .map(|_| {
            let (s, u, ini) = (script.clone(), uri.clone(), ini.clone());
            std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn(move || {
                    let mut times = Vec::new();
                    let mut last = (0, Vec::new());
                    for _ in 0..n {
                        let t = Instant::now();
                        last = one(&s, &u, &ini);
                        times.push(t.elapsed().as_secs_f64() * 1000.0);
                    }
                    (times, last)
                })
                .unwrap()
        })
        .collect();
    let mut all = Vec::new();
    for w in workers {
        let (t, (code, body)) = w.join().unwrap();
        let mut rest = t[1..].to_vec();
        rest.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = rest.get(rest.len() / 2).copied().unwrap_or(0.0);
        eprintln!(
            "first {:.1} ms, then median {median:.2} ms; status {code}, body {} bytes, starts {:?}",
            t[0],
            body.len(),
            String::from_utf8_lossy(&body[..body.len().min(60)])
        );
        all.extend(t);
    }
    let wall = start.elapsed().as_secs_f64();
    eprintln!(
        "{} requests on {threads} threads in {wall:.2} s: {:.0} req/s",
        all.len(),
        all.len() as f64 / wall
    );
}
