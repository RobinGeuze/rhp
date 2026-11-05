use super::*;

#[test]
fn parses_sections() {
    let p = Phpt::parse(
        b"--TEST--\nname\n--SKIPIF--\n<?php ?>\n--FILE--\n<?php\necho 1;\n?>\n--EXPECT--\n1\n",
    );
    assert_eq!(p.section("TEST"), Some(&b"name\n"[..]));
    assert_eq!(p.section("FILE"), Some(&b"<?php\necho 1;\n?>\n"[..]));
    assert_eq!(p.section("EXPECT"), Some(&b"1\n"[..]));
    assert!(p.has("SKIPIF") && !p.has("INI"));
}

#[test]
fn section_header_rules() {
    assert_eq!(section_header(b"--FILE--\n"), Some("FILE".into()));
    assert_eq!(
        section_header(b"--FILE_EXTERNAL--  \r\n"),
        Some("FILE_EXTERNAL".into())
    );
    assert_eq!(section_header(b"--file--\n"), None);
    assert_eq!(section_header(b"-- FILE --\n"), None);
    assert_eq!(section_header(b"----\n"), None);
    assert_eq!(section_header(b"$a--;--\n"), None);
}

#[test]
fn normalises_like_run_tests() {
    assert_eq!(normalise(b"\n a\r\nb \x0b\0\n"), b"a\nb");
}

#[test]
fn strips_diagnostics_and_blank_lines() {
    let php = b"1\n\nWarning: Undefined variable $x in /t.php on line 3\n2\n";
    assert_eq!(strip_diagnostics(php), strip_diagnostics(b"1\n2\n"));
}

#[test]
fn ignore_list_format() {
    let l = parse_ignored("# c\n\na/b.phpt  why: this\nc.phpt\tthat\n").unwrap();
    assert_eq!(l["a/b.phpt"], "why: this");
    assert_eq!(l["c.phpt"], "that");
    assert_eq!(
        parse_ignored("a.phpt\n").unwrap_err(),
        "line 1: a.phpt has no reason"
    );
    assert_eq!(
        parse_ignored("a.phpt x\na.phpt y\n").unwrap_err(),
        "line 2: a.phpt is listed twice"
    );
}

/// The committed list parses, is C-sorted, and known-passing.txt lists
/// none of it (`--check-known` checks that too).
#[test]
fn committed_ignore_list() {
    let ignored = parse_ignored(IGNORED).unwrap();
    assert!(ignored.keys().all(|t| t.ends_with(".phpt")));
    let order: Vec<&str> = IGNORED
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert!(order.is_sorted(), "tests/phpt/ignored.txt is not C-sorted");
    let known = read_known(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/phpt/known-passing.txt"),
    )
    .unwrap();
    let both: Vec<&String> = known.iter().filter(|t| ignored.contains_key(*t)).collect();
    assert!(both.is_empty(), "known-passing and ignored: {both:?}");
}

