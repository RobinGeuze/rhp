//! Compiles the vendored C into static archives, one per enabled feature.
//! Optimisation is fixed (php builds with -O2, libargon2 with -O3) so debug
//! builds hash at release speed; the output bytes don't depend on it. Each
//! archive force-includes glue/prefix_<lib>.h, which gives every global symbol
//! an rhp_ prefix.

use std::path::{Path, PathBuf};

fn main() {
    let crypt = std::env::var_os("CARGO_FEATURE_CRYPT").is_some();
    let argon2 = std::env::var_os("CARGO_FEATURE_ARGON2").is_some();
    let timelib = std::env::var_os("CARGO_FEATURE_TIMELIB").is_some();
    println!("cargo:rerun-if-changed=build.rs");

    if crypt {
        php_src(&[
            "ext/standard/crypt_blowfish.c",
            "ext/standard/crypt_freesec.c",
            "ext/standard/crypt_sha256.c",
            "ext/standard/crypt_sha512.c",
        ])
        .flag("-include")
        .flag(glue("prefix_crypt.h"))
        .file("glue/crypt.c")
        .compile("rhp_csrc_crypt");
        rerun(&["shim", "glue", "vendor/php-src"]);
    }

    if timelib {
        // ext/date/config0.m4: timelib's sources and PHP_TIMELIB_CFLAGS
        // (-fwrapv: timelib relies on signed overflow wrapping), plus php's
        // global -ffp-contract=off (astro.c's floating point). It builds
        // without the Zend shim; glue/timelib_config.h stands in for php's
        // generated timelib_config.h.
        let lib = Path::new("vendor/php-src/ext/date/lib");
        let mut b = cc::Build::new();
        b.include("glue")
            .include(lib)
            .opt_level(2)
            .warnings(false)
            .define("HAVE_TIMELIB_CONFIG_H", "1")
            .flag("-fwrapv")
            .flag("-ffp-contract=off")
            .flag("-fvisibility=hidden")
            .flag("-include")
            .flag(glue("prefix_timelib.h"));
        for f in [
            "astro.c",
            "dow.c",
            "parse_date.c",
            "parse_tz.c",
            "parse_posix.c",
            "timelib.c",
            "tm2unixtime.c",
            "unixtime2tm.c",
            "parse_iso_intervals.c",
            "interval.c",
        ] {
            b.file(lib.join(f));
        }
        b.file("glue/timelib.c").compile("rhp_csrc_timelib");
        rerun(&["glue", "vendor/php-src"]);
    }

    if argon2 {
        let dir = Path::new("vendor/argon2");
        let mut b = cc::Build::new();
        b.include(dir.join("include"))
            .include(dir.join("src"))
            .opt_level(3)
            .warnings(false)
            .flag("-pthread")
            .flag("-fvisibility=hidden")
            .flag("-include")
            .flag(glue("prefix_argon2.h"));
        for f in [
            "src/argon2.c",
            "src/core.c",
            "src/blake2/blake2b.c",
            "src/thread.c",
            "src/encoding.c",
            "src/ref.c",
        ] {
            b.file(dir.join(f));
        }
        b.file("glue/argon2.c").compile("rhp_csrc_argon2");
        rerun(&["glue", "vendor/argon2"]);
    }
}

/// A build of `files` (paths under vendor/php-src) plus the shared Zend shim,
/// with the flags php's configure adds (-ffp-contract=off,
/// -fvisibility=hidden) and -fwrapv (php-src's timelib flag, harmless here).
fn php_src(files: &[&str]) -> cc::Build {
    let vendor = Path::new("vendor/php-src");
    let mut b = cc::Build::new();
    b.include("shim")
        .include(vendor.join("ext/standard"))
        .opt_level(2)
        .warnings(false)
        .flag("-fwrapv")
        .flag("-ffp-contract=off")
        .flag("-fvisibility=hidden")
        .file("shim/zend_shim.c");
    for f in files {
        b.file(vendor.join(f));
    }
    b
}

fn glue(header: &str) -> PathBuf {
    Path::new(&std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("glue")
        .join(header)
}

fn rerun(dirs: &[&str]) {
    for d in dirs {
        println!("cargo:rerun-if-changed={d}");
    }
}
