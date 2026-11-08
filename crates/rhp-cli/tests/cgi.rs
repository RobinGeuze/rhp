//! Differential test of `rhp --cgi` against php-cgi: every `tests/cgi/*.php`
//! fixture runs under both with the same environment, ini settings, argv and
//! stdin, and stdout, stderr and the exit status must match byte for byte.
//! Skipped when there is no php-cgi (`rhp_difftest::php_cgi`: `RHP_PHP_CGI`
//! names it). It must be the real binary, not CI's `-n` shim: the shim's
//! extra `-d extension=…` arguments change php-cgi's option parsing.
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
//!   unless given by `env`); several `stdin` lines are concatenated.
//! - `stdinrep N TEXT`: append `TEXT` repeated `N` times to the body.
//! - `nocgi`: command-line mode (no CGI variables at all); `arg A` adds an
//!   argument after the options.
//! - `cwd DIR`: the directory both binaries start in.
//! - `closeafter N`: the client goes away after reading N bytes of the
//!   response (stdout is closed); the rest of stdout is not compared.
//! - `ttfb`: the response must start arriving while the script still runs:
//!   stdin gets a line only once the first stdout bytes came, so a script
//!   that waits for it on `php://stdin` hangs if the engine holds output
//!   back (the run then fails after 10 seconds).
//! - `log`: `CGI_LOG` names a fresh file; what the script wrote there is
//!   compared too (what a script does after its client went away).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use rhp_difftest::{php_cgi, replace_bytes, spawn, Ran};

#[derive(Default)]
struct Spec {
    env: Vec<(String, String)>,
    ini: Vec<String>,
    args: Vec<String>,
    stdin: Vec<u8>,
    nocgi: bool,
    cwd: Option<String>,
    close_after: Option<usize>,
    ttfb: bool,
    log: bool,
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
                "stdin" => s.stdin.extend(unescape(rest)),
                "stdinrep" => {
                    let (n, text) = rest.split_once(' ').unwrap_or((rest, ""));
                    let n: usize = n.parse().expect("stdinrep count");
                    s.stdin.extend(unescape(text).repeat(n));
                }
                "nocgi" => s.nocgi = true,
                "cwd" => s.cwd = Some(rest.to_string()),
                "closeafter" => s.close_after = Some(rest.parse().expect("closeafter count")),
                "ttfb" => s.ttfb = true,
                "log" => s.log = true,
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

fn run(bin: &Path, pre: &[&str], s: &Spec) -> Ran {
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
    let log = s.log.then(|| {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("rhp-cgi-log-{}-{n}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        cmd.env("CGI_LOG", &p);
        p
    });
    let spawned = if let Some(n) = s.close_after {
        spawn_close_after(&mut cmd, &s.stdin, n)
    } else if s.ttfb {
        spawn_ttfb(&mut cmd)
    } else {
        spawn(&mut cmd, Some(&s.stdin))
    };
    let mut ran = spawned.unwrap_or_else(|e| panic!("spawn {}: {e}", bin.display()));
    // `-T`'s timing.
    let tag = b"Elapsed time: ";
    if let Some(at) = ran.stderr.windows(tag.len()).position(|w| w == tag) {
        let from = at + tag.len();
        let to = ran.stderr[from..]
            .iter()
            .position(|&b| b == b' ')
            .map_or(ran.stderr.len(), |n| from + n);
        ran.stderr.splice(from..to, b"N".iter().copied());
    }
    if let Some(p) = log {
        ran.stdout.extend_from_slice(b"\n--- CGI_LOG\n");
        ran.stdout
            .extend_from_slice(&std::fs::read(&p).unwrap_or_else(|_| b"(none)".to_vec()));
        let _ = std::fs::remove_file(&p);
    }
    ran
}

/// stderr, read on a thread of its own so the child never blocks on it.
fn drain(mut r: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = r.read_to_end(&mut v);
        v
    })
}

