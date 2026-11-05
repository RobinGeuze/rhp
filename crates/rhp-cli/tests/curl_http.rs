//! ext/curl over real HTTP. Each `tests/curl_http/NAME.php` talks to a small
//! local HTTP/1.1 server started here; `__BASE__` in the script becomes its
//! `http://127.0.0.1:PORT` and the server prints `PORT` wherever the port
//! would appear, so `NAME.out` (the exact stdout of `php -n NAME.php` against
//! this server) stays portable. Regenerate the expectations with
//! `cargo test -p rhp-cli --test curl_http -- --ignored regenerate`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;

use rhp_sapi::{CaptureSapi, Outcome, Request, Script};

// The engine's memory accounting (memory_get_usage(), memory_limit).
#[global_allocator]
static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;

/// `gzencode("compressed payload!")` as php produces it.
const GZ_BODY: &[u8] = &[
    0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x13, 0x4b, 0xce, 0xcf, 0x2d, 0x28, 0x4a,
    0x2d, 0x2e, 0x4e, 0x4d, 0x51, 0x28, 0x48, 0xac, 0xcc, 0xc9, 0x4f, 0x4c, 0x51, 0x04, 0x00, 0x18,
    0xc3, 0xbd, 0xa2, 0x13, 0x00, 0x00, 0x00,
];

fn respond(stream: &mut TcpStream, status: &str, headers: &[(&str, String)], body: &[u8]) {
    let mut out = format!("HTTP/1.1 {status}\r\n").into_bytes();
    for (n, v) in headers {
        out.extend_from_slice(format!("{n}: {v}\r\n").as_bytes());
    }
    out.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    out.extend_from_slice(b"Connection: close\r\n\r\n");
    out.extend_from_slice(body);
    let _ = stream.write_all(&out);
}

fn handle(mut stream: TcpStream, port: u16) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut head: Vec<String> = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let line = line.trim_end_matches(['\r', '\n']).to_string();
        if line.is_empty() {
            break;
        }
        head.push(line);
    }
    let Some(request_line) = head.first().cloned() else {
        return;
    };
    let len = head
        .iter()
        .skip(1)
        .filter_map(|l| l.split_once(':'))
        .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    let _ = reader.read_exact(&mut body);
    let path = request_line.split(' ').nth(1).unwrap_or("/").to_string();
    let path = path.split('?').next().unwrap_or("").to_string();
    let port_s = port.to_string();
    match path.as_str() {
        "/hello" => respond(
            &mut stream,
            "200 OK",
            &[
                ("Content-type", "text/html; charset=UTF-8".into()),
                ("X-lower-Case", "a".into()),
                ("x-UPPER", "b".into()),
            ],
            b"hello world",
        ),
        "/echo" => {
            let mut text = String::new();
            for l in &head {
                text.push_str(&l.replace(&port_s, "PORT"));
                text.push('\n');
            }
            text.push_str("--\n");
            text.push_str(&String::from_utf8_lossy(&body));
            respond(
                &mut stream,
                "200 OK",
                &[("Content-Type", "text/plain".into())],
                text.as_bytes(),
            );
        }
        "/xhost" => respond(
            &mut stream,
            "302 Found",
            &[("Location", format!("http://localhost:{port}/echo"))],
            b"",
        ),
        "/same" => respond(
            &mut stream,
            "302 Found",
            &[("Location", "/echo".into())],
            b"",
        ),
        "/hop" => respond(
            &mut stream,
            "301 Moved Permanently",
            &[("location", "/hello".into()), ("X-Hop", "One".into())],
            b"moved",
        ),
        "/gz" => respond(
            &mut stream,
            "200 OK",
            &[
                ("Content-Type", "text/plain".into()),
                ("Content-Encoding", "gzip".into()),
            ],
            GZ_BODY,
        ),
        _ => respond(&mut stream, "404 Not Found", &[], b"nf"),
    }
}

/// Start the server; returns its port.
fn serve() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            std::thread::spawn(move || handle(s, port));
        }
    });
    port
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/curl_http")
        .canonicalize()
        .expect("tests/curl_http exists")
}

fn cases() -> Vec<PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(fixtures_dir())
        .expect("read fixtures")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "php"))
        .collect();
    v.sort();
    v
}

fn source_for(php: &Path, port: u16) -> String {
    std::fs::read_to_string(php)
        .expect("read fixture")
        .replace("__BASE__", &format!("http://127.0.0.1:{port}"))
}

fn run_vm(source: &str, name: &str) -> Vec<u8> {
    let script = Script::Source {
        path: name.to_string(),
        bytes: source.as_bytes().to_vec(),
    };
    let (outcome, out) = CaptureSapi::cli().run(Request::new(script));
    assert_eq!(
        outcome,
        Outcome::Exit(0),
        "{name}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

#[test]
fn curl_http_matches_php() {
    let port = serve();
    let mut failures = Vec::new();
    for php in cases() {
        let name = php.file_stem().unwrap().to_string_lossy().into_owned();
        let expected = std::fs::read(php.with_extension("out")).expect("missing .out");
        let actual = run_vm(&source_for(&php, port), &name);
        if actual != expected {
            failures.push(format!(
                "{name}: MISMATCH\n  expected: {:?}\n  actual:   {:?}",
                String::from_utf8_lossy(&expected),
                String::from_utf8_lossy(&actual)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Rewrites every `NAME.out` from the real php (`php -n`) against the server.
#[test]
#[ignore]
fn regenerate() {
    let port = serve();
    let tmp = std::env::temp_dir().join(format!("rhp_curl_http_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mkdir");
    for php in cases() {
        let script = tmp.join(php.file_name().unwrap());
        std::fs::write(&script, source_for(&php, port)).expect("write");
        let out = Command::new(rhp_difftest::php_bin())
            .arg("-n")
            .arg(&script)
            .output()
            .expect("run php");
        std::fs::write(php.with_extension("out"), &out.stdout).expect("write .out");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
