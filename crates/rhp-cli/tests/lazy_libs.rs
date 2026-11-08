//! Libraries rhp opens on first use (docs/design/libraries.md,
//! `Mode::Loaded`) stay unloaded in a process that does not use them: the
//! dynamic loader's own trace (macOS `DYLD_PRINT_LIBRARIES`, glibc
//! `LD_DEBUG=files`) names every library it loads.

use std::process::Command;

fn loads(code: &str, lib: &str) -> bool {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rhp"));
    cmd.args(["-n", "-r", code]);
    if cfg!(target_os = "macos") {
        cmd.env("DYLD_PRINT_LIBRARIES", "1");
    } else {
        cmd.env("LD_DEBUG", "files");
    }
    let out = cmd.output().expect("run rhp");
    assert!(out.status.success(), "rhp -r {code:?} fails: {out:?}");
    String::from_utf8_lossy(&out.stderr).contains(lib)
}

#[test]
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
fn libgd_opens_on_first_use() {
    assert!(!loads("echo 1;", "libgd"), "rhp loads libgd at startup");
    assert!(
        !loads("echo function_exists('imagecreatefrompng');", "libgd"),
        "registering ext/gd loads libgd"
    );
    assert!(
        loads("imagecreatetruecolor(1, 1);", "libgd"),
        "the trace does not show libgd loading"
    );
}