/// `closeafter`: read `n` bytes of stdout, then close it.
fn spawn_close_after(cmd: &mut Command, stdin: &[u8], n: usize) -> std::io::Result<Ran> {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let mut pipe = child.stdin.take().unwrap();
    let body = stdin.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = pipe.write_all(&body);
    });
    let err = drain(child.stderr.take().unwrap());
    let mut out = child.stdout.take().unwrap();
    let mut got = vec![0u8; n];
    let mut have = 0;
    while have < n {
        match out.read(&mut got[have..])? {
            0 => break,
            k => have += k,
        }
    }
    got.truncate(have);
    drop(out);
    let status = child.wait()?;
    let _ = writer.join();
    Ok(Ran {
        stdout: got,
        stderr: err.join().unwrap_or_default(),
        status: status.code(),
    })
}

/// `ttfb`: stdin gets its line only after the first stdout bytes.
fn spawn_ttfb(cmd: &mut Command) -> std::io::Result<Ran> {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let mut pipe = child.stdin.take().unwrap();
    let err = drain(child.stderr.take().unwrap());
    let mut out = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut all = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            match out.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(k) => {
                    all.extend_from_slice(&buf[..k]);
                    let _ = tx.send(());
                }
            }
        }
        all
    });
    let early = rx.recv_timeout(Duration::from_secs(10)).is_ok();
    if early {
        let _ = pipe.write_all(b"go\n");
    } else {
        let _ = child.kill();
    }
    drop(pipe);
    let status = child.wait()?;
    let mut stdout = reader.join().unwrap_or_default();
    if !early {
        stdout.splice(0..0, b"(no output while the script ran) ".iter().copied());
    }
    Ok(Ran {
        stdout,
        stderr: err.join().unwrap_or_default(),
        status: status.code(),
    })
}

/// The `X.Y.Z` version php-cgi reports for `-n -v` (`PHP 8.5.12 (cgi-fcgi) …`).
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
    // rhp is php 8.5: another minor version's php-cgi (a contributor's
    // older php, say) is not a reference. A different 8.5 patch release only
    // changes the X-Powered-By version, which is normalised below.
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
    // php-cgi prints its own file name in the usage text; Debian's is
    // `php-cgi8.5`, rhp always says `php-cgi`.
    let their_name = php.file_name().unwrap().to_string_lossy().into_owned();
    let mut version_skipped = Vec::new();
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
            want.stdout = replace_bytes(&want.stdout, &their_powered, &our_powered);
            // Anything else printing php's version (headers_list(),
            // PHP_VERSION, var_dump lengths) can't match another patch
            // release: skip it, as the phpt runner does.
            let v = theirs.as_bytes();
            if want.stdout.windows(v.len()).any(|w| w == v) {
                version_skipped.push(name);
                continue;
            }
        }
        if their_name != "php-cgi" {
            want.stdout = replace_bytes(
                &want.stdout,
                format!("Usage: {their_name} ").as_bytes(),
                b"Usage: php-cgi ",
            );
            want.stdout = replace_bytes(
                &want.stdout,
                format!("       {their_name} <file>").as_bytes(),
                b"       php-cgi <file>",
            );
        }
        let got = run(rhp, &["--cgi"], &s);
        let diffs = want.differences("php-cgi", &got, "rhp");
        if !diffs.is_empty() {
            failures.push(format!("== {name}\n{}", diffs.join("\n")));
        }
    }
    if !version_skipped.is_empty() {
        eprintln!(
            "php-cgi is {theirs}, rhp is {ours}: skipped {} fixture(s) that print the version: {}",
            version_skipped.len(),
            version_skipped.join(", ")
        );
    }
    assert!(
        failures.is_empty(),
        "{} of {} fixtures differ:\n{}",
        failures.len(),
        fixtures.len(),
        failures.join("\n")
    );
}