#[test]
fn ini_noops_run_others_skip() {
    let dir = Path::new("/w");
    let rules = IniRules {
        probe: IniProbe::new(None, &[]),
        extra: Vec::new(),
    };
    let test_ini = |p: &Phpt| rules.test_ini(p, dir, false);
    let p = Phpt::parse(b"--INI--\nprecision=14\nopcache.enable=0\n--FILE--\n");
    assert_eq!(test_ini(&p).unwrap().len(), 2);
    let p = Phpt::parse(b"--INI--\nvariables_order=GPS\n--FILE--\n");
    assert_eq!(test_ini(&p).unwrap_err(), "ini:variables_order");
    let p = Phpt::parse(b"--INI--\noutput_buffering=0\ndisplay_errors=On\n--FILE--\n");
    assert_eq!(test_ini(&p).unwrap().len(), 2);
    let p = Phpt::parse(b"--INI--\noutput_buffering=4096\n--FILE--\n");
    assert_eq!(test_ini(&p).unwrap_err(), "ini:output_buffering");
    // Directives rhp applies run, and only those reach rhp.
    let p =
        Phpt::parse(b"--INI--\npcre.jit=0\noutput_buffering=0\nopcache.jit=tracing\n--FILE--\n");
    let ini = test_ini(&p).unwrap();
    assert_eq!(ini.len(), 3);
    let to_rhp: Vec<&String> = ini.iter().filter(|kv| rules.rhp_applies(kv)).collect();
    assert_eq!(to_rhp, ["pcre.jit=0", "opcache.jit=tracing"]);
    // opcache runs, except with dumps or the file cache. Listed opcode
    // dumps never get here; an unlisted one is flagged.
    let p = Phpt::parse(b"--INI--\nopcache.enable_cli=1\n--FILE--\n");
    assert_eq!(test_ini(&p).unwrap(), ["opcache.enable_cli=1"]);
    let p = Phpt::parse(b"--INI--\nopcache.opt_debug_level=0x20000\n--FILE--\n");
    assert_eq!(
        test_ini(&p).unwrap_err(),
        "opcode dump not on tests/phpt/ignored.txt"
    );
    let p = Phpt::parse(b"--INI--\nopcache.opt_debug_level=0\n--FILE--\n");
    assert!(test_ini(&p).is_ok());
    let p = Phpt::parse(b"--INI--\n;opcache.opt_debug_level=0x20000\n--FILE--\n");
    assert!(test_ini(&p).is_ok());
    let p = Phpt::parse(b"--INI--\nopcache.jit_debug=0x1\n--FILE--\n");
    assert_eq!(test_ini(&p).unwrap_err(), "opcache:jit_debug");
    let p = Phpt::parse(b"--INI--\nopcache.file_cache={PWD}\n--FILE--\n");
    assert_eq!(test_ini(&p).unwrap_err(), "opcache:file_cache");
    // As run-tests' settings2array: trimmed, the last value kept, and a
    // line without '=' dropped.
    let p = Phpt::parse(
        b"--INI--\nerror_reporting = E_ALL\nprecision=10\nprecision = 12 \nflag\n--FILE--\n",
    );
    assert_eq!(
        test_ini(&p).unwrap(),
        ["error_reporting=E_ALL", "precision=12"]
    );
    // --rhp-ini
    let rules = IniRules {
        probe: IniProbe::new(None, &[]),
        extra: vec!["variables_order".into()],
    };
    let p = Phpt::parse(b"--INI--\nvariables_order=GPS\n--FILE--\n");
    assert_eq!(
        rules.test_ini(&p, dir, false).unwrap(),
        ["variables_order=GPS"]
    );
}

#[test]
fn ini_substitutions() {
    let dir = Path::new("/w/t");
    assert_eq!(
        substitute_ini("include_path={PWD}", dir).unwrap(),
        "include_path=/w/t"
    );
    std::env::set_var("RHP_PHPT_UNIT_ENV", "u");
    assert_eq!(
        substitute_ini("a={ENV:RHP_PHPT_UNIT_ENV}!", dir).unwrap(),
        "a=u!"
    );
    assert_eq!(
        substitute_ini("a={ENV:RHP_PHPT_UNSET_ENV_X}", dir).unwrap_err(),
        "ini:env:RHP_PHPT_UNSET_ENV_X"
    );
}

/// Needs php: a setting equal to php's effective default is a no-op, an
/// unknown directive never is.
#[test]
fn ini_probe_effective_values() {
    if !crate::php_available() {
        return;
    }
    let defaults: Vec<String> = PHP_DEFAULT_INI.iter().map(|s| s.to_string()).collect();
    let probe = IniProbe::new(Some(php_bin().into()), &defaults);
    assert!(probe.is_noop("allow_url_fopen", "allow_url_fopen=1"));
    assert!(probe.is_noop("zend.enable_gc", "zend.enable_gc=On"));
    // E_ALL is 30719 in php 8.5, run-tests sets 32767.
    assert!(!probe.is_noop("error_reporting", "error_reporting=E_ALL"));
    assert!(!probe.is_noop("precision", "precision=10"));
    assert!(!probe.is_noop("no_such.directive", "no_such.directive=1"));
}

#[test]
fn tree_roots() {
    assert_eq!(
        tree_root("ext/standard/tests/array/a.phpt"),
        "ext/standard/tests"
    );
    assert_eq!(tree_root("Zend/tests/x/y.phpt"), "Zend/tests");
    assert_eq!(tree_root("tests/lang/015.phpt"), "tests");
    assert_eq!(tree_root("sapi/cli/a.phpt"), "sapi/cli");
}

fn out(merged: &[u8]) -> Outcome {
    Outcome {
        out: merged.to_vec(),
        timed_out: false,
        signaled: false,
        signal: None,
    }
}

