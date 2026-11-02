//! Differential test for the output layer's startup ini (`-d output_buffering`,
//! `-d output_handler`): run `php -n -d …` and the built `rhp -d …` on the same
//! script, comparing stdout and the exit code. Skips when no `php` is on PATH.

use std::process::Command;

fn php_available() -> bool {
    Command::new("php")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

const SCRIPT: &str =
    r#"echo "abc\n"; var_dump(ini_get("output_buffering"), ob_get_level(), ob_list_handlers());"#;

fn run(bin: &str, ini: &str) -> (Vec<u8>, Option<i32>) {
    let mut c = Command::new(bin);
    if bin == "php" {
        c.arg("-n");
    }
    let o = c
        .args(["-d", ini, "-r", SCRIPT])
        .output()
        .expect("run engine");
    (o.stdout, o.status.code())
}

#[test]
fn output_startup_ini_matches_php() {
    if !php_available() {
        eprintln!("skipping: no php on PATH");
        return;
    }
    for ini in [
        "output_buffering=On",
        "output_buffering=off",
        "output_buffering=1",
        "output_buffering=\"1\"",
        "output_handler=strtoupper",
        "output_handler=no_such_function",
        "output_handler=NoSuchClass::method",
    ] {
        let php = run("php", ini);
        let rhp = run(env!("CARGO_BIN_EXE_rhp"), ini);
        assert_eq!(
            String::from_utf8_lossy(&php.0),
            String::from_utf8_lossy(&rhp.0),
            "stdout differs for -d {ini}"
        );
        assert_eq!(php.1, rhp.1, "exit code differs for -d {ini}");
    }
}
