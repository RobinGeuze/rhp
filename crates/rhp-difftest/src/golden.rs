//! `rhp-difftest golden --check|--write`: re-run php on the golden fixtures.
//!
//! Every `tests/golden/**/NAME.php` has a `NAME.out`: the stdout of the
//! reference php (`rhp_builtins::version`: php 8.5.12, the Homebrew build on
//! macOS) for it, run the canonical way:
//!
//! - `php -n /absolute/path/NAME.php` (`RHP_PHP` names the php),
//! - from the fixture's directory,
//! - with the environment inherited and stdin empty;
//! - only stdout is recorded (not stderr, not the exit status).
//!
//! `--check` runs php on every fixture and compares with the committed
//! `.out`; `--write` rewrites the `.out` files. The fixtures listed in
//! `tests/golden/reference-only.txt` print facts of the reference build (its
//! configuration, library versions and features); `--check --portable` skips
//! them, so another build of php 8.5.12 (CI's) checks the rest.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use crate::php_bin;

const USAGE: &str = "usage: rhp-difftest golden (--check [--portable] | --write) \
[--filter SUBSTR] [--dir DIR]\n\
\x20 --check     run php on every fixture and compare with its .out\n\
\x20 --portable  skip the fixtures in DIR/reference-only.txt\n\
\x20 --write     rewrite the .out files (reference php only)\n\
\x20 --filter    only fixtures whose name contains SUBSTR\n\
\x20 --dir       the golden directory (default: tests/golden)\n";

/// The file listing the reference-build-only fixtures, in the golden dir.
pub const REFERENCE_ONLY: &str = "reference-only.txt";

/// The reference build's `PHP_OS_FAMILY`: only it may write the
/// reference-only fixtures.
const REFERENCE_OS_FAMILY: &str = "Darwin";

/// One fixture: its name (path below the golden dir, no extension) and
/// script.
pub struct Fixture {
    pub name: String,
    pub php: PathBuf,
}

impl Fixture {
    pub fn out(&self) -> PathBuf {
        self.php.with_extension("out")
    }
}

/// Every `*.php` below `dir`, sorted by name.
pub fn fixtures(dir: &Path) -> std::io::Result<Vec<Fixture>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<Fixture>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(root, &path, out)?;
            } else if path.extension().is_some_and(|e| e == "php") {
                let rel = path.strip_prefix(root).unwrap_or(&path).with_extension("");
                let name = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push(Fixture { name, php: path });
            }
        }
        Ok(())
    }
    let mut v = Vec::new();
    walk(dir, dir, &mut v)?;
    v.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(v)
}

