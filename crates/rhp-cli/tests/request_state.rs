//! Builtin per-request state (ini overrides, default timezone, precision,
//! strtok / preg / json state, putenv) must not leak from one engine to the
//! next one created on the same thread, and putenv must not touch the process
//! environment.

use rhp_compiler::compile_with_source;
use rhp_frontend::Arena;
use rhp_oracle::Interp;
use rhp_vm::Vm;

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

const CLEAN: &str = "0.33333333333333 0.1 a=1&b=2 14 32767 UTC false 0 0 false";

fn run_vm(source: &[u8]) -> String {
    let arena = Arena::new();
    let parsed = arena.parse(b"t.php", source);
    let program = compile_with_source(parsed.program(), source, "t.php").expect("compile");
    let mut vm = Vm::new();
    vm.run(&program).expect("run");
    String::from_utf8_lossy(vm.output()).into_owned()
}

fn run_oracle(source: &[u8]) -> String {
    let arena = Arena::new();
    let parsed = arena.parse(b"t.php", source);
    let mut interp = Interp::new();
    if let Err(e) = interp.run(parsed.program()) {
        panic!("{e:?}");
    }
    String::from_utf8_lossy(interp.output()).into_owned()
}

#[test]
fn vm_state_does_not_leak_between_engines() {
    let clean = run_vm(CHECK);
    assert_eq!(clean, CLEAN);
    assert_eq!(run_vm(DIRTY), "2 0 dirty");
    assert_eq!(run_vm(CHECK), clean);
    assert!(std::env::var_os("RHP_REQUEST_STATE_TEST").is_none());
}

/// The oracle has no preg support: drop those parts of the scripts.
fn without_preg(src: &[u8]) -> Vec<u8> {
    String::from_utf8_lossy(src)
        .lines()
        .filter(|l| !l.starts_with("preg_") && !l.starts_with("echo preg_"))
        .map(|l| l.replace("preg_last_error(), ' ', ", ""))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

#[test]
fn oracle_state_does_not_leak_between_engines() {
    let check = without_preg(CHECK);
    let clean = run_oracle(&check);
    assert_eq!(clean, CLEAN.replacen(" 0 0 ", " 0 ", 1));
    assert_eq!(run_oracle(&without_preg(DIRTY)), "0 dirty");
    assert_eq!(run_oracle(&check), clean);
    assert!(std::env::var_os("RHP_REQUEST_STATE_TEST").is_none());
}

#[test]
fn clean_state_matches_php_defaults() {
    // php -n prints the same except error_reporting (30719 = E_ALL); rhp keeps
    // the phpt harness's 32767 as its untouched default.
    assert_eq!(run_vm(CHECK), CLEAN);
}
