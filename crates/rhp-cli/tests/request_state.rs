//! Builtin per-request state (ini overrides, default timezone, precision,
//! strtok / preg / json state, putenv) must not leak from one engine to the
//! next one created on the same thread, and putenv must not touch the process
//! environment.

use rhp_sapi::{CaptureSapi, Request, Script};

// The engine's memory accounting (memory_get_usage(), memory_limit).
#[global_allocator]
static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;

const DIRTY: &[u8] = br#"<?php
ini_set('precision', 5);
ini_set('serialize_precision', 3);
ini_set('arg_separator.output', ';');
ini_set('allow_url_fopen', '0');
error_reporting(0);
date_default_timezone_set('America/New_York');
strtok('a b c', ' ');
preg_replace('/(?:\D+|<\d+>)*[!?]/', '', 'foobar foobar foobar');
echo preg_last_error(), ' ';
json_decode('{');
putenv('RHP_REQUEST_STATE_TEST=dirty');
echo error_reporting(), ' dirty';
"#;

const CHECK: &[u8] = br#"<?php
echo 1/3, ' ', json_encode(0.1), ' ', http_build_query(['a' => 1, 'b' => 2]), ' ',
    ini_get('precision'), ' ', error_reporting(), ' ',
    date_default_timezone_get(), ' ', var_export(strtok(' '), true), ' ',
    preg_last_error(), ' ', json_last_error(), ' ',
    var_export(getenv('RHP_REQUEST_STATE_TEST'), true);
"#;

const CLEAN: &str = "0.33333333333333 0.1 a=1&b=2 14 30719 UTC false 0 0 false";

fn run_vm(source: &[u8]) -> String {
    let script = Script::Source {
        path: "t.php".to_string(),
        bytes: source.to_vec(),
    };
    let (_, out) = CaptureSapi::cli().run(Request::new(script));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn vm_state_does_not_leak_between_engines() {
    let clean = run_vm(CHECK);
    assert_eq!(clean, CLEAN);
    assert_eq!(run_vm(DIRTY), "2 0 dirty");
    assert_eq!(run_vm(CHECK), clean);
    assert!(std::env::var_os("RHP_REQUEST_STATE_TEST").is_none());
}

#[test]
fn clean_state_matches_php_defaults() {
    // Exactly what `php -n` prints (error_reporting = E_ALL = 30719; the
    // phpt harness passes its 32767 with -d, which rhp honours).
    assert_eq!(run_vm(CHECK), CLEAN);
}
