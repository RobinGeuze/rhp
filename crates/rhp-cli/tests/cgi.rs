//! Differential test of `rhp --cgi` against php-cgi: every `tests/cgi/*.php`
//! fixture runs under both with the same environment, ini settings, argv and
//! stdin, and stdout, stderr and the exit status must match byte for byte.
//! Skipped when php-cgi is not installed (`PHP_CGI` overrides the path).
//!
//! An optional `NAME.cgi` next to `NAME.php` describes the request, one
//! directive per line (`{SCRIPT}` / `{DIR}` expand to the fixture's absolute
//! path / directory; `\r` / `\n` escapes are decoded in `stdin`):
//!
//! - `env K=V`: set a CGI variable (defaults: `REDIRECT_STATUS=1`,
//!   `REQUEST_METHOD=GET`, `SCRIPT_FILENAME={SCRIPT}`).
//! - `script PATH`: `SCRIPT_FILENAME`.
//! - `ini k=v`: a `-d k=v` option.
//! - `stdin BODY`: the request body (`CONTENT_LENGTH` set to its length
//!   unless given by `env`).
//! - `nocgi`: command-line mode (no CGI variables at all); `arg A` adds an
//!   argument after the options.
//! - `cwd DIR`: the directory both binaries start in.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn php_cgi() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("PHP_CGI") {
        return Some(PathBuf::from(p));
    }
    for dir in std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/bin"),
        ])
    {
        let c = dir.join("php-cgi");
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

#[derive(Default)]
struct Spec {
    env: Vec<(String, String)>,
    ini: Vec<String>,
    args: Vec<String>,
    stdin: Vec<u8>,
    nocgi: bool,
    cwd: Option<String>,
}

fn unescape(s: &str) -> Vec<u8> {
    s.replace("\\r", "\r").replace("\\n", "\n").into_bytes()
}

fn spec(php: &Path) -> Spec {
    let script = php.to_string_lossy().into_owned();
    let dir = php.parent().unwrap().to_string_lossy().into_owned();
    let mut s = Spec::default();
    let mut script_filename = Some(script.clone());
    let mut has_cl = false;
    if let Ok(text) = std::fs::read_to_string(php.with_extension("cgi")) {
        for line in text.lines() {
            let line = line.replace("{SCRIPT}", &script).replace("{DIR}", &dir);
            let (kw, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
            match kw {
                "env" => {
                    let (k, v) = rest.split_once('=').unwrap_or((rest, ""));
                    has_cl |= k == "CONTENT_LENGTH";
                    s.env.push((k.to_string(), v.to_string()));
                }
                "script" => script_filename = Some(rest.to_string()),
                "ini" => s.ini.push(rest.to_string()),
                "arg" => s.args.push(rest.to_string()),
                "stdin" => s.stdin = unescape(rest),
                "nocgi" => s.nocgi = true,
                "cwd" => s.cwd = Some(rest.to_string()),
                "" => {}
                other => panic!("{}: unknown directive {other}", php.display()),
            }
        }
    }
    if !s.nocgi {
        let mut base = vec![
            ("REDIRECT_STATUS".to_string(), "1".to_string()),
            ("REQUEST_METHOD".to_string(), "GET".to_string()),
        ];
        if let Some(sf) = script_filename {
            base.push(("SCRIPT_FILENAME".to_string(), sf));
        }
        base.retain(|(k, _)| !s.env.iter().any(|(n, _)| n == k));
        base.append(&mut s.env);
        s.env = base;
        if !s.stdin.is_empty() && !has_cl {
            s.env
                .push(("CONTENT_LENGTH".to_string(), s.stdin.len().to_string()));
        }
    }
    s
}

fn replace(hay: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(hay.len());
    let mut i = 0;
    while i < hay.len() {
        if hay[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(hay[i]);
            i += 1;
        }
    }
    out
}

struct Run {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    status: Option<i32>,
}

fn run(bin: &Path, pre: &[&str], s: &Spec) -> Run {
    let mut cmd = Command::new(bin);
    cmd.env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("TZ", "UTC")
        .args(pre)
        .arg("-n")
        .arg("-d")
        .arg("opcache.enable=0");
    for i in &s.ini {
        cmd.arg("-d").arg(i);
    }
    cmd.args(&s.args);
    if let Some(dir) = &s.cwd {
        cmd.current_dir(dir);
    }
    for (k, v) in &s.env {
        cmd.env(k, v);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut stdin = child.stdin.take().unwrap();
    let body = s.stdin.clone();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&body);
    });
    let out = child.wait_with_output().expect("wait");
    let _ = writer.join();
    Run {
        stdout: out.stdout,
        stderr: out.stderr,
        status: out.status.code(),
    }
}

