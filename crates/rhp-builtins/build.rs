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

    // ext/dom's HTML5 glue: libxml2 and rhp-csrc's lexbor (headers and
    // symbol prefix from its build script).
    println!("cargo:rerun-if-changed=src/ext/dom/html5c/glue.c");
    let lexbor = std::env::var("DEP_RHP_CSRC_LEXBOR_INCLUDE").expect("rhp-csrc's lexbor feature");
    let prefix = std::env::var("DEP_RHP_CSRC_LEXBOR_PREFIX").expect("rhp-csrc's lexbor feature");
    cc::Build::new()
        .file("src/ext/dom/html5c/glue.c")
        .include(include("LIBXML2"))
        .include(lexbor)
        .define("LEXBOR_STATIC", None)
        .flag("-include")
        .flag(&prefix)
        .warnings(false)
        .opt_level(2)
        .compile("rhp_dom_html5");

    let icu = include("ICU_UC");
    icu_checks(&icu);
    // C++: some of php's constants are ICU class members, and the
    // MessageFormat, Calendar and TimeZone parts have no C API.
    for f in [
        "shim.cpp",
        "icu.cpp",
        "breakiter.cpp",
        "consts.h",
        "class_consts.h",
    ] {
        println!("cargo:rerun-if-changed=src/ext/intl/{f}");
    }
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .cpp_link_stdlib(None)
        .file("src/ext/intl/shim.cpp")
        .file("src/ext/intl/icu.cpp")
        .file("src/ext/intl/breakiter.cpp")
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

    // ext/sodium's constants and optional parts against the linked sodium.h.
    println!("cargo:rerun-if-changed=src/ext/sodium/shim.c");
    cc::Build::new()
        .file("src/ext/sodium/shim.c")
        .include(include("LIBSODIUM"))
        .warnings(true)
        .extra_warnings(true)
        .compile("rhp_sodium_shim");

    // mbstring's mb_ereg* against the linked oniguruma.h.
    println!("cargo:rerun-if-changed=src/ext/mbstring/onig_shim.c");
    cc::Build::new()
        .file("src/ext/mbstring/onig_shim.c")
        .include(include("ONIGURUMA"))
        .warnings(true)
        .extra_warnings(true)
        .compile("rhp_onig_shim");

    // ext/gettext's calls through libintl.h (Homebrew's renames them).
    println!("cargo:rerun-if-changed=src/ext/gettext/shim.c");
    let mut intl = cc::Build::new();
    intl.file("src/ext/gettext/shim.c")
        .warnings(true)
        .extra_warnings(true);
    let intl_include = include("LIBINTL");
    if !intl_include.as_os_str().is_empty() {
        intl.include(intl_include);
    }
    intl.compile("rhp_gettext_shim");

    // ext/sockets' constants (consts.h) against the system's headers.
    for f in ["src/ext/sockets/shim.c", "src/ext/sockets/consts.h"] {
        println!("cargo:rerun-if-changed={f}");
    }
    cc::Build::new()
        .file("src/ext/sockets/shim.c")
        .warnings(true)
        .compile("rhp_sockets_shim");

    // ext/gd's struct access, constants and error callback against gd.h,
    // and the formats php's configure finds in the library.
    let gd = linked.iter().find(|l| l.name == "LIBGD").expect("LIBGD");
    let formats = gd_formats(&gd.include, &gd.libdir);
    println!("cargo:rerun-if-changed=src/ext/gd/shim.c");
    cc::Build::new()
        .file("src/ext/gd/shim.c")
        .include(&gd.include)
        .define("RHP_GD_FORMATS", formats.to_string().as_str())
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

/// php's configure (PHP_GD_CHECK_FORMAT in ext/gd/config.m4): build and
/// run `gdImageCreateFrom<Fmt>(NULL)` against the library with an error
/// method that exits 1. libgd built without a format still defines its
/// functions, as stubs that raise an error. Bits: 1 PNG, 2 AVIF, 4 WebP,
/// 8 JPEG, 16 XPM (HAVE_GD_*). A cross build cannot run them and takes
/// `RHP_GD_FORMATS` instead.
fn gd_formats(include: &std::path::Path, libdir: &std::path::Path) -> u32 {
    println!("cargo:rerun-if-env-changed=RHP_GD_FORMATS");
    if let Ok(v) = std::env::var("RHP_GD_FORMATS") {
        return v
            .parse()
            .unwrap_or_else(|_| panic!("RHP_GD_FORMATS={v:?}: expected a number"));
    }
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    if var("TARGET") != var("HOST") {
        panic!("cross build: set RHP_GD_FORMATS to the libgd formats (see build.rs)");
    }
    let out = std::path::PathBuf::from(var("OUT_DIR")).join("gd-formats");
    std::fs::create_dir_all(&out).expect("OUT_DIR");
    let compiler = cc::Build::new().get_compiler();
    let mut bits = 0;
    for (bit, fmt) in [
        (1, "Png"),
        (2, "Avif"),
        (4, "Webp"),
        (8, "Jpeg"),
        (16, "Xpm"),
    ] {
        let arg = if fmt == "Xpm" {
            "char* f = \"test.xpm\""
        } else {
            "FILE* f = NULL"
        };
        let src = out.join(format!("{fmt}.c"));
        let exe = out.join(fmt);
        std::fs::write(
            &src,
            format!(
                "#include <stdio.h>\n#include <unistd.h>\n#include <gd.h>\n\
                 void exit1(int priority, const char *format, va_list args) {{\n\
                 (void)priority; (void)format; (void)args; _exit(1);\n}}\n\
                 int main(void)\n{{\n{arg};\ngdSetErrorMethod(exit1);\n\
                 gdImagePtr p = gdImageCreateFrom{fmt}(f);\n(void)p;\nreturn 0;\n}}\n"
            ),
        )
        .expect("write the gd probe");
        let built = compiler
            .to_command()
            .arg("-I")
            .arg(include)
            .arg(&src)
            .arg("-o")
            .arg(&exe)
            .arg("-L")
            .arg(libdir)
            .arg(format!("-Wl,-rpath,{}", libdir.display()))
            .arg("-lgd")
            .status()
            .expect("run the C compiler");
        if !built.success() {
            panic!("the libgd format probe {} does not build", src.display());
        }
        let ran = std::process::Command::new(&exe)
            .current_dir(&out)
            .status()
            .expect("run the gd probe");
        if ran.success() {
            bits |= bit;
        }
    }
    bits
}
