#[path = "build/native.rs"]
mod native;

fn main() {
    println!("cargo:rerun-if-changed=build/native.rs");
    // libz-sys builds its bundled zlib when it finds no system one, and then
    // tells its dependents where (`cargo:root`). rhp links the zlib php
    // links (docs/design/libraries.md): refuse the fallback.
    if let Some(root) = std::env::var_os("DEP_Z_ROOT") {
        panic!(
            "libz-sys built its bundled zlib (in {}): install the system zlib \
             (macOS: the SDK's /usr/lib/libz; Debian/Ubuntu: apt install zlib1g-dev)",
            std::path::Path::new(&root).display()
        );
    }
    let linked = native::link_all();
    native::check_openssl();
    let include = |name: &str| {
        linked
            .iter()
            .find(|l| l.name == name)
            .unwrap_or_else(|| panic!("native.rs links {name}"))
            .include
            .clone()
    };
    // Macros and struct access the Rust side cannot do (see the file).
    println!("cargo:rerun-if-changed=src/ext/libxml/shim.c");
    cc::Build::new()
        .file("src/ext/libxml/shim.c")
        .include(include("LIBXML2"))
        .warnings(true)
        .extra_warnings(true)
        .flag_if_supported("-Wno-deprecated-declarations")
        .flag_if_supported("-Wno-unused-parameter")
        .compile("rhp_libxml_shim");

    let icu = include("ICU_UC");
    icu_checks(&icu);
    println!("cargo:rerun-if-changed=src/ext/intl/shim.c");
    println!("cargo:rerun-if-changed=src/ext/intl/consts.h");
    println!("cargo:rerun-if-changed=src/ext/intl/class_consts.h");
    cc::Build::new()
        .file("src/ext/intl/shim.c")
        .include(&icu)
        .warnings(true)
        .extra_warnings(true)
        .compile("rhp_intl_shim");

    // ext/curl's constants against the linked curl.h, and its stdio shim.
    let curl = include("CURL");
    for f in ["src/ext/curl/consts.c", "src/ext/curl/shim.c"] {
        println!("cargo:rerun-if-changed={f}");
    }
    cc::Build::new()
        .file("src/ext/curl/consts.c")
        .file("src/ext/curl/shim.c")
        .include(&curl)
        .warnings(true)
        .extra_warnings(true)
        .flag_if_supported("-Wno-deprecated-declarations")
        .flag_if_supported("-Wno-unused-parameter")
        .compile("rhp_curl_shim");

    // ext/zip's constants against the linked zip.h, and its layout checks.
    for f in ["src/ext/zip/shim.c", "src/ext/zip/consts.h"] {
        println!("cargo:rerun-if-changed={f}");
    }
    cc::Build::new()
        .file("src/ext/zip/shim.c")
        .include(include("LIBZIP"))
        .warnings(true)
        .extra_warnings(true)
        .flag_if_supported("-Wno-deprecated-declarations")
        .compile("rhp_zip_shim");

    // ext/gmp's layout checks against the linked gmp.h.
    println!("cargo:rerun-if-changed=src/ext/gmp/shim.c");
    cc::Build::new()
        .file("src/ext/gmp/shim.c")
        .include(include("GMP"))
        .warnings(true)
        .extra_warnings(true)
        .compile("rhp_gmp_shim");

    // ext/gd's struct access, constants and error callback against gd.h.
    println!("cargo:rerun-if-changed=src/ext/gd/shim.c");
    cc::Build::new()
        .file("src/ext/gd/shim.c")
        .include(include("LIBGD"))
        .warnings(true)
        .extra_warnings(true)
        .compile("rhp_gd_shim");

    // ICU is C++; its .pc files leave the C++ runtime to the linker driver.
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") | Ok("ios") => println!("cargo:rustc-link-lib=dylib=c++"),
        _ => println!("cargo:rustc-link-lib=dylib=stdc++"),
    }
}

/// sys.rs names ICU's symbols with the version suffix ICU's default build
/// adds (`ubrk_open_79`); a build without renaming would not link.
fn icu_checks(include: &std::path::Path) {
    let uconfig = include.join("unicode/uconfig.h");
    let text =
        std::fs::read_to_string(&uconfig).unwrap_or_else(|e| panic!("{}: {e}", uconfig.display()));
    let renaming_off = text.lines().any(|l| {
        let l: String = l.split_whitespace().collect::<Vec<_>>().join(" ");
        l == "#define U_DISABLE_RENAMING 1"
    });
    if renaming_off {
        panic!(
            "{} sets U_DISABLE_RENAMING: rhp needs an ICU built with versioned symbol names",
            uconfig.display()
        );
    }
}
