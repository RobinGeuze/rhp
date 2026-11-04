//! Compiles libmbfl and php_unicode.c (vendor/php-src, written by
//! crates/rhp-csrc/tools/sync-php-src-c.sh) plus glue/mbfl.c into one static
//! archive, against rhp-csrc's shared Zend shim, with php's flags. Every C
//! file force-includes glue/prefix_mbfl.h, which gives each global symbol an
//! rhp_ prefix.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=glue");
    println!("cargo:rerun-if-changed=vendor/php-src");
    let shim = std::env::var_os("DEP_RHP_CSRC_SHIM").expect("rhp-csrc exports its shim directory");
    // For the test that no symbol is defined both here and in rhp-csrc.
    let csrc_out = std::env::var("DEP_RHP_CSRC_OUT").expect("rhp-csrc exports its OUT_DIR");
    println!("cargo:rustc-env=RHP_CSRC_OUT_DIR={csrc_out}");
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let mbstring = manifest.join("vendor/php-src/ext/mbstring");
    let libmbfl = mbstring.join("libmbfl");

    // The .c files php's config.m4 lists are every one under libmbfl.
    let mut files = Vec::new();
    for dir in ["mbfl", "filters", "nls"] {
        for entry in std::fs::read_dir(libmbfl.join(dir)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension() == Some("c".as_ref()) {
                files.push(path);
            }
        }
    }
    files.sort();
    files.push(mbstring.join("php_unicode.c"));

    let mut b = cc::Build::new();
    // php's include path for mbstring: the shim stands in for php's own
    // headers (php.h, zend.h, mbstring.h, libmbfl/config.h).
    b.include(&shim)
        .include(&mbstring)
        .include(&libmbfl)
        .include(libmbfl.join("mbfl"))
        .opt_level(2)
        .warnings(false)
        .flag("-fwrapv")
        .flag("-ffp-contract=off")
        .flag("-fvisibility=hidden")
        .flag("-include")
        .flag(manifest.join("glue/prefix_mbfl.h"));
    b.files(&files).file(Path::new("glue/mbfl.c"));
    b.compile("rhp_mbfl");
}