#[test]
fn php_anomalies() {
    let asan = b"x\n=================================================================\n\
==123==ERROR: AddressSanitizer: heap-use-after-free on address 0x6020 at pc 0x55 bp 0x7f\n\
READ of size 8 at 0x6020 thread T0\n\
    #0 0x55d5 in __interceptor_memcpy (/php+0x1)\n\
    #1 0x55d6 in zend_hash_find /src/Zend/zend_hash.c:123:5\n\
    #2 0x55d7 in zif_foo /src/ext/standard/foo.c:9:1\n";
    assert_eq!(
        php_anomaly(&out(asan)).as_deref(),
        Some("asan heap-use-after-free in zend_hash_find")
    );
    let ubsan = b"/src/Zend/zend_operators.c:2041:18: runtime error: signed integer overflow: 9223372036854775807 + 1 cannot be represented in type 'long'\n";
    assert_eq!(
        php_anomaly(&out(ubsan)).as_deref(),
        Some("ubsan zend_operators.c:2041: signed integer overflow: N + N cannot be represented in type 'long'")
    );
    assert_eq!(
        php_anomaly(&out(b"zend_mm_heap corrupted\n")).as_deref(),
        Some("zend_mm_heap corrupted")
    );
    let mut segv = out(b"partial");
    segv.signal = Some(11);
    segv.signaled = true;
    assert_eq!(
        php_anomaly(&segv).as_deref(),
        Some("php killed by signal 11")
    );
    // Ordinary php errors and timeouts are not php bugs.
    assert_eq!(
        php_anomaly(&out(b"PHP Fatal error:  Allowed memory size exhausted\n")),
        None
    );
    let mut slow = out(b"");
    slow.timed_out = true;
    assert_eq!(php_anomaly(&slow), None);
    // A program printing the words is not a sanitizer report.
    for text in [
        &b"parse.php:3: runtime error: something\n"[..],
        b"Bug #63305 (zend_mm_heap corrupted with traits)\n",
        b"echo ERROR: AddressSanitizer: heap-use-after-free\n",
    ] {
        assert_eq!(php_anomaly(&out(text)), None);
    }
    // Addresses differ per run (ASLR): the key must not carry them.
    let ub = |a: &str| {
        php_anomaly(&out(format!("/s/Zend/zend_execute.h:364:64: runtime error: member access within address {a} with insufficient space\n").as_bytes()))
    };
    assert_eq!(ub("0xaaaae4bf9320"), ub("0xaaaad7bf9320"));
    assert_eq!(
        ub("0x1").as_deref(),
        Some("ubsan zend_execute.h:364: member access within address ADDR with insufficient space")
    );
    // Where a stack overflow is caught varies run to run.
    let so = b"==9==ERROR: AddressSanitizer: stack-overflow on address 0xfff\n    #0 0x1 in zend_array_destroy /s/Zend/zend_hash.c:1815\n";
    assert_eq!(
        php_anomaly(&out(so)).as_deref(),
        Some("asan stack-overflow")
    );
    // The key names the first frame in php's sources, past interceptors.
    let hbo = b"==9==ERROR: AddressSanitizer: heap-buffer-overflow on address 0x1\n    #0 0x1 in memchr (/usr/lib/libasan.so+0x1)\n    #1 0x2 in php_memnstr /s/main/php.h:9:1\n";
    assert_eq!(
        php_anomaly(&out(hbo)).as_deref(),
        Some("asan heap-buffer-overflow in php_memnstr")
    );
}

#[test]
fn classification() {
    let c = |p: &[u8], r: &[u8]| classify(&out(p), &out(r));
    assert_eq!(c(b"1\n", b"1").status, Status::Match);
    let d = c(
        b"1\n\nWarning: Undefined variable $x in /a/t.php on line 3\n",
        b"1\n",
    );
    assert_eq!(d.status, Status::DiagnosticsOnly);
    assert_eq!(d.key, "diagnostics: Warning: Undefined variable $x");
    let u = c(
        b"1\n2\n",
        b"1\nRHP_UNSUPPORTED: goto statement is not supported (phase 2)\n",
    );
    assert_eq!(u.status, Status::Unsupported);
    assert_eq!(u.key, "unsupported: goto statement is not supported");
    let m = c(b"int(3)\n", b"int(4)\n");
    assert_eq!(
        (m.status, m.key.as_str()),
        (Status::Mismatch, "diff: php «int(N)» rhp «int(N)»")
    );
    let f = c(
        b"a\n",
        b"PHP Fatal error:  Uncaught Error: Call to undefined function strtok()\n",
    );
    assert_eq!(
        f.key,
        "rhp fatal: Error: Call to undefined function strtok()"
    );
    let k = c(
        b"a",
        b"thread 'main' panicked at crates/x/src/lib.rs:12:5:\n",
    );
    assert_eq!(k.status, Status::RhpCrash);
}