/// Uploaded files live in `upload_tmp_dir` until the request ends: the ones
/// the script did not move away are deleted then (php's
/// `destroy_uploaded_files_hash`), under php-cgi and `rhp --cgi` alike.
#[test]
fn uploads_are_deleted_after_the_request() {
    let base = std::env::temp_dir().join(format!("rhp-cgi-uploads-{}", std::process::id()));
    let tmp = base.join("tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    let script = base.join("upload.php");
    std::fs::write(
        &script,
        "<?php\nvar_dump(move_uploaded_file($_FILES['a']['tmp_name'], __DIR__ . '/tmp/kept.txt'));\n\
         var_dump(count(glob(__DIR__ . '/tmp/php*')));\n",
    )
    .unwrap();
    let body = "--XX\r\nContent-Disposition: form-data; name=\"a\"; filename=\"a.txt\"\r\n\r\nA\r\n\
                --XX\r\nContent-Disposition: form-data; name=\"b\"; filename=\"b.txt\"\r\n\r\nB\r\n--XX--\r\n";
    let s = Spec {
        env: vec![
            ("REDIRECT_STATUS".into(), "1".into()),
            ("REQUEST_METHOD".into(), "POST".into()),
            (
                "SCRIPT_FILENAME".into(),
                script.to_string_lossy().into_owned(),
            ),
            (
                "CONTENT_TYPE".into(),
                "multipart/form-data; boundary=XX".into(),
            ),
            ("CONTENT_LENGTH".into(), body.len().to_string()),
        ],
        ini: vec![format!("upload_tmp_dir={}", tmp.display())],
        stdin: body.as_bytes().to_vec(),
        ..Spec::default()
    };
    let mut engines: Vec<(PathBuf, Vec<&str>)> =
        vec![(PathBuf::from(env!("CARGO_BIN_EXE_rhp")), vec!["--cgi"])];
    if let Some(php) = php_cgi() {
        engines.push((php, vec![]));
    }
    for (bin, pre) in &engines {
        let r = run(bin, pre, &s);
        let out = String::from_utf8_lossy(&r.stdout).into_owned();
        assert!(
            out.ends_with("bool(true)\nint(1)\n"),
            "{}: {out}",
            bin.display()
        );
        let mut left: Vec<String> = std::fs::read_dir(&tmp)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["kept.txt"], "{}", bin.display());
        std::fs::remove_file(tmp.join("kept.txt")).unwrap();
    }
    std::fs::remove_dir_all(&base).unwrap();
}

/// A form body of 16 KiB or more goes to a temporary file in
/// `upload_tmp_dir` while the request runs (php's `php://temp` request
/// body), deleted when it ends; a smaller one stays in memory.
#[test]
fn post_bodies_spill_to_a_temporary_file() {
    let base = std::env::temp_dir().join(format!("rhp-cgi-spill-{}", std::process::id()));
    let tmp = base.join("tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    let script = base.join("spill.php");
    std::fs::write(
        &script,
        "<?php\necho strlen($_POST['a']), ' ', count(glob(__DIR__ . '/tmp/php*')), \"\\n\";\n",
    )
    .unwrap();
    let mut engines: Vec<(PathBuf, Vec<&str>)> =
        vec![(PathBuf::from(env!("CARGO_BIN_EXE_rhp")), vec!["--cgi"])];
    if let Some(php) = php_cgi() {
        engines.push((php, vec![]));
    }
    for (len, files) in [(16383, 0), (16384, 1), (40000, 1)] {
        let body = format!("a={}", "x".repeat(len - 2));
        let s = Spec {
            env: vec![
                ("REDIRECT_STATUS".into(), "1".into()),
                ("REQUEST_METHOD".into(), "POST".into()),
                (
                    "SCRIPT_FILENAME".into(),
                    script.to_string_lossy().into_owned(),
                ),
                (
                    "CONTENT_TYPE".into(),
                    "application/x-www-form-urlencoded".into(),
                ),
                ("CONTENT_LENGTH".into(), body.len().to_string()),
            ],
            ini: vec![format!("upload_tmp_dir={}", tmp.display())],
            stdin: body.into_bytes(),
            ..Spec::default()
        };
        for (bin, pre) in &engines {
            let r = run(bin, pre, &s);
            let out = String::from_utf8_lossy(&r.stdout).into_owned();
            let want = format!("{} {files}\n", len - 2);
            assert!(out.ends_with(&want), "{} {len}: {out}", bin.display());
            assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0);
        }
    }
    std::fs::remove_dir_all(&base).unwrap();
}
