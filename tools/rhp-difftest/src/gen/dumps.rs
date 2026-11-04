//! `gen dumps [--check]`: ini_table.rs and const_modules.rs from the
//! reference php (Homebrew's php on macOS, the release rhp reports), through
//! `tools/capture-ref-build.php tables`: ini_get_all() under `php -n` plus
//! php-cgi's own directives under `php-cgi -n`, and get_defined_constants(true).
//! Elsewhere it skips: another build has other paths, extensions and defaults.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use super::util::{finish, repo_root};

const INI: &str = "crates/rhp-builtins/src/main/ini_table.rs";
const CONSTS: &str = "crates/rhp-builtins/src/const_modules.rs";

/// Directives whose owner rhp does not load but which rhp shows anyway, as
/// core ones (rhp has no FFI).
const OWNER_OVERRIDES: &[(&str, &str)] = &[("ffi.enable", "core"), ("ffi.preload", "core")];

/// Constants an extension's stub declares only for a newer library than the
/// reference php links (an `#if` on the library version): (module, name,
/// the constant it follows). get_defined_constants() lists a name only when
/// rhp defines it, so an entry is harmless where the library lacks it.
const CONST_EXTRAS: &[(&str, &str, &str)] = &[
    // libxml.stub.php: #if LIBXML_VERSION >= 21300 (the mac's SDK has 2.9)
    ("libxml", "LIBXML_NO_XXE", "LIBXML_NOENT"),
];

pub fn main(args: &[String]) -> ExitCode {
    let check = args.iter().any(|a| a == "--check");
    let php = PathBuf::from(crate::php_bin());
    if let Some(why) = not_reference(&php) {
        eprintln!("gen dumps: skipped: {why}");
        return ExitCode::SUCCESS;
    }
    finish("dumps", check, files(&php))
}

fn files(php: &Path) -> Result<Vec<(&'static str, String)>, String> {
    let cgi = crate::phpt::find_php_cgi(None).ok_or("no php-cgi next to the reference php")?;
    let script = repo_root().join("tools/capture-ref-build.php");
    let cli = run(php, &["-n".as_ref(), script.as_os_str(), "tables".as_ref()])?;
    let cgi = run(
        &cgi,
        &[
            "-n".as_ref(),
            "-q".as_ref(),
            "-f".as_ref(),
            script.as_os_str(),
            "--".as_ref(),
            "tables".as_ref(),
        ],
    )?;
    Ok(vec![
        (INI, ini_table(&cli, &cgi)?),
        (CONSTS, const_modules(&cli)?),
    ])
}

/// Why `php` is not the reference build, if it is not.
fn not_reference(php: &Path) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return Some("the reference php is Homebrew's on macOS".into());
    }
    let code = "echo PHP_BUILD_PROVIDER, ' ', PHP_VERSION;";
    let Ok(out) = run(php, &["-n".as_ref(), "-r".as_ref(), code.as_ref()]) else {
        return Some(format!("{} does not run", php.display()));
    };
    let want = format!("Homebrew {}", rhp_builtins::version::PHP_VERSION);
    (out != want).then(|| format!("php is {out:?}, the reference is {want:?}"))
}

