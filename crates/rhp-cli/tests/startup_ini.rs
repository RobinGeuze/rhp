//! Startup (`-d`) ini values run their directive's on_modify handler, as
//! php's module startup does: memory_hard_limit_percent warns "in Unknown on
//! line 0" and a refused value leaves the directive NULL; a bad
//! openssl.libctx is an E_ERROR that leaves "custom" and exit status 255
//! while the script still runs. Checked against fixed output and, when the
//! local php is the same release as rhp, against `php -n` itself (stdout,
//! stderr and exit status).

use std::process::Command;

const SCRIPT: &str = r#"echo "start\n";
var_dump(ini_get('memory_hard_limit_percent'));
var_dump(ini_get_all(null, false)['memory_hard_limit_percent']);
var_dump(error_get_last());
var_dump(ini_set('memory_hard_limit_percent', '200'));"#;

const LIBCTX_SCRIPT: &str = r#"echo "start\n";
var_dump(ini_get('openssl.libctx'), error_get_last(), ini_set('openssl.libctx', 'default'));"#;

fn run(bin: &str, ini: &[&str]) -> (String, String, Option<i32>) {
    run_script(bin, ini, SCRIPT)
}

fn run_script(bin: &str, ini: &[&str], script: &str) -> (String, String, Option<i32>) {
    let mut c = Command::new(bin);
    c.arg("-n");
    for kv in ini {
        c.args(["-d", kv]);
    }
    let o = c.args(["-r", script]).output().expect("run engine");
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
        o.status.code(),
    )
}

const RHP: &str = env!("CARGO_BIN_EXE_rhp");

#[test]
fn refused_startup_value_warns_and_stays_null() {
    let (out, err, code) = run(RHP, &["memory_hard_limit_percent=50"]);
    assert_eq!(
        out,
        "\nWarning: memory_hard_limit_percent must be >= 100, or -1 in Unknown on line 0\n\
         start\nstring(0) \"\"\nNULL\nNULL\nstring(0) \"\"\n"
    );
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

#[test]
fn bad_openssl_libctx_is_a_startup_fatal() {
    let (out, err, code) = run_script(RHP, &["openssl.libctx=bogus"], LIBCTX_SCRIPT);
    assert_eq!(
        out,
        "\nFatal error: PHP Startup: OpenSSL libctx \"bogus\" cannot be found in Unknown on line 0\n\
         Stack trace:\n#0 {main}\nstart\nstring(6) \"custom\"\nNULL\nbool(false)\n"
    );
    assert_eq!(err, "");
    assert_eq!(code, Some(255));
}

fn same_release() -> bool {
    let same = Command::new("php")
        .args(["-n", "-r", "echo PHP_VERSION;"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| o.stdout)
        == Command::new(RHP)
            .args(["-n", "-r", "echo PHP_VERSION;"])
            .output()
            .ok()
            .map(|o| o.stdout);
    if !same {
        eprintln!("skipping: php is missing or not rhp's release");
    }
    same
}

#[test]
fn openssl_libctx_matches_php() {
    if !same_release() {
        return;
    }
    let has_openssl = Command::new("php")
        .args(["-n", "-r", "echo extension_loaded('openssl') ? 1 : 0;"])
        .output()
        .is_ok_and(|o| o.stdout == b"1");
    if !has_openssl {
        eprintln!("skipping: php -n has no openssl");
        return;
    }
    for v in ["bogus", "default", "custom", "", "Custom"] {
        let kv = format!("openssl.libctx={v}");
        for extra in [
            &[][..],
            &["display_errors=0"],
            &["display_errors=stderr"],
            &["display_startup_errors=0"],
            &["html_errors=1"],
            &["fatal_error_backtraces=0"],
            &["error_reporting=0"],
            &["error_reporting=0", "display_errors=0"],
        ] {
            let mut ini = vec![kv.as_str()];
            ini.extend_from_slice(extra);
            assert_eq!(
                run_script("php", &ini, LIBCTX_SCRIPT),
                run_script(RHP, &ini, LIBCTX_SCRIPT),
                "-d {ini:?}"
            );
        }
    }
}

#[test]
fn startup_ini_matches_php() {
    if !same_release() {
        return;
    }
    for v in [
        "50",
        "abc",
        "150",
        "-1",
        "",
        "1e3",
        "0x",
        "99999999999999999999",
        "9223372036854775808",
    ] {
        let kv = format!("memory_hard_limit_percent={v}");
        for extra in [
            &[][..],
            &["display_errors=0"],
            &["display_errors=stderr"],
            &["display_startup_errors=0"],
            &["html_errors=1"],
        ] {
            let mut ini = vec![kv.as_str()];
            ini.extend_from_slice(extra);
            assert_eq!(run("php", &ini), run(RHP, &ini), "-d {ini:?}");
        }
    }
}
