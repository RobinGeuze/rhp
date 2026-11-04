//! Finds the C libraries rhp links and tells cargo how to link them.
//!
//! One [`Lib`] row per library. For each row:
//! - locate it with pkg-config, or on macOS without pkg-config, through
//!   Homebrew's opt prefix; an OS library on macOS comes from the SDK
//!   (`xcrun --show-sdk-path`);
//! - read its version from its header and export the listed macros to the
//!   crate as `RHP_<MACRO>` (`env!("RHP_PCRE2_MAJOR")`);
//! - link it statically or dynamically as the row says for the target OS,
//!   unless `RHP_<NAME>_LINK=static|dynamic` overrides that (an LGPL
//!   library is never linked statically), and tell the
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
    /// The shared library of an LGPL library, which may not be linked
    /// statically: `RHP_<NAME>_LINK=static` is refused.
    Lgpl,
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

// A static link's private dependencies (its .pc file's `Libs.private` and
// `Requires.private`) are linked by [`link_private`]: on macOS statically
// when a Homebrew formula ships the archive, else from the SDK; elsewhere
// dynamically. A dependency with a row of its own, or one openssl-sys links
// (libssl, libcrypto), is left to that link: one copy of each library.

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
    // ICU: three archives, in link order. Their C++ code needs the C++
    // runtime, which build.rs links (the .pc files leave it out).
    Lib {
        name: "ICU_I18N",
        pkg: "icu-i18n",
        brew: Some("icu4c@79"),
        macos_sdk: false,
        link: "icui18n",
        incdir: "",
        header: "unicode/uvernum.h",
        macros: &[],
        version: &["U_ICU_VERSION"],
        macos: Mode::Static,
        linux: Mode::Static,
        install: ICU_INSTALL,
    },
    Lib {
        name: "ICU_UC",
        pkg: "icu-uc",
        brew: Some("icu4c@79"),
        macos_sdk: false,
        link: "icuuc",
        incdir: "",
        header: "unicode/uvernum.h",
        macros: &["U_ICU_VERSION", "U_ICU_VERSION_MAJOR_NUM"],
        version: &["U_ICU_VERSION"],
        macos: Mode::Static,
        linux: Mode::Static,
        install: ICU_INSTALL,
    },
    // No .pc of its own: icu-uc's names it in Libs.private.
    Lib {
        name: "ICU_DATA",
        pkg: "icu-uc",
        brew: Some("icu4c@79"),
        macos_sdk: false,
        link: "icudata",
        incdir: "",
        header: "unicode/uvernum.h",
        macros: &[],
        version: &["U_ICU_VERSION"],
        macos: Mode::Static,
        linux: Mode::Static,
        install: ICU_INSTALL,
    },
    // php links Homebrew's keg-only curl on macOS, not the SDK's older one.
    Lib {
        name: "CURL",
        pkg: "libcurl",
        brew: Some("curl"),
        macos_sdk: false,
        link: "curl",
        incdir: "",
        header: "curl/curlver.h",
        macros: &["LIBCURL_VERSION"],
        version: &["LIBCURL_VERSION"],
        macos: Mode::Static,
        linux: Mode::Static,
        install: "macOS: brew install curl; Debian/Ubuntu: apt install pkg-config \
                  libcurl4-openssl-dev",
    },
    // LGPL-3.0+/GPL-2.0+: a dynamic system library only, never static.
    Lib {
        name: "GMP",
        pkg: "gmp",
        brew: Some("gmp"),
        macos_sdk: false,
        link: "gmp",
        incdir: "",
        header: "gmp.h",
        macros: &[],
        version: &[
            "__GNU_MP_VERSION",
            "__GNU_MP_VERSION_MINOR",
            "__GNU_MP_VERSION_PATCHLEVEL",
        ],
        macos: Mode::Lgpl,
        linux: Mode::Lgpl,
        install: "macOS: brew install gmp; Debian/Ubuntu: apt install pkg-config libgmp-dev",
    },
];

const ICU_INSTALL: &str =
    "macOS: brew install icu4c@79; Debian/Ubuntu: apt install pkg-config libicu-dev";

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
    /// The .pc file, when there is one.
    pc_file: Option<PathBuf>,
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

