//! Startup (`-d`) ini values run their directive's on_modify handler, as
//! php's module startup does: a sweep of memory_hard_limit_percent,
//! openssl.libctx and bcmath.scale values (the first two crossed with the
//! display settings), compared with `php -n` (stdout, stderr and exit
//! status) when the local php is rhp's release. The fixed cases are CLI
//! fixtures (`tests/cli/startup_ini`).

use std::process::Command;

use rhp_difftest::php_bin;

const SCRIPT: &str = r#"echo "start\n";
var_dump(ini_get('memory_hard_limit_percent'));
var_dump(ini_get_all(null, false)['memory_hard_limit_percent']);
var_dump(error_get_last());
var_dump(ini_set('memory_hard_limit_percent', '200'));"#;

const BCMATH_SCRIPT: &str = r#"var_dump(bcscale(), ini_get('bcmath.scale'));
echo bcadd('1', '2'), "\n";
ini_set('bcmath.scale', '2');
var_dump(bcscale());
ini_restore('bcmath.scale');
var_dump(bcscale(), ini_get_all('bcmath')['bcmath.scale']['global_value']);"#;

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

fn same_release() -> bool {
    let same = Command::new(php_bin())
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
    let has_openssl = Command::new(php_bin())
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
                run_script(&php_bin(), &ini, LIBCTX_SCRIPT),
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
            assert_eq!(run(&php_bin(), &ini), run(RHP, &ini), "-d {ini:?}");
        }
    }
}

#[test]
fn bcmath_scale_matches_php() {
    if !same_release() {
        return;
    }
    for v in ["4", "-1", "abc", "3x", " 7 ", "2147483648", "1k", ""] {
        let kv = format!("bcmath.scale={v}");
        assert_eq!(
            run_script(&php_bin(), &[&kv], BCMATH_SCRIPT),
            run_script(RHP, &[&kv], BCMATH_SCRIPT),
            "-d {kv}"
        );
    }
}
