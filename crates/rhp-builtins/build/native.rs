//! Finds the C libraries rhp links and tells cargo how to link them.
//!
//! One [`Lib`] row per library. For each row:
//! - locate it with pkg-config, or on macOS without pkg-config, through
//!   Homebrew's opt prefix; an OS library on macOS comes from the SDK
//!   (`xcrun --show-sdk-path`);
//! - read its version from its header and export the listed macros to the
//!   crate as `RHP_<MACRO>` (`env!("RHP_PCRE2_MAJOR")`);
//! - link it statically or dynamically as the row says for the target OS,
//!   unless `RHP_<NAME>_LINK=static|dynamic` overrides that, and tell the
//!   crate which as `env!("RHP_<NAME>_LINK_MODE")`. A static
//!   archive is linked from a directory of its own under `OUT_DIR`, so the
//!   linker cannot pick the shared library next to it (ld64 has no
//!   `-Bstatic`, and Homebrew ships both);
//! - fail the build, saying how to fix it, when the library, its header or
//!   its static archive is missing. There is no bundled fallback.
//!
//! The policy is in docs/design/libraries.md.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// How a library is linked on one OS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The static archive `lib<link>.a`.
    Static,
    /// The shared library. Only for the exceptions in libraries.md; the
    /// string says why.
    Dynamic(&'static str),
}

pub struct Lib {
    /// Shown in messages; also names the override `RHP_<NAME>_LINK`.
    pub name: &'static str,
    /// pkg-config module.
    pub pkg: &'static str,
    /// Homebrew formula: its opt prefix is searched on macOS, and its
    /// pkgconfig directory is put on `PKG_CONFIG_PATH` (keg-only formulae).
    pub brew: Option<&'static str>,
    /// macOS: take headers and library from the SDK (an OS library php also
    /// takes from there) instead of pkg-config or Homebrew.
    pub macos_sdk: bool,
    /// The library to link, as in `-l<link>`.
    pub link: &'static str,
    /// The subdirectory of the include directory that C code puts on its
    /// `-I` path ("" for the include directory itself).
    pub incdir: &'static str,
    /// Header, relative to `incdir`, that defines `macros`.
    pub header: &'static str,
    /// Header macros exported to the crate as `RHP_<MACRO>`.
    pub macros: &'static [&'static str],
    /// The macros whose values, joined with '.', are the version. It must
    /// equal pkg-config's version for the module when there is a .pc file.
    pub version: &'static [&'static str],
    pub macos: Mode,
    pub linux: Mode,
    /// How to install it, for error messages.
    pub install: &'static str,
}

// A static link's private dependencies (`Libs.private`) are linked
// dynamically from the linker's default paths. One that is itself a Homebrew
// or -dev library needs its own row.

/// Every library rhp-builtins links.
pub const LIBS: &[Lib] = &[
    Lib {
        name: "PCRE2",
        pkg: "libpcre2-8",
        brew: Some("pcre2"),
        macos_sdk: false,
        link: "pcre2-8",
        incdir: "",
        header: "pcre2.h",
        macros: &["PCRE2_MAJOR", "PCRE2_MINOR"],
        version: &["PCRE2_MAJOR", "PCRE2_MINOR"],
        macos: Mode::Static,
        linux: Mode::Static,
        install: "macOS: brew install pcre2; Debian/Ubuntu: apt install pkg-config libpcre2-dev",
    },
    // php takes libxml2 from the OS on macOS (/usr/lib/libxml2.2.dylib);
    // the version constants come from src/ext/libxml/shim.c.
    Lib {
        name: "LIBXML2",
        pkg: "libxml-2.0",
        brew: None,
        macos_sdk: true,
        link: "xml2",
        incdir: "libxml2",
        header: "libxml/xmlversion.h",
        macros: &[],
        version: &["LIBXML_DOTTED_VERSION"],
        macos: Mode::Dynamic("an OS library with no static archive (/usr/lib)"),
        linux: Mode::Static,
        install: "macOS: the Command Line Tools SDK (xcode-select --install); \
                  Debian/Ubuntu: apt install pkg-config libxml2-dev",
    },
];