/// The `X.Y.Z` version php-cgi reports for `-n -v` (`PHP 8.5.8 (cgi-fcgi) …`).
fn version(bin: &Path) -> Option<String> {
    let out = Command::new(bin).args(["-n", "-v"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let v = text.strip_prefix("PHP ")?.split_whitespace().next()?;
    Some(v.to_string())
}

/// rhp's `PHP_VERSION`.
fn rhp_version(rhp: &Path) -> String {
    let out = Command::new(rhp)
        .args(["-n", "-r", "echo PHP_VERSION;"])
        .output()
        .expect("rhp -r");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn minor(v: &str) -> &str {
    let mut it = v.match_indices('.');
    match it.nth(1) {
        Some((i, _)) => &v[..i],
        None => v,
    }
}

#[test]
fn cgi_matches_php_cgi() {
    let Some(php) = php_cgi() else {
        eprintln!("php-cgi not found; skipping");
        return;
    };
    let rhp = Path::new(env!("CARGO_BIN_EXE_rhp"));
    // rhp is php 8.5: another minor version's php-cgi (CI's 8.4 leg) is
    // not a reference. A different 8.5 patch release only changes the
    // X-Powered-By version, which is normalised below.
    let ours = rhp_version(rhp);
    let Some(theirs) = version(&php) else {
        eprintln!("{}: no version; skipping", php.display());
        return;
    };
    if minor(&ours) != minor(&theirs) {
        eprintln!("php-cgi is {theirs}, rhp is {ours}; skipping");
        return;
    }
    let powered = |v: &str| format!("X-Powered-By: PHP/{v}\r\n").into_bytes();
    let (their_powered, our_powered) = (powered(&theirs), powered(&ours));
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cgi");
    let dir = std::fs::canonicalize(dir).expect("tests/cgi");
    let mut fixtures: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "php"))
        .collect();
    fixtures.sort();
    assert!(!fixtures.is_empty());
    let only = std::env::var("CGI_ONLY").ok();
    let mut failures = Vec::new();
    for f in &fixtures {
        let name = f.file_stem().unwrap().to_string_lossy().into_owned();
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        let s = spec(f);
        let mut want = run(&php, &[], &s);
        if their_powered != our_powered {
            want.stdout = replace(&want.stdout, &their_powered, &our_powered);
        }
        let got = run(rhp, &["--cgi"], &s);
        let mut diffs = Vec::new();
        if want.stdout != got.stdout {
            diffs.push(format!(
                "stdout:\n--- php-cgi\n{}\n--- rhp\n{}",
                String::from_utf8_lossy(&want.stdout),
                String::from_utf8_lossy(&got.stdout)
            ));
        }
        if want.stderr != got.stderr {
            diffs.push(format!(
                "stderr:\n--- php-cgi\n{}\n--- rhp\n{}",
                String::from_utf8_lossy(&want.stderr),
                String::from_utf8_lossy(&got.stderr)
            ));
        }
        if want.status != got.status {
            diffs.push(format!(
                "exit: php-cgi {:?}, rhp {:?}",
                want.status, got.status
            ));
        }
        if !diffs.is_empty() {
            failures.push(format!("== {name}\n{}", diffs.join("\n")));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} fixtures differ:\n{}",
        failures.len(),
        fixtures.len(),
        failures.join("\n")
    );
}