/// The names in `dir/reference-only.txt` (`#` starts a comment).
pub fn reference_only(dir: &Path) -> std::io::Result<Vec<String>> {
    let text = std::fs::read_to_string(dir.join(REFERENCE_ONLY))?;
    Ok(text
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// Run php on `php` the canonical way; its stdout and stderr.
pub fn run_php_fixture(php: &Path) -> Result<(Vec<u8>, Vec<u8>), String> {
    let dir = php.parent().unwrap_or(Path::new("."));
    let out = Command::new(php_bin())
        .arg("-n")
        .arg(php)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("spawn {}: {e}", php_bin()))?;
    Ok((out.stdout, out.stderr))
}

/// php's answer to `code` (`php -n -r`).
fn php_eval(code: &str) -> Option<String> {
    let out = Command::new(php_bin())
        .args(["-n", "-r", code])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The first line where `want` and `got` differ, for a report.
fn first_difference(want: &[u8], got: &[u8]) -> String {
    let (w, g) = (String::from_utf8_lossy(want), String::from_utf8_lossy(got));
    let mut wl = w.split_inclusive('\n');
    let mut gl = g.split_inclusive('\n');
    let mut line = 1;
    loop {
        match (wl.next(), gl.next()) {
            (Some(a), Some(b)) if a == b => line += 1,
            (None, None) => return "same lines, different bytes".to_string(),
            (a, b) => {
                return format!(
                    "line {line}\n    .out: {:?}\n    php:  {:?}",
                    a.unwrap_or("<end>"),
                    b.unwrap_or("<end>")
                )
            }
        }
    }
}

pub fn main(args: &[String]) -> ExitCode {
    let flag = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let (check, write, portable) = (flag("--check"), flag("--write"), flag("--portable"));
    if check == write || (write && portable) {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    }
    let dir = PathBuf::from(value("--dir").unwrap_or_else(|| "tests/golden".to_string()));
    let dir = match dir.canonicalize() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("golden: {}: {e}", dir.display());
            return ExitCode::from(2);
        }
    };
    let filter = value("--filter");
    let (all, reference) = match (fixtures(&dir), reference_only(&dir)) {
        (Ok(f), Ok(r)) => (f, r),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("golden: {}: {e}", dir.display());
            return ExitCode::from(2);
        }
    };
    let stale: Vec<_> = reference
        .iter()
        .filter(|n| !all.iter().any(|f| &f.name == *n))
        .collect();
    if !stale.is_empty() {
        eprintln!("golden: {REFERENCE_ONLY} names no such fixture: {stale:?}");
        return ExitCode::from(2);
    }
    let Some(about) = php_eval("echo PHP_VERSION, ' ', PHP_OS_FAMILY;") else {
        eprintln!("golden: cannot run `{} -n` (set RHP_PHP)", php_bin());
        return ExitCode::from(2);
    };
    let (version, os) = about.split_once(' ').unwrap_or((&about, ""));
    eprintln!("golden: php {version} ({os})");
    let reference_build =
        version == rhp_builtins::version::PHP_VERSION && os == REFERENCE_OS_FAMILY;
    if write && version != rhp_builtins::version::PHP_VERSION {
        eprintln!(
            "golden: --write needs the reference php {}, not {version}",
            rhp_builtins::version::PHP_VERSION
        );
        return ExitCode::from(2);
    }

    let (mut ok, mut skipped, mut written) = (0, 0, 0);
    let mut failures = Vec::new();
    for f in &all {
        if filter
            .as_ref()
            .is_some_and(|s| !f.name.contains(s.as_str()))
        {
            continue;
        }
        let is_reference = reference.contains(&f.name);
        if is_reference && (portable || (write && !reference_build)) {
            skipped += 1;
            continue;
        }
        let (stdout, stderr) = match run_php_fixture(&f.php) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("golden: {e}");
                return ExitCode::from(2);
            }
        };
        let committed = std::fs::read(f.out()).ok();
        if write {
            if committed.as_deref() != Some(&stdout[..]) {
                if let Err(e) = std::fs::write(f.out(), &stdout) {
                    eprintln!("golden: cannot write {}: {e}", f.out().display());
                    return ExitCode::FAILURE;
                }
                println!("  wrote     {}", f.name);
                written += 1;
            }
            ok += 1;
            continue;
        }
        match committed {
            Some(want) if want == stdout => ok += 1,
            Some(want) => {
                let mut msg = format!("{}: {}", f.name, first_difference(&want, &stdout));
                if !stderr.is_empty() {
                    let err = String::from_utf8_lossy(&stderr);
                    msg.push_str(&format!("\n    stderr: {:?}", err.trim_end()));
                }
                failures.push(msg);
            }
            None => failures.push(format!("{}: no .out", f.name)),
        }
    }
    if check && filter.is_none() {
        // An .out without its script is a leftover.
        let mut outs = Vec::new();
        collect_outs(&dir, &mut outs);
        for out in outs {
            if !out.with_extension("php").exists() {
                failures.push(format!("{}: .out without a .php", out.display()));
            }
        }
    }

    for f in &failures {
        println!("  MISMATCH  {f}");
    }
    if write {
        println!(
            "golden: {ok} fixture(s), {written} .out rewritten, {skipped} reference-only skipped"
        );
    } else {
        println!(
            "golden: {ok} match, {} mismatch, {skipped} reference-only skipped",
            failures.len()
        );
    }
    if failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn collect_outs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_outs(&p, out);
        } else if p.extension().is_some_and(|e| e == "out") {
            out.push(p);
        }
    }
}