/// A library [`link_all`] linked.
pub struct Linked {
    pub name: &'static str,
    /// The directory to put on a C compiler's `-I` path for it.
    pub include: PathBuf,
}

/// Where a library was found.
struct Found {
    include: PathBuf,
    libdir: PathBuf,
    /// The .pc file's version, when there is one.
    pc_version: Option<String>,
    /// Other libraries a static link needs (`Libs.private`).
    private: Vec<String>,
    via: String,
}

/// Link every row of [`LIBS`].
pub fn link_all() -> Vec<Linked> {
    for var in [
        "PKG_CONFIG",
        "PKG_CONFIG_PATH",
        "PKG_CONFIG_LIBDIR",
        "PKG_CONFIG_SYSROOT_DIR",
        "HOMEBREW_PREFIX",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    let mut linked = Vec::new();
    for lib in LIBS {
        match link(lib) {
            Ok(include) => linked.push(Linked {
                name: lib.name,
                include,
            }),
            Err(e) => {
                eprintln!("\nerror: {e}\n");
                panic!("{}: {e}", lib.name);
            }
        }
    }
    linked
}

fn target_os() -> String {
    env::var("CARGO_CFG_TARGET_OS").unwrap_or_default()
}

fn link(lib: &Lib) -> Result<PathBuf, String> {
    let override_var = format!("RHP_{}_LINK", lib.name);
    println!("cargo:rerun-if-env-changed={override_var}");
    let os = target_os();
    let mut mode = if os == "macos" { lib.macos } else { lib.linux };
    match env::var(&override_var).as_deref() {
        Ok("static") => mode = Mode::Static,
        Ok("dynamic") => mode = Mode::Dynamic("RHP_*_LINK=dynamic"),
        Ok(other) => {
            return Err(format!(
                "{override_var}={other:?}: expected \"static\" or \"dynamic\""
            ))
        }
        Err(_) => {}
    }

    let found = find(lib, &os)?;
    let include = found.include.join(lib.incdir);
    let header = include.join(lib.header);
    let text = fs::read_to_string(&header).map_err(|e| {
        format!(
            "{} found via {} but its header {} is unreadable ({e}). Install it ({}).",
            lib.name,
            found.via,
            header.display(),
            lib.install
        )
    })?;
    println!("cargo:rerun-if-changed={}", header.display());

    let get = |m: &str| {
        header_macro(&text, m).ok_or_else(|| format!("{} defines no {m}", header.display()))
    };
    for m in lib.macros {
        println!("cargo:rustc-env=RHP_{m}={}", get(m)?);
    }
    let version = lib
        .version
        .iter()
        .map(|m| get(m))
        .collect::<Result<Vec<_>, _>>()?
        .join(".");
    if let Some(pc) = &found.pc_version {
        if *pc != version {
            return Err(format!(
                "{} {} has version {version}, but its pkg-config file says {pc}: the \
                 headers and the library come from different installs ({})",
                lib.name,
                header.display(),
                found.via
            ));
        }
    }

    let mode_name = if mode == Mode::Static {
        "static"
    } else {
        "dynamic"
    };
    println!("cargo:rustc-env=RHP_{}_LINK_MODE={mode_name}", lib.name);
    match mode {
        Mode::Static => {
            let file = format!("lib{}.a", lib.link);
            let archive = found.libdir.join(&file);
            if !archive.is_file() {
                return Err(format!(
                    "{} {version} ({}) has no static archive {}. rhp links {} statically; \
                     install the package that ships it ({}), or set {override_var}=dynamic \
                     for an experiment.",
                    lib.name,
                    found.via,
                    archive.display(),
                    lib.name,
                    lib.install
                ));
            }
            println!("cargo:rerun-if-changed={}", archive.display());
            let dir = archive_dir(lib, &archive, &file)?;
            println!("cargo:rustc-link-search=native={}", dir.display());
            println!("cargo:rustc-link-lib=static={}", lib.link);
            for p in &found.private {
                println!("cargo:rustc-link-lib=dylib={p}");
            }
        }
        Mode::Dynamic(_) => {
            println!("cargo:rustc-link-search=native={}", found.libdir.display());
            println!("cargo:rustc-link-lib=dylib={}", lib.link);
        }
    }
    Ok(include)
}

/// `$OUT_DIR/native-<name>/` holding only a link to `archive`: the one
/// search path a static link gets.
fn archive_dir(lib: &Lib, archive: &Path, file: &str) -> Result<PathBuf, String> {
    let out = env::var_os("OUT_DIR").ok_or_else(|| "OUT_DIR is not set".to_string())?;
    let dir = PathBuf::from(out).join(format!("native-{}", lib.name.to_ascii_lowercase()));
    let fail = |e: std::io::Error| format!("{}: {e}", dir.display());
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(fail)?;
    }
    fs::create_dir_all(&dir).map_err(fail)?;
    let dest = dir.join(file);
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(archive, &dest);
    #[cfg(not(unix))]
    let linked = fs::copy(archive, &dest).map(|_| ());
    linked.map_err(fail)?;
    Ok(dir)
}