/// Debian's multiarch tuple for the target (`x86_64-linux-gnu`).
fn multiarch_tuple() -> String {
    let var = |k: &str| env::var(k).unwrap_or_default();
    let arch = match var("CARGO_CFG_TARGET_ARCH").as_str() {
        "x86" => "i386".to_string(),
        "powerpc64" if var("CARGO_CFG_TARGET_ENDIAN") == "little" => "powerpc64le".into(),
        a => a.to_string(),
    };
    format!(
        "{arch}-{}-{}{}",
        var("CARGO_CFG_TARGET_OS"),
        var("CARGO_CFG_TARGET_ENV"),
        var("CARGO_CFG_TARGET_ABI")
    )
}

fn link(lib: &Lib) -> Result<PathBuf, String> {
    let override_var = format!("RHP_{}_LINK", lib.name);
    println!("cargo:rerun-if-env-changed={override_var}");
    let os = target_os();
    let mut mode = if os == "macos" { lib.macos } else { lib.linux };
    match env::var(&override_var).as_deref() {
        Ok("static") if mode == Mode::Lgpl => {
            return Err(format!(
                "{override_var}=static: {} is LGPL and is only linked dynamically \
                 (docs/design/libraries.md)",
                lib.name
            ))
        }
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
    let mut include = found.include.join(lib.incdir);
    if !include.join(lib.header).is_file() {
        // Debian/Ubuntu put headers that differ per architecture (gmp.h)
        // in the multiarch directory, which the .pc file does not name.
        let multiarch = found.include.join(multiarch_tuple()).join(lib.incdir);
        if multiarch.join(lib.header).is_file() {
            include = multiarch;
        }
    }
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
            let dir = archive_dir(&lib.name.to_ascii_lowercase(), &archive, &file)?;
            println!("cargo:rustc-link-search=native={}", dir.display());
            println!("cargo:rustc-link-lib=static={}", lib.link);
            if let Some(pc) = &found.pc_file {
                link_private(lib, pc, &os)?;
            }
        }
        Mode::Dynamic(_) | Mode::Lgpl => {
            println!("cargo:rustc-link-search=native={}", found.libdir.display());
            println!("cargo:rustc-link-lib=dylib={}", lib.link);
        }
    }
    Ok(include)
}