fn run(bin: &Path, args: &[&std::ffi::OsStr]) -> Result<String, String> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| format!("{}: {e}", bin.display()))?;
    if !out.status.success() {
        return Err(format!(
            "{} failed: {}",
            bin.display(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The lines of one `# name` section.
fn section<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let header = format!("# {name}");
    text.lines()
        .skip_while(|l| *l != header)
        .skip(1)
        .take_while(|l| !l.starts_with("# "))
        .collect()
}

/// A `var_export()`ed string or NULL.
fn unexport(v: &str) -> Result<Option<String>, String> {
    if v == "NULL" {
        return Ok(None);
    }
    let inner = v
        .strip_prefix('\'')
        .and_then(|v| v.strip_suffix('\''))
        .ok_or(format!("not an exported string: {v}"))?;
    Ok(Some(inner.replace("\\'", "'").replace("\\\\", "\\")))
}

/// One ini directive: (module, name, access, default).
type IniRow = (String, String, String, Option<String>);

/// The rows of an `# ini` section.
fn ini_rows(text: &str) -> Result<Vec<IniRow>, String> {
    section(text, "ini")
        .into_iter()
        .map(|l| {
            let f: Vec<&str> = l.splitn(4, '\t').collect();
            let [module, name, access, value] = f[..] else {
                return Err(format!("bad ini row {l:?}"));
            };
            Ok((
                module.to_string(),
                name.to_string(),
                access.to_string(),
                unexport(value)?,
            ))
        })
        .collect()
}

fn path_row<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    section(text, "paths")
        .into_iter()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(" = "))
}

fn ini_table(cli: &str, cgi: &str) -> Result<String, String> {
    let ext_dir = path_row(cli, "PHP_EXTENSION_DIR")
        .map(unexport)
        .transpose()?
        .flatten();
    let inc_path = path_row(cli, "DEFAULT_INCLUDE_PATH")
        .map(unexport)
        .transpose()?
        .flatten();
    let rows = ini_rows(cli)?;
    let cgi_rows: Vec<_> = ini_rows(cgi)?
        .into_iter()
        .filter(|r| !rows.iter().any(|c| c.1 == r.1))
        .collect();
    let line =
        |o: &mut String,
         (module, name, access, value): &(String, String, String, Option<String>)| {
            let module = OWNER_OVERRIDES
                .iter()
                .find(|(n, _)| n == name)
                .map_or(module.as_str(), |(_, m)| m);
            let value = match value {
                None => "None".to_string(),
                Some(v) if name == "extension_dir" && Some(v) == ext_dir.as_ref() => {
                    "Some(EXTENSION_DIR)".into()
                }
                Some(v) if name == "include_path" && Some(v) == inc_path.as_ref() => {
                    "Some(INCLUDE_PATH)".into()
                }
                Some(v) => format!("Some({v:?})"),
            };
            writeln!(o, "    ({module:?}, {name:?}, {value}, {access}),").unwrap();
        };
    let mut o = String::from(
        "//! The reference php's (`php -n`, see `version.rs`) ini directives:\n\
         //! (owning module lowercased, name, default value or None, access mask).\n\
         //!\n\
         //! Generated by `rhp-difftest gen dumps` from `tools/capture-ref-build.php\n\
         //! tables` (ini_get_all()). Do not edit.\n\n\
         // Compile-time paths (they show up in include failure messages).\n\
         use crate::version::{EXTENSION_DIR, INCLUDE_PATH};\n\n\
         pub(crate) const INI_TABLE: &[(&str, &str, Option<&str>, u8)] = &[\n",
    );
    for r in &rows {
        line(&mut o, r);
    }
    o.push_str(
        "    // php-cgi's own directives (sapi/cgi/cgi_main.c), present only under the\n    \
         // cgi-fcgi SAPI.\n",
    );
    for r in &cgi_rows {
        line(&mut o, r);
    }
    o.push_str("];\n");
    Ok(o)
}

fn const_modules(cli: &str) -> Result<String, String> {
    let mut o = String::from(
        "//! The reference php's constants by module, in its registration order:\n\
         //! get_defined_constants(true), plus the library-gated ones in\n\
         //! CONST_EXTRAS (tools/rhp-difftest/src/gen/dumps.rs). Used to\n\
         //! categorize and order `get_defined_constants()` output.\n\
         //!\n\
         //! Generated by `rhp-difftest gen dumps` from `tools/capture-ref-build.php\n\
         //! tables`. Do not edit.\n\n\
         pub(crate) const CONST_MODULES: &[(&str, &str)] = &[\n",
    );
    for l in section(cli, "constants") {
        let (module, names) = l
            .split_once('\t')
            .ok_or(format!("bad constants row {l:?}"))?;
        let mut names: Vec<&str> = names.split(' ').collect();
        for &(m, name, after) in CONST_EXTRAS {
            if m != module || names.contains(&name) {
                continue;
            }
            let i = names
                .iter()
                .position(|n| *n == after)
                .ok_or(format!("{module} has no {after} to put {name} after"))?;
            names.insert(i + 1, name);
        }
        writeln!(o, "    ({module:?}, r\"{}\"),", names.join(" ")).unwrap();
    }
    o.push_str("];\n");
    Ok(o)
}