#[test]
fn stderr_is_compared() {
    // rhp fatals in a trailing silent statement: same stdout, but the
    // merged stream differs and php printed no diagnostic -> not a match.
    let v = classify(
        &out(b"done\n"),
        &out(b"done\nPHP Fatal error:  Uncaught Error: Call to undefined function cleanup()\n"),
    );
    assert_eq!(v.status, Status::Mismatch);
    assert_eq!(
        v.key,
        "rhp fatal: Error: Call to undefined function cleanup()"
    );
}

#[test]
fn blank_line_differences_are_not_diagnostics() {
    let v = classify(
        &out(b"a\n\nb\n\nWarning: x in /t.php on line 1\n"),
        &out(b"a\nb\n"),
    );
    assert_eq!(v.status, Status::Mismatch);
}

#[test]
fn cluster_key_keeps_later_numbers() {
    assert_eq!(cluster_key("", "int(3) and 42"), "int(N) and N");
}

#[test]
fn run_proc_merges_streams_and_survives_background_children() {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "echo out; echo err >&2; (sleep 30 &) ; echo end"]);
    let started = Instant::now();
    let o = run_proc(cmd, None, Duration::from_secs(5));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(String::from_utf8_lossy(&o.out), "out\nerr\nend\n");
    assert!(!o.timed_out && !o.signaled);
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "sleep 30"]);
    assert!(run_proc(cmd, None, Duration::from_millis(200)).timed_out);
}

#[test]
fn uncaught_exception_trace_counts_as_diagnostic() {
    let php =
        b"start\nPHP Fatal error:  Uncaught Error: Call to undefined function f() in /t.php:3\n\
                    Stack trace:\n#0 {main}\n  thrown in /t.php on line 3\n";
    assert_eq!(strip_diagnostics(php), "start");
    let php = b"start\n\nFatal error: Uncaught Exception: a in /t.php:2\nStack trace:\n#0 {main}\n\n\
                    Next RuntimeException: b in /t.php:4\nStack trace:\n#0 {main}\n  thrown in /t.php on line 4\n";
    assert_eq!(strip_diagnostics(php), "start");
    // A `#`-line that isn't part of a trace is real output.
    assert_eq!(strip_diagnostics(b"#1 not a frame\n"), "#1 not a frame");
}