/// `$OUT_DIR/native-<key>/` holding only a link to `archive`: the one
/// search path a static link gets.
fn archive_dir(key: &str, archive: &Path, file: &str) -> Result<PathBuf, String> {
    let out = env::var_os("OUT_DIR").ok_or_else(|| "OUT_DIR is not set".to_string())?;
    let dir = PathBuf::from(out).join(format!("native-{key}"));
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

/// Link the libraries a static archive needs, read from its .pc file.
///
/// macOS: each `-l` of the .pc (and of the .pc files it requires) that a
/// Homebrew formula ships as `lib<name>.a` is linked statically, with that
/// archive's own dependencies; Homebrew's .pc files name Cellar paths, which
/// are mapped to the formula's opt prefix (the build php's dylibs load).
/// Every other `-l` must be an OS library in the SDK, and `-framework`s are
/// linked as frameworks.
///
/// Elsewhere the archive's direct dependencies are linked dynamically: its
/// `Libs.private`, and the `Libs` of the modules in its `Requires.private`.
fn link_private(lib: &Lib, pc_file: &Path, os: &str) -> Result<(), String> {
    if os == "macos" {
        link_private_macos(lib, pc_file)
    } else {
        link_private_dynamic(lib, pc_file)
    }
}

fn link_private_macos(lib: &Lib, pc_file: &Path) -> Result<(), String> {
    // Only the Homebrew prefix the library itself comes from.
    let roots: Vec<PathBuf> = brew_roots()
        .into_iter()
        .filter(|r| pc_file.starts_with(r))
        .collect();
    let mut queue = std::collections::VecDeque::from([pc_file.to_path_buf()]);
    let mut seen_pc = Vec::new();
    let mut seen = vec![lib.link.to_string()];
    let mut statics: Vec<(String, PathBuf)> = Vec::new();
    let mut dylibs: Vec<String> = Vec::new();
    let mut frameworks: Vec<String> = Vec::new();
    while let Some(pcf) = queue.pop_front() {
        let key = fs::canonicalize(&pcf).unwrap_or_else(|_| pcf.clone());
        if seen_pc.contains(&key) {
            continue;
        }
        seen_pc.push(key);
        println!("cargo:rerun-if-changed={}", pcf.display());
        let pc = read_pc(&pcf).ok_or_else(|| format!("{}: unreadable", pcf.display()))?;
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut names = Vec::new();
        for w in pc.libs.iter().chain(&pc.libs_private) {
            if let Some(d) = w.strip_prefix("-L") {
                dirs.push(brew_opt_path(Path::new(d), &roots));
            } else if let Some(n) = w.strip_prefix("-l") {
                names.push(n.to_string());
            } else if let Some(f) = w.strip_prefix("-framework ") {
                if !frameworks.iter().any(|x| x == f) {
                    frameworks.push(f.to_string());
                }
            }
        }
        if let Some(d) = &pc.libdir {
            dirs.insert(0, brew_opt_path(d, &roots));
        }
        dirs.extend(roots.iter().map(|r| r.join("lib")));
        for n in names {
            if seen.contains(&n) {
                continue;
            }
            seen.push(n.clone());
            if linked_elsewhere(&n) {
                continue;
            }
            let file = format!("lib{n}.a");
            match dirs.iter().map(|d| d.join(&file)).find(|a| a.is_file()) {
                Some(a) => {
                    if let Some(p) = pc_naming(&a, &n) {
                        queue.push_back(p);
                    }
                    statics.push((n, a));
                }
                None => dylibs.push(n),
            }
        }
        let mut pcdirs: Vec<PathBuf> = pcf.parent().map(Path::to_path_buf).into_iter().collect();
        pcdirs.extend(dirs.iter().map(|d| d.join("pkgconfig")));
        for m in pc.requires.iter().chain(&pc.requires_private) {
            let file = format!("{m}.pc");
            if let Some(p) = pcdirs.iter().map(|d| d.join(&file)).find(|p| p.is_file()) {
                queue.push_back(p);
            }
        }
    }
    for (n, a) in &statics {
        println!("cargo:rerun-if-changed={}", a.display());
        let key = format!("{}-{n}", lib.name.to_ascii_lowercase());
        let dir = archive_dir(&key, a, &format!("lib{n}.a"))?;
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rustc-link-lib=static={n}");
    }
    let sdk_lib = sdk_root(lib)?.join("usr/lib");
    for n in &dylibs {
        if !sdk_lib.join(format!("lib{n}.tbd")).is_file() {
            return Err(format!(
                "{} needs lib{n}, which is neither a static archive in Homebrew nor an OS \
                 library in the SDK ({}). Install the formula that ships it.",
                lib.name,
                sdk_lib.display()
            ));
        }
        println!("cargo:rustc-link-lib=dylib={n}");
    }
    for f in &frameworks {
        println!("cargo:rustc-link-lib=framework={f}");
    }
    Ok(())
}

fn link_private_dynamic(lib: &Lib, pc_file: &Path) -> Result<(), String> {
    println!("cargo:rerun-if-changed={}", pc_file.display());
    let pc = read_pc(pc_file).ok_or_else(|| format!("{}: unreadable", pc_file.display()))?;
    let mut words = pc.libs_private.clone();
    if !pc.requires_private.is_empty() {
        if let Some(bin) = pkg_config_bin() {
            // A module that is not installed is skipped: its libraries are
            // usually in Libs.private too, and the linker names a missing one.
            for m in &pc.requires_private {
                if let Ok(out) = Command::new(&bin).args(["--libs", m.as_str()]).output() {
                    if out.status.success() {
                        let s = String::from_utf8_lossy(&out.stdout).into_owned();
                        words.extend(s.split_whitespace().map(str::to_string));
                    }
                }
            }
        }
    }
    let mut done: Vec<&str> = vec![lib.link];
    for w in &words {
        if let Some(d) = w.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={d}");
        }
    }
    for w in &words {
        let Some(n) = w.strip_prefix("-l") else {
            continue;
        };
        if done.contains(&n) || linked_elsewhere(n) {
            continue;
        }
        done.push(n);
        // `-l:FILE` goes through `+verbatim` (rustc would read `:FILE` as
        // `NAME:RENAME`); as a dylib it comes after all the Rust archives,
        // and GNU ld still links a `-l:lib.a` statically.
        match n.strip_prefix(':') {
            Some(file) => println!("cargo:rustc-link-lib=dylib:+verbatim={file}"),
            None => println!("cargo:rustc-link-lib=dylib={n}"),
        }
    }
    Ok(())
}