fn find(lib: &Lib, os: &str) -> Result<Found, String> {
    if os == "macos" && lib.macos_sdk {
        return find_sdk(lib);
    }
    let brew_prefixes: Vec<PathBuf> = match (os, lib.brew) {
        ("macos", Some(formula)) => brew_roots()
            .into_iter()
            .map(|r| r.join("opt").join(formula))
            .filter(|p| p.is_dir())
            .collect(),
        _ => Vec::new(),
    };
    if let Some(bin) = pkg_config_bin() {
        let extra: Vec<PathBuf> = brew_prefixes
            .iter()
            .map(|p| p.join("lib/pkgconfig"))
            .filter(|p| p.is_dir())
            .collect();
        if let Some(f) = pkg_config(&bin, lib, &extra) {
            return Ok(f);
        }
        if brew_prefixes.is_empty() {
            return Err(format!(
                "{} not found: `{bin} {}` knows no such module. Install it ({}).",
                lib.name, lib.pkg, lib.install
            ));
        }
    }
    for prefix in &brew_prefixes {
        if prefix
            .join("include")
            .join(lib.incdir)
            .join(lib.header)
            .is_file()
        {
            let pc = parse_pc(
                &prefix.join("lib/pkgconfig").join(format!("{}.pc", lib.pkg)),
                lib.link,
            );
            return Ok(Found {
                include: prefix.join("include"),
                libdir: prefix.join("lib"),
                pc_version: pc.as_ref().map(|p| p.0.clone()),
                private: pc.map(|p| p.1).unwrap_or_default(),
                via: format!("Homebrew {}", prefix.display()),
            });
        }
    }
    let pkg_config = match pkg_config_bin() {
        Some(bin) => format!("`{bin}` knows no {}", lib.pkg),
        // Off macOS pkg-config is the only way the helper looks.
        None if os != "macos" => {
            return Err(format!(
                "{} not found: pkg-config is not installed, and off macOS rhp finds \
                 libraries only through it. Install pkg-config and {} ({}).",
                lib.name, lib.name, lib.install
            ))
        }
        None => "no pkg-config".into(),
    };
    let looked: Vec<String> = match lib.brew {
        Some(f) if os == "macos" => brew_roots()
            .iter()
            .map(|r| r.join("opt").join(f).join("include").display().to_string())
            .collect(),
        _ => Vec::new(),
    };
    let brew = if looked.is_empty() {
        String::new()
    } else {
        format!(", and no {} in {}", lib.header, looked.join(" or "))
    };
    Err(format!(
        "{} not found: {pkg_config}{brew}. Install it ({}).",
        lib.name, lib.install
    ))
}

