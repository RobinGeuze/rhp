//! Finds the C libraries rhp links and tells cargo how to link them.
//!
//! One [`Lib`] row per library. For each row:
//! - locate it with pkg-config, or on macOS without pkg-config, through
//!   Homebrew's opt prefix;
//! - read its version from its header and export the listed macros to the
//!   crate as `RHP_<MACRO>` (`env!("RHP_PCRE2_MAJOR")`);
//! - link it statically or dynamically as the row says for the target OS,
//!   unless `RHP_<NAME>_LINK=static|dynamic` overrides that;
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
    /// The library to link, as in `-l<link>`.
    pub link: &'static str,
    /// Header, relative to the include directory, that defines `macros`.
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

/// Every library rhp-builtins links.
pub const LIBS: &[Lib] = &[Lib {
    name: "PCRE2",
    pkg: "libpcre2-8",
    brew: Some("pcre2"),
    link: "pcre2-8",
    header: "pcre2.h",
    macros: &["PCRE2_MAJOR", "PCRE2_MINOR"],
    version: &["PCRE2_MAJOR", "PCRE2_MINOR"],
    macos: Mode::Static,
    linux: Mode::Static,
    install: "macOS: brew install pcre2; Debian/Ubuntu: apt install libpcre2-dev",
}];

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
pub fn link_all() {
    for var in [
        "PKG_CONFIG",
        "PKG_CONFIG_PATH",
        "PKG_CONFIG_LIBDIR",
        "PKG_CONFIG_SYSROOT_DIR",
        "HOMEBREW_PREFIX",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    for lib in LIBS {
        if let Err(e) = link(lib) {
            eprintln!("\nerror: {e}\n");
            panic!("{}: {e}", lib.name);
        }
    }
}

fn target_os() -> String {
    env::var("CARGO_CFG_TARGET_OS").unwrap_or_default()
}

fn link(lib: &Lib) -> Result<(), String> {
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
    let header = found.include.join(lib.header);
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

    println!("cargo:rustc-link-search=native={}", found.libdir.display());
    match mode {
        Mode::Static => {
            let archive = found.libdir.join(format!("lib{}.a", lib.link));
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
            println!("cargo:rustc-link-lib=static={}", lib.link);
            for p in &found.private {
                println!("cargo:rustc-link-lib=dylib={p}");
            }
        }
        Mode::Dynamic(_) => println!("cargo:rustc-link-lib=dylib={}", lib.link),
    }
    Ok(())
}

fn find(lib: &Lib, os: &str) -> Result<Found, String> {
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
        if let Some(f) = pkg_config(&bin, lib.pkg, &extra) {
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
        if prefix.join("include").join(lib.header).is_file() {
            let pc = parse_pc(&prefix.join("lib/pkgconfig").join(format!("{}.pc", lib.pkg)));
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

fn pkg_config(bin: &str, module: &str, extra_paths: &[PathBuf]) -> Option<Found> {
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
    let own = module.strip_prefix("lib").unwrap_or(module);
    Some(Found {
        include: PathBuf::from(include),
        libdir: PathBuf::from(libdir),
        pc_version: Some(version),
        private: link_names(&libs, own),
        via: format!("{bin} {module}"),
    })
}

/// The `-l` names in `flags`, minus the library itself.
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

/// `Version` and the `-l` names of `Libs.private` from a .pc file.
fn parse_pc(path: &Path) -> Option<(String, Vec<String>)> {
    let text = fs::read_to_string(path).ok()?;
    let own = path.file_stem()?.to_str()?;
    let own = own.strip_prefix("lib").unwrap_or(own);
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