/// A library linked by a row of its own or by openssl-sys.
fn linked_elsewhere(name: &str) -> bool {
    LIBS.iter().any(|l| l.link == name) || matches!(name, "ssl" | "crypto")
}

/// A path under `<brew>/Cellar/<formula>/<version>/` as the same path under
/// `<brew>/opt/<formula>/`: the installed version, which a .pc file written
/// before an upgrade of the formula may not name.
fn brew_opt_path(p: &Path, roots: &[PathBuf]) -> PathBuf {
    for r in roots {
        if let Ok(rest) = p.strip_prefix(r.join("Cellar")) {
            let mut parts = rest.components();
            if let (Some(formula), Some(_version)) = (parts.next(), parts.next()) {
                return r.join("opt").join(formula).join(parts.as_path());
            }
        }
    }
    p.to_path_buf()
}

/// The .pc file next to static archive `a` (in its `pkgconfig`
/// subdirectory) whose `Libs` links `-l<name>`.
fn pc_naming(a: &Path, name: &str) -> Option<PathBuf> {
    let dir = a.parent()?.join("pkgconfig");
    let want = format!("-l{name}");
    let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "pc"))
        .collect();
    entries.sort();
    entries
        .into_iter()
        .find(|p| read_pc(p).is_some_and(|pc| pc.libs.contains(&want)))
}

/// What the helper reads from a .pc file, with `${var}`s expanded.
#[derive(Default)]
struct Pc {
    version: Option<String>,
    /// The words of `Libs` and `Libs.private`; `-framework X` is one word.
    libs: Vec<String>,
    libs_private: Vec<String>,
    /// The modules `Requires` and `Requires.private` name.
    requires: Vec<String>,
    requires_private: Vec<String>,
    libdir: Option<PathBuf>,
}

fn read_pc(path: &Path) -> Option<Pc> {
    let text = fs::read_to_string(path).ok()?;
    let mut vars: Vec<(String, String)> = Vec::new();
    if let Some(dir) = path.parent() {
        vars.push(("pcfiledir".into(), dir.display().to_string()));
    }
    let expand = |s: &str, vars: &[(String, String)]| -> String {
        let mut s = s.to_string();
        for _ in 0..16 {
            let Some(start) = s.find("${") else { break };
            let Some(len) = s[start..].find('}') else {
                break;
            };
            let name = &s[start + 2..start + len];
            let value = vars
                .iter()
                .rev()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            s.replace_range(start..start + len + 1, &value);
        }
        s
    };
    let mut pc = Pc::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let colon = line.find(':');
        let eq = line.find('=');
        match (colon, eq) {
            (Some(c), e) if e.is_none_or(|e| c < e) => {
                let value = expand(line[c + 1..].trim(), &vars);
                match &line[..c] {
                    "Version" => pc.version = Some(value),
                    "Libs" => pc.libs = lib_words(&value),
                    "Libs.private" => pc.libs_private = lib_words(&value),
                    "Requires" => pc.requires = modules(&value),
                    "Requires.private" => pc.requires_private = modules(&value),
                    _ => {}
                }
            }
            (_, Some(e)) => {
                let name = line[..e].trim().to_string();
                let value = expand(line[e + 1..].trim(), &vars);
                if name == "libdir" {
                    pc.libdir = Some(PathBuf::from(&value));
                }
                vars.push((name, value));
            }
            _ => {}
        }
    }
    Some(pc)
}

fn lib_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut words = s.split_whitespace();
    while let Some(w) = words.next() {
        if w == "-framework" {
            if let Some(f) = words.next() {
                out.push(format!("-framework {f}"));
            }
        } else {
            out.push(w.to_string());
        }
    }
    out
}

/// The module names in a `Requires` line, without version constraints.
fn modules(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for w in s.split(|c: char| c == ',' || c.is_whitespace()) {
        if w.is_empty() {
            continue;
        }
        if skip {
            skip = false;
            continue;
        }
        if matches!(w, "=" | "<" | ">" | "<=" | ">=" | "!=") {
            skip = true;
            continue;
        }
        out.push(w.to_string());
    }
    out
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
            let pc_file = prefix.join("lib/pkgconfig").join(format!("{}.pc", lib.pkg));
            let pc_version = read_pc(&pc_file).and_then(|pc| pc.version);
            return Ok(Found {
                include: prefix.join("include"),
                libdir: prefix.join("lib"),
                pc_file: pc_version.is_some().then_some(pc_file),
                pc_version,
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
    let sdk = sdk_root(lib)?;
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
        pc_file: None,
        via: format!("the macOS SDK {}", sdk.display()),
    })
}