#[test]
fn restore_removes_only_new_paths() {
    use std::os::unix::fs::PermissionsExt;
    let d = std::env::temp_dir().join(format!("rhp-phpt-restore-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("keep_dir")).unwrap();
    fs::write(d.join("keep.txt"), b"x").unwrap();
    let snap = snapshot(&d);
    fs::write(d.join("new.txt"), b"y").unwrap();
    fs::write(d.join("keep_dir/new_inner.txt"), b"y").unwrap();
    fs::create_dir_all(d.join("new_dir/sub")).unwrap();
    fs::write(d.join("new_dir/sub/f"), b"z").unwrap();
    fs::set_permissions(d.join("new_dir"), fs::Permissions::from_mode(0o000)).unwrap();
    restore(&d, &snap);
    assert_eq!(snapshot(&d).paths, snap.paths);
    assert_eq!(fs::read(d.join("keep.txt")).unwrap(), b"x");
    // A run that puts a directory's mtime back is still caught (ctime).
    let mtime = fs::metadata(d.join("keep_dir"))
        .unwrap()
        .modified()
        .unwrap();
    fs::write(d.join("keep_dir/sneaky.txt"), b"y").unwrap();
    fs::File::open(d.join("keep_dir"))
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    restore(&d, &snap);
    assert_eq!(snapshot(&d).paths, snap.paths);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn expectf_translation_matches_run_tests() {
    assert_eq!(preg_quote("a.b/c(d)"), "a\\.b\\/c\\(d\\)");
    assert_eq!(expectf_to_regex("int(%d)"), "int\\(\\d+\\)");
    assert_eq!(expectf_to_regex("x%r[ab]+%ry."), "x([ab]+)y\\.");
    assert_eq!(
        expectf_to_regex("%s|%S|%a|%A"),
        "[^\\r\\n]+\\|[^\\r\\n]*\\|.+?\\|.*?"
    );
    assert_eq!(expectf_to_regex("100%"), "100%"); // lone % is literal
}

#[test]
fn spec_fallback_accepts_unspecified_values() {
    let p = Phpt::parse(b"--FILE--\n<?php\n--EXPECTREGEX--\nint\\(-[1-9][0-9]*\\)\n");
    assert_eq!(spec_matches(&p, b"int(-1)\n"), Some(true));
    assert_eq!(spec_matches(&p, b"int(-32)"), Some(true));
    assert_eq!(spec_matches(&p, b"int(1)"), Some(false));
    let p = Phpt::parse(b"--FILE--\n<?php\n--EXPECTF--\nfloat(%f) at %s\nDone\n");
    assert_eq!(spec_matches(&p, b"float(1.5) at x.php\nDone"), Some(true));
    assert_eq!(spec_matches(&p, b"float(1.5) at x.php\n"), Some(false));
    let p = Phpt::parse(b"--FILE--\n<?php\n--EXPECT--\n1\n");
    assert_eq!(spec_matches(&p, b"1"), None);
}

#[test]
fn portability_heuristic() {
    assert!(is_portable(b"<?php var_dump(1 + 2);"));
    assert!(!is_portable(b"<?php include 'x.inc';"));
    assert!(!is_portable(b"<?php echo __DIR__;"));
}

#[test]
fn json_escaping() {
    assert_eq!(json_str("a\"b\\c\nd\u{1}"), "\"a\\\"b\\\\c\\nd\\u0001\"");
}

#[test]
fn cgi_detection_follows_run_tests() {
    let cgi = |s: &[u8]| Phpt::parse(s).is_cgi();
    assert!(cgi(b"--CGI--\n--FILE--\n<?php\n"));
    assert!(cgi(b"--GET--\na=1\n--FILE--\n<?php\n"));
    assert!(cgi(b"--EXPECTHEADERS--\nStatus: 404\n--FILE--\n<?php\n"));
    assert!(!cgi(b"--GET--\n--FILE--\n<?php\n"));
    assert!(!cgi(b"--FILE--\n<?php\n"));
}

#[test]
fn cgi_requests_match_run_tests() {
    let script = Path::new("/w/t.php");
    let env = |r: &CgiRequest, k: &str| r.env.get(k).cloned().unwrap_or_default();

    let p = Phpt::parse(b"--GET--\na=1&b=2\n--COOKIE--\nc=3\n--FILE--\n<?php\n");
    let r = cgi_request(&p, script, &[]).unwrap();
    assert_eq!(env(&r, "REQUEST_METHOD"), "GET");
    assert_eq!(env(&r, "QUERY_STRING"), "a=1&b=2");
    assert_eq!(env(&r, "HTTP_COOKIE"), "c=3");
    assert_eq!(env(&r, "SCRIPT_FILENAME"), "/w/t.php");
    assert_eq!(env(&r, "PATH_TRANSLATED"), "/w/t.php");
    assert_eq!(env(&r, "REDIRECT_STATUS"), "1");
    assert!(r.env.contains_key("CONTENT_LENGTH") && r.body.is_none());
    assert!(cgi_passes_args(&r), "a GET gets `-- <ARGS>`");

    let p = Phpt::parse(b"--POST--\nx=1\n\n--FILE--\n<?php\n");
    let r = cgi_request(&p, script, &[]).unwrap();
    assert!(!cgi_passes_args(&r), "run-tests drops ARGS for a POST");
    assert_eq!(env(&r, "REQUEST_METHOD"), "POST");
    assert_eq!(env(&r, "CONTENT_TYPE"), "application/x-www-form-urlencoded");
    assert_eq!(env(&r, "CONTENT_LENGTH"), "3");
    assert_eq!(r.body.as_deref(), Some(&b"x=1"[..]));

    let p = Phpt::parse(
        b"--POST_RAW--\nContent-Type: multipart/form-data; boundary=x1\r\n--x1\r\nbody\r\n--x1--\n--FILE--\n<?php\n",
    );
    let r = cgi_request(&p, script, &[("REQUEST_METHOD".into(), "PUT".into())]).unwrap();
    assert_eq!(env(&r, "CONTENT_TYPE"), "multipart/form-data; boundary=x1");
    assert_eq!(env(&r, "REQUEST_METHOD"), "PUT"); // ENV wins where run-tests only fills
    assert_eq!(r.body.as_deref(), Some(&b"--x1\r\nbody\r\n--x1--"[..]));
    assert_eq!(env(&r, "CONTENT_LENGTH"), "18");

    let p = Phpt::parse(b"--GZIP_POST--\na=b\n--FILE--\n<?php\n");
    let r = cgi_request(&p, script, &[]).unwrap();
    assert_eq!(env(&r, "HTTP_CONTENT_ENCODING"), "gzip");
    let body = r.body.unwrap();
    assert_eq!(&body[..2], b"\x1f\x8b");
    let mut plain = Vec::new();
    flate2::read::GzDecoder::new(&body[..])
        .read_to_end(&mut plain)
        .unwrap();
    assert_eq!(plain, b"a=b");
}

#[test]
fn cgi_output_splits_and_version_is_rewritten() {
    let out =
        b"X-Powered-By: PHP/8.5.9\r\nContent-type: text/html; charset=UTF-8\r\n\r\nbody PHP/8.5.9";
    let (h, b) = split_cgi(out).unwrap();
    assert_eq!(
        h,
        &b"X-Powered-By: PHP/8.5.9\r\nContent-type: text/html; charset=UTF-8"[..]
    );
    assert_eq!(b, b"body PHP/8.5.9");
    assert_eq!(split_cgi(b"a\n\nb").unwrap(), (&b"a"[..], &b"b"[..]));
    assert!(split_cgi(b"no blank line\r\n").is_none());
    let mut v = out.to_vec();
    normalise_powered_by(&mut v, "8.5.9", "8.5.8");
    assert!(v.starts_with(b"X-Powered-By: PHP/8.5.8\r\n"));
    assert!(v.ends_with(b"body PHP/8.5.9"), "the body is left alone");
}

#[test]
fn capture_stdio_follows_run_tests() {
    let p = Phpt::parse(b"--CAPTURE_STDIO--\nstdin STDERR\n--FILE--\n<?php\n");
    assert_eq!(
        Capture::of(&p),
        Capture {
            stdin: true,
            stdout: false,
            stderr: true
        }
    );
    assert_eq!(
        Capture::of(&Phpt::parse(b"--FILE--\n<?php\n")),
        Capture::ALL
    );
}

#[test]
fn uncaptured_streams_are_a_terminal() {
    let script = "[ -t 0 ] && echo in-tty; [ -t 1 ] && echo out-tty >&2; [ -t 2 ] && echo err-tty; echo out; echo err >&2";
    let run = |capture| {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", script]);
        let o = run_proc_capturing(cmd, None, Duration::from_secs(5), capture);
        String::from_utf8_lossy(&o.out).into_owned()
    };
    let only = |stdin, stdout, stderr| Capture {
        stdin,
        stdout,
        stderr,
    };
    assert_eq!(run(Capture::ALL), "out\nerr\n");
    // Only stderr captured: stdin and stdout are the pty.
    assert_eq!(run(only(false, false, true)), "out-tty\nerr\n");
    // Only stdout captured.
    assert_eq!(run(only(true, true, false)), "err-tty\nout\n");
    assert_eq!(run(only(false, false, false)), "");
}

#[test]
fn conflicts_and_flakiness_follow_run_tests() {
    assert_eq!(
        parse_conflicts(b"# comment\nall\n  server # why\n"),
        ["all", "server"]
    );
    let p = |s: &str| Phpt::parse(s.as_bytes());
    assert!(is_flaky(&p("--FLAKY--\nwhy\n--FILE--\n<?php\n")));
    assert!(is_flaky(&p("--FILE--\n<?php usleep(1);\n")));
    assert!(is_flaky(&p("--FILE--\n<?php $t = HRTIME();\n")));
    assert!(!is_flaky(&p("--FILE--\n<?php my_sleep(1);\n")));
    assert!(is_flaky(&p(
        "--SKIPIF--\n<?php getenv('SKIP_PERF_SENSITIVE');\n--FILE--\n<?php\n"
    )));
    assert!(is_flaky_output(b"Error: Connection refused."));
    assert!(!is_flaky_output(b"timed outs"));
}

#[test]
fn shell_quoting_matches_php() {
    assert_eq!(escapeshellarg("/a b/it's"), r"'/a b/it'\''s'");
    assert_eq!(addslashes(r#"a"b'c\d"#), r#"a\"b\'c\\d"#);
}
