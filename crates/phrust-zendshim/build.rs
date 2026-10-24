//! Compile the Zend-shim glue and the vendored extension C against our own
//! `zend_shim.h` (never against php). The resulting objects are linked into this
//! Rust crate as a static archive; the bridge in `src/lib.rs` calls the
//! extension entry points (`zif_*`) directly.

use std::path::PathBuf;

fn main() {
    // Declare the custom cfg so the unexpected_cfgs lint stays quiet.
    println!("cargo:rustc-check-cfg=cfg(zendshim_bin2hex)");

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let include = manifest.join("include");
    let csrc = manifest.join("csrc");

    let mut build = cc::Build::new();
    build
        .include(&include)
        .warnings(true)
        .flag_if_supported("-std=c11")
        .file(csrc.join("shim.c"))
        .file(csrc.join("demo_ext.c"));

    // Stage 2: a real php-src function, compiled from its actual source, enabled
    // only when the vendored file is present (it is checked into the repo).
    let bin2hex = csrc.join("php_bin2hex.c");
    if bin2hex.exists() {
        build.file(&bin2hex);
        println!("cargo:rustc-cfg=zendshim_bin2hex");
    }
    println!("cargo:rerun-if-changed={}", bin2hex.display());

    build.compile("phrust_zendshim_c");

    println!("cargo:rerun-if-changed={}", csrc.join("shim.c").display());
    println!("cargo:rerun-if-changed={}", csrc.join("demo_ext.c").display());
    println!("cargo:rerun-if-changed={}", include.join("zend_shim.h").display());
}
