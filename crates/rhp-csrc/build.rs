//! Compiles the vendored C into static archives, one per enabled feature,
//! plus the out-of-line half of the Zend shim. Optimisation is fixed (php
//! builds with -O2, libargon2 with -O3) so debug builds hash at release
//! speed; the output bytes don't depend on it. Each archive force-includes
//! glue/prefix_<lib>.h, which gives every global symbol an rhp_ prefix (the
//! shim names its own symbols rhp_ directly).

use std::path::{Path, PathBuf};

fn main() {
    let crypt = std::env::var_os("CARGO_FEATURE_CRYPT").is_some();
    let argon2 = std::env::var_os("CARGO_FEATURE_ARGON2").is_some();
    let timelib = std::env::var_os("CARGO_FEATURE_TIMELIB").is_some();
    let avifinfo = std::env::var_os("CARGO_FEATURE_AVIFINFO").is_some();
    let fileinfo = std::env::var_os("CARGO_FEATURE_FILEINFO").is_some();
    let cli_server = std::env::var_os("CARGO_FEATURE_CLI_SERVER").is_some();
    let lexbor = std::env::var_os("CARGO_FEATURE_LEXBOR").is_some();
    let calendar = std::env::var_os("CARGO_FEATURE_CALENDAR").is_some();
    println!("cargo:rerun-if-changed=build.rs");

    // The shim, for every crate that compiles php-src C (rhp-mbfl-sys reads
    // it as DEP_RHP_CSRC_SHIM). Its symbols live here, once.
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:shim={}", manifest.join("shim").display());
    // Where the archives go, for rhp-mbfl-sys's symbol test.
    println!("cargo:out={}", std::env::var("OUT_DIR").unwrap());

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
        rerun(&["glue", "vendor/php-src"]);
    }

    if timelib {
        // ext/date/config0.m4: timelib's sources and PHP_TIMELIB_CFLAGS
        // (-fwrapv: timelib relies on signed overflow wrapping), plus php's
        // global -ffp-contract=off (astro.c's floating point). It builds
        // without the Zend shim's headers; glue/timelib_config.h stands in
        // for php's generated timelib_config.h and points timelib's
        // allocator at the shim's emalloc family.
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

    if avifinfo {
        // libavifinfo, which ext/standard/image.c bundles: plain C99 with
        // no php headers.
        let dir = Path::new("vendor/php-src/ext/standard/libavifinfo");
        let mut b = cc::Build::new();
        b.include(dir)
            .opt_level(2)
            .warnings(false)
            .flag("-fvisibility=hidden")
            .flag("-include")
            .flag(glue("prefix_avifinfo.h"))
            .file(dir.join("avifinfo.c"))
            .compile("rhp_csrc_avifinfo");
        rerun(&["glue", "vendor/php-src"]);
    }

    if fileinfo {
        // ext/fileinfo/config.m4: libmagic's sources (strcasestr.c only
        // where the C library lacks it) with -I libmagic, against the shim.
        // glue/fileinfo_config.h stands in for php_config.h's macros.
        let dir = Path::new("vendor/php-src/ext/fileinfo");
        let mut b = php_cc();
        b.include(dir.join("libmagic"))
            .include(dir)
            .flag("-include")
            .flag(glue("fileinfo_config.h"))
            .flag("-include")
            .flag(glue("prefix_fileinfo.h"));
        for f in [
            "apprentice.c",
            "apptype.c",
            "ascmagic.c",
            "buffer.c",
            "cdf_time.c",
            "cdf.c",
            "compress.c",
            "der.c",
            "encoding.c",
            "fsmagic.c",
            "funcs.c",
            "is_csv.c",
            "is_json.c",
            "is_simh.c",
            "is_tar.c",
            "magic.c",
            "print.c",
            "readcdf.c",
            "softmagic.c",
        ] {
            b.file(dir.join("libmagic").join(f));
        }
        b.file(dir.join("php_libmagic.c"))
            .file("glue/fileinfo.c")
            .compile("rhp_csrc_fileinfo");
        rerun(&["glue", "vendor/php-src"]);
    }

    if cli_server {
        // sapi/cli's HTTP request parser and mime type table, for rhp -S:
        // plain C with no php headers (glue/cli_server stands in for
        // php_config.h).
        let dir = Path::new("vendor/php-src/sapi/cli");
        let mut b = cc::Build::new();
        b.include(dir)
            .include("glue/cli_server")
            .opt_level(2)
            .warnings(false)
            .flag("-fvisibility=hidden")
            .flag("-include")
            .flag(glue("prefix_cli_server.h"))
            .file(dir.join("php_http_parser.c"))
            .file("glue/cli_server.c")
            .compile("rhp_csrc_cli_server");
        rerun(&["glue", "vendor/php-src"]);
    }

    if lexbor {
        // ext/lexbor/config.m4: lexbor's sources (not url, punycode and
        // unicode, which only ext/uri uses) with -I ext/lexbor. rhp-builtins
        // compiles ext/dom's lexbor glue against the same headers and
        // prefix (DEP_RHP_CSRC_LEXBOR_INCLUDE, DEP_RHP_CSRC_LEXBOR_PREFIX).
        let dir = Path::new("vendor/php-src/ext/lexbor");
        let m4 = std::fs::read_to_string(dir.join("config.m4")).unwrap();
        let mut b = cc::Build::new();
        b.include(dir)
            .include("shim")
            .opt_level(2)
            .warnings(false)
            .define("LEXBOR_STATIC", None)
            .flag("-fvisibility=hidden")
            .flag("-include")
            .flag(glue("prefix_lexbor.h"));
        let mut n = 0;
        for f in m4.split_whitespace() {
            let Some(f) = f.strip_prefix("$LEXBOR_DIR/") else {
                continue;
            };
            if f.ends_with(".c")
                && !["url/", "punycode/", "unicode/"]
                    .iter()
                    .any(|m| f.starts_with(m))
            {
                b.file(dir.join("lexbor").join(f));
                n += 1;
            }
        }
        assert!(n > 100, "no lexbor sources found in config.m4");
        b.file("glue/lexbor.c").compile("rhp_csrc_lexbor");
        println!("cargo:lexbor_include={}", manifest.join(dir).display());
        println!("cargo:lexbor_prefix={}", glue("prefix_lexbor.h").display());
        rerun(&["glue", "vendor/php-src"]);
    }

    if calendar {
        // ext/calendar's date conversions (sdncal.h), which need only
        // zend_long from the shim; calendar.c, easter.c and cal_unix.c are
        // ported to Rust (rhp-builtins ext/calendar).
        let dir = Path::new("vendor/php-src/ext/calendar");
        let mut b = php_cc();
        b.include(dir)
            .flag("-include")
            .flag(glue("prefix_calendar.h"));
        for f in ["dow.c", "french.c", "gregor.c", "jewish.c", "julian.c"] {
            b.file(dir.join(f));
        }
        b.compile("rhp_csrc_calendar");
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

    // Linked after the archives above, which call into it: GNU ld (the
    // default linker on aarch64 Linux) takes archives left to right.
    php_cc()
        .file("shim/zend_shim.c")
        .file("shim/php_printf.c")
        .compile("rhp_csrc_zend");
    rerun(&["shim"]);
}

/// A build of `files` (paths under vendor/php-src) against the shared Zend
/// shim.
fn php_src(files: &[&str]) -> cc::Build {
    let vendor = Path::new("vendor/php-src");
    let mut b = php_cc();
    b.include(vendor.join("ext/standard"));
    for f in files {
        b.file(vendor.join(f));
    }
    b
}

/// The shim's include directory and the flags php's configure adds
/// (-ffp-contract=off, -fvisibility=hidden), plus -fwrapv (php-src's
/// timelib flag; elsewhere it only defines signed overflow, which php's
/// release build leaves undefined).
fn php_cc() -> cc::Build {
    let mut b = cc::Build::new();
    b.include("shim")
        .opt_level(2)
        .warnings(false)
        .flag("-fwrapv")
        .flag("-ffp-contract=off")
        .flag("-fvisibility=hidden");
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