/// `$SDKROOT`, else `xcrun --show-sdk-path`.
fn sdk_root(lib: &Lib) -> Result<PathBuf, String> {
    println!("cargo:rerun-if-env-changed=SDKROOT");
    Ok(match env::var("SDKROOT") {
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
    let pc_file = run(&["--variable=pcfiledir"])
        .map(|d| PathBuf::from(d).join(format!("{module}.pc")))
        .filter(|f| f.is_file());
    Some(Found {
        include: PathBuf::from(include),
        libdir: PathBuf::from(libdir),
        pc_version: Some(version),
        pc_file,
        via: format!("{bin} {module}"),
    })
}

/// OpenSSL is linked by openssl-sys, configured through the repo's
/// `.cargo/config.toml` (`OPENSSL_STATIC`, and on Linux the library and
/// header directories). Check that what it linked is the OpenSSL php links,
/// statically, and export the header's version as `RHP_OPENSSL_VERSION_TEXT`
/// and `RHP_OPENSSL_VERSION_NUMBER` (php reports its compile-time header).
pub fn check_openssl() {
    let lib = Lib {
        name: "OPENSSL",
        pkg: "libcrypto",
        brew: Some("openssl@4"),
        macos_sdk: false,
        link: "crypto",
        incdir: "",
        header: "openssl/opensslv.h",
        macros: &["OPENSSL_VERSION_TEXT"],
        version: &[
            "OPENSSL_VERSION_MAJOR",
            "OPENSSL_VERSION_MINOR",
            "OPENSSL_VERSION_PATCH",
        ],
        macos: Mode::Static,
        linux: Mode::Static,
        install: "macOS: brew install openssl@4; Debian/Ubuntu: apt install pkg-config libssl-dev",
    };
    if let Err(e) = openssl(&lib) {
        eprintln!("\nerror: {e}\n");
        panic!("OPENSSL: {e}");
    }
}

fn openssl(lib: &Lib) -> Result<(), String> {
    let os = target_os();
    let target = env::var("TARGET").unwrap_or_default();
    let prefix = target.to_uppercase().replace('-', "_");
    // openssl-sys reads `<TARGET>_<NAME>` before `<NAME>`.
    let var = |name: &str| {
        let prefixed = format!("{prefix}_{name}");
        println!("cargo:rerun-if-env-changed={prefixed}");
        println!("cargo:rerun-if-env-changed={name}");
        env::var(&prefixed).or_else(|_| env::var(name)).ok()
    };
    let include = PathBuf::from(
        env::var("DEP_OPENSSL_INCLUDE")
            .map_err(|_| "openssl-sys exported no include directory".to_string())?,
    );
    let number = env::var("DEP_OPENSSL_VERSION_NUMBER")
        .map_err(|_| "openssl-sys found no OpenSSL 1.1+ headers".to_string())?;
    let header = include.join(lib.header);
    let text = fs::read_to_string(&header)
        .map_err(|e| format!("{} is unreadable ({e})", header.display()))?;
    println!("cargo:rerun-if-changed={}", header.display());
    let get = |m: &str| {
        header_macro(&text, m).ok_or_else(|| format!("{} defines no {m}", header.display()))
    };
    let version = lib
        .version
        .iter()
        .map(|m| get(m))
        .collect::<Result<Vec<_>, _>>()?
        .join(".");
    println!(
        "cargo:rustc-env=RHP_OPENSSL_VERSION_TEXT={}",
        get("OPENSSL_VERSION_TEXT")?
    );
    println!("cargo:rustc-env=RHP_OPENSSL_VERSION_NUMBER={number}");
    // The OPENSSL_NO_* defines xp_ssl.c checks, from the configuration
    // header opensslconf.h includes (Debian/Ubuntu keep it in the multiarch
    // directory).
    let multiarch = {
        let mut parts = target.split('-');
        let arch = parts.next().unwrap_or("");
        format!("{arch}-{}-{}", os, target.rsplit('-').next().unwrap_or(""))
    };
    let conf = [
        include.join("openssl/configuration.h"),
        include.join(&multiarch).join("openssl/configuration.h"),
        include.join("openssl/opensslconf.h"),
    ]
    .into_iter()
    .find(|p| p.is_file())
    .ok_or_else(|| {
        format!(
            "no openssl/configuration.h under {} (or {multiarch}/)",
            include.display()
        )
    })?;
    let conf_text = fs::read_to_string(&conf).unwrap_or_default();
    println!("cargo:rerun-if-changed={}", conf.display());
    let no: Vec<&str> = [
        "OPENSSL_NO_SSL3",
        "OPENSSL_NO_TLS1_METHOD",
        "OPENSSL_NO_TLS1_1_METHOD",
        "OPENSSL_NO_TLS1_2_METHOD",
        "OPENSSL_NO_TLS1_3",
        "OPENSSL_NO_TLSEXT",
    ]
    .into_iter()
    .filter(|m| {
        conf_text.lines().any(|l| {
            let l = l.trim_start_matches(|c: char| c == '#' || c.is_whitespace());
            l.strip_prefix("define")
                .is_some_and(|r| r.split_whitespace().next() == Some(m))
        })
    })
    .collect();
    println!("cargo:rustc-env=RHP_OPENSSL_NO={}", no.join(","));

    // The OpenSSL php links: Homebrew's openssl@4 on macOS, pkg-config's
    // libcrypto elsewhere. openssl-sys must have used the same headers.
    let found = find(lib, &os)?;
    let same = |a: &Path, b: &Path| match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same(&found.include, &include) {
        return Err(format!(
            "openssl-sys compiled against the headers in {}, but the OpenSSL php links \
             is {} (headers in {}). Point OPENSSL_DIR (or OPENSSL_LIB_DIR and \
             OPENSSL_INCLUDE_DIR) at it.",
            include.display(),
            found.via,
            found.include.display()
        ));
    }
    if let Some(pc) = &found.pc_version {
        if *pc != version {
            return Err(format!(
                "OpenSSL {} has version {version}, but its pkg-config file says {pc}",
                header.display()
            ));
        }
    }

    let statik = var("OPENSSL_STATIC").is_some_and(|v| v != "0");
    println!(
        "cargo:rustc-env=RHP_OPENSSL_LINK_MODE={}",
        if statik { "static" } else { "dynamic" }
    );
    if !statik {
        // OPENSSL_STATIC=0: an explicit experiment, as RHP_<NAME>_LINK=dynamic.
        return Ok(());
    }
    // pkg-config never links a library under /usr statically, so openssl-sys
    // links statically only from a directory it was given.
    let libdir = match var("OPENSSL_LIB_DIR") {
        Some(d) => PathBuf::from(d),
        None => match var("OPENSSL_DIR") {
            Some(d) => PathBuf::from(d).join("lib"),
            None if os == "macos" => found.libdir.clone(),
            None => {
                return Err(format!(
                    "openssl-sys would find OpenSSL through pkg-config, which links {} \
                     dynamically. Set OPENSSL_LIB_DIR={} and OPENSSL_INCLUDE_DIR={}.",
                    found.libdir.display(),
                    found.libdir.display(),
                    found.include.display()
                ))
            }
        },
    };
    if !same(&libdir, &found.libdir) {
        return Err(format!(
            "openssl-sys links OpenSSL from {}, but the OpenSSL php links is in {} ({})",
            libdir.display(),
            found.libdir.display(),
            found.via
        ));
    }
    // openssl-sys links only libssl and libcrypto: the rest of a static
    // libcrypto's Libs.private is linked as for the other rows (Ubuntu's:
    // `-l:libjitterentropy.a -lz -lzstd -ldl`).
    if let Some(pc) = &found.pc_file {
        link_private(lib, pc, &os)?;
    }
    for name in ["libcrypto.a", "libssl.a"] {
        let archive = found.libdir.join(name);
        if !archive.is_file() {
            return Err(format!(
                "OpenSSL {version} ({}) has no static archive {}. Install the package that \
                 ships it ({}), or set OPENSSL_STATIC=0 for an experiment.",
                found.via,
                archive.display(),
                lib.install
            ));
        }
    }
    Ok(())
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