/// An OS library in the macOS SDK: headers under `$SDK/usr/include`, the
/// `.tbd` stub under `$SDK/usr/lib`. The SDK has no .pc files.
fn find_sdk(lib: &Lib) -> Result<Found, String> {
    println!("cargo:rerun-if-env-changed=SDKROOT");
    let sdk = match env::var("SDKROOT") {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => {
            let out = Command::new("xcrun")
                .arg("--show-sdk-path")
                .output()
                .map_err(|e| {
                    format!(
                        "{} not found: xcrun did not run ({e}). Install it ({}).",
                        lib.name, lib.install
                    )
                })?;
            if !out.status.success() {
                return Err(format!(
                    "{} not found: `xcrun --show-sdk-path` failed. Install it ({}).",
                    lib.name, lib.install
                ));
            }
            PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())
        }
    };
    let include = sdk.join("usr/include");
    if !include.join(lib.incdir).join(lib.header).is_file() {
        return Err(format!(
            "{} not found: no {} in the SDK {}. Install it ({}).",
            lib.name,
            lib.header,
            include.join(lib.incdir).display(),
            lib.install
        ));
    }
    Ok(Found {
        include,
        libdir: sdk.join("usr/lib"),
        pc_version: None,
        private: Vec::new(),
        via: format!("the macOS SDK {}", sdk.display()),
    })
}

fn brew_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(p) = env::var("HOMEBREW_PREFIX") {
        roots.push(PathBuf::from(p));
    }
    for p in ["/opt/homebrew", "/usr/local"] {
        let p = PathBuf::from(p);
        if !roots.contains(&p) {
            roots.push(p);
        }
    }
    roots
}

/// `$PKG_CONFIG`, else `pkg-config`, else `pkgconf`, if it runs.
fn pkg_config_bin() -> Option<String> {
    let mut candidates = Vec::new();
    if let Ok(p) = env::var("PKG_CONFIG") {
        candidates.push(p);
    }
    candidates.push("pkg-config".into());
    candidates.push("pkgconf".into());
    candidates.into_iter().find(|c| {
        Command::new(c)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

fn pkg_config(bin: &str, lib: &Lib, extra_paths: &[PathBuf]) -> Option<Found> {
    let module = lib.pkg;
    let mut path: Vec<PathBuf> = extra_paths.to_vec();
    if let Some(p) = env::var_os("PKG_CONFIG_PATH") {
        path.extend(env::split_paths(&p));
    }
    let run = |args: &[&str]| -> Option<String> {
        let mut cmd = Command::new(bin);
        cmd.args(args).arg(module);
        if !path.is_empty() {
            cmd.env("PKG_CONFIG_PATH", env::join_paths(&path).ok()?);
        }
        let out = cmd.output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let version = run(&["--modversion"])?;
    let include = run(&["--variable=includedir"])?;
    let libdir = run(&["--variable=libdir"])?;
    let libs = run(&["--static", "--libs-only-l"]).unwrap_or_default();
    Some(Found {
        include: PathBuf::from(include),
        libdir: PathBuf::from(libdir),
        pc_version: Some(version),
        private: link_names(&libs, lib.link),
        via: format!("{bin} {module}"),
    })
}

/// The `-l` names in `flags`, minus the library itself (`own`, its link
/// name: a second `-l` for it would turn the static link dynamic).
fn link_names(flags: &str, own: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in flags.split_whitespace() {
        if let Some(l) = f.strip_prefix("-l") {
            if l != own && !out.iter().any(|o| o == l) {
                out.push(l.to_string());
            }
        }
    }
    out
}

/// `Version` and the `-l` names of `Libs.private` from a .pc file, minus
/// `own`.
fn parse_pc(path: &Path, own: &str) -> Option<(String, Vec<String>)> {
    let text = fs::read_to_string(path).ok()?;
    let mut version = None;
    let mut private = Vec::new();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("Version:") {
            version = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("Libs.private:") {
            private = link_names(v, own);
        }
    }
    Some((version?, private))
}

/// The value of `#define NAME value` in a C header, without quotes or a
/// trailing comment.
fn header_macro(text: &str, name: &str) -> Option<String> {
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix('#') else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix("define") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(value) = rest.strip_prefix(name) else {
            continue;
        };
        if !value.starts_with([' ', '\t']) {
            continue;
        }
        let value = value.split("/*").next().unwrap_or("").trim();
        return Some(value.trim_matches('"').to_string());
    }
    None
}
