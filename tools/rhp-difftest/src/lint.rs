//! `rhp-difftest lint` — a compile-only scan of PHP source trees.
//!
//! Every `.php` file under the given directories goes through the exact path
//! an `include` takes in rhp (`rhp_sapi::compile_file`), without running
//! anything. A file rhp *refuses* (parse error, compile-time
//! `Unsupported`, compile error or compiler panic) is a file that would fatal
//! the moment an app includes it, so the refused count over a framework's
//! `vendor/` is a cheap, monotone progress metric.
//!
//! Like `phpt --check-known`, a *known-refused* list ratchets it: with
//! `--check-known <file>` the scan fails only on a refusal that is **not** on
//! the list. Listed files that now compile are reported (and, under GitHub
//! Actions, emitted as `::notice` annotations) so the list can be shrunk, but
//! don't fail the run unless `--strict-known` is given — a change elsewhere
//! that makes more files compile must never break CI.
//!
//! Files are keyed by `<root dir name>/<path below the root>`, e.g.
//! `vendor/symfony/console/Application.php`, so a list is independent of the
//! directory the scan runs from.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use rhp_sapi::{IncludeError, Mode};
use rhp_values::Thrown;

use crate::read_known;

const USAGE: &str = "\
usage: rhp-difftest lint [options] <dir|file>...

Parses and compiles every .php file under the given paths the way rhp compiles
an included file, and reports each file rhp refuses.

  --check-known <file>   exit 1 if a file NOT on this list is refused; listed
                         files that now compile are reported as shrinkable
  --strict-known         with --check-known, also fail on shrinkable/missing entries
  --write-known <file>   write the sorted refused files (the known-refused list)
  --php-lint             run `php -l` on each refused file and set aside the ones
                         php rejects too (not rhp gaps; never counted as refusals)
  --jobs <n>             parallel workers (default: all cores)
  --top <n>              refusal reasons to print in the summary (default: 20)
  -q, --quiet            don't list every refused file, only the summary
";

/// Why rhp refused a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    Parse {
        line: u32,
        message: String,
    },
    /// `line` is where the compiler located the refusal, if it did (the
    /// file is the one being scanned, so it is not repeated here).
    Unsupported {
        feature: String,
        reason: String,
        line: Option<u32>,
    },
    Error {
        class: String,
        message: String,
    },
    Panic(String),
}

impl Refusal {
    /// A short, stable grouping key for the summary (no line numbers).
    pub fn cluster(&self) -> String {
        match self {
            Refusal::Parse { message, .. } => format!("parse: {}", first_line(message)),
            Refusal::Unsupported { feature, .. } => format!("unsupported: {feature}"),
            Refusal::Error { class, message } => format!("{class}: {}", first_line(message)),
            Refusal::Panic(m) => format!("panic: {}", first_line(m)),
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Parse { line, message } => {
                write!(f, "parse error: {} (line {line})", first_line(message))
            }
            Refusal::Unsupported {
                feature,
                reason,
                line,
            } => {
                write!(f, "unsupported {feature}: {}", first_line(reason))?;
                match line {
                    Some(l) => write!(f, " (line {l})"),
                    None => Ok(()),
                }
            }
            Refusal::Error { class, message } => write!(f, "{class}: {}", first_line(message)),
            Refusal::Panic(m) => write!(f, "compiler panic: {}", first_line(m)),
        }
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("").trim()
}

/// Parse and compile one source file exactly like rhp's includer does.
pub fn lint_source(file: &str, src: &[u8]) -> Result<(), Refusal> {
    let res = catch_unwind(AssertUnwindSafe(|| {
        match rhp_sapi::compile_file(file, src, Mode::Include) {
            Ok(_) => Ok(()),
            Err(IncludeError::Parse { message, line, .. }) => Err(Refusal::Parse { line, message }),
            Err(IncludeError::Compile { message, .. }) => Err(Refusal::Error {
                class: "CompileError".into(),
                message,
            }),
            Err(IncludeError::Thrown(Thrown::Unsupported {
                feature,
                reason,
                location,
            })) => Err(Refusal::Unsupported {
                feature,
                reason,
                line: location.map(|(_, l)| l),
            }),
            Err(IncludeError::Thrown(Thrown::Error { class, message })) => Err(Refusal::Error {
                class: class.to_string(),
                message,
            }),
            Err(IncludeError::HardLimit { line }) => Err(Refusal::Error {
                class: "HardLimit".into(),
                message: format!("hard memory limit exhausted compiling line {line}"),
            }),
            Err(IncludeError::Thrown(other)) => Err(Refusal::Error {
                class: "Thrown".into(),
                message: format!("{other:?}"),
            }),
        }
    }));
    match res {
        Ok(r) => r,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "(non-string panic payload)".into());
            Err(Refusal::Panic(msg))
        }
    }
}

/// One `.php` file to scan: its list key and where it is on disk.
struct Entry {
    key: String,
    path: PathBuf,
}

/// Collect the `.php` files under `root` (a directory or a single file),
/// sorted by key. Symlinked directories are not followed (no cycles).
fn collect(root: &Path, out: &mut Vec<Entry>) -> std::io::Result<()> {
    let base = root
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    if root.is_file() {
        out.push(Entry {
            key: base,
            path: root.to_path_buf(),
        });
        return Ok(());
    }
    let mut stack = vec![(root.to_path_buf(), base)];
    while let Some((dir, key)) = stack.pop() {
        for ent in fs::read_dir(&dir)? {
            let ent = ent?;
            let ft = ent.file_type()?;
            let name = ent.file_name().to_string_lossy().into_owned();
            let path = ent.path();
            let k = format!("{key}/{name}");
            if ft.is_dir() {
                stack.push((path, k));
            } else if name.ends_with(".php") && (ft.is_file() || path.is_file()) {
                out.push(Entry { key: k, path });
            }
        }
    }
    Ok(())
}

/// Does `php -l` accept this file? `None` if php could not be run.
fn php_lint_ok(path: &Path) -> Option<bool> {
    Command::new(crate::php_bin())
        .args(["-n", "-l"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .map(|s| s.success())
}

struct Opts {
    roots: Vec<PathBuf>,
    check_known: Option<PathBuf>,
    strict_known: bool,
    write_known: Option<PathBuf>,
    php_lint: bool,
    jobs: usize,
    top: usize,
    quiet: bool,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        roots: Vec::new(),
        check_known: None,
        strict_known: false,
        write_known: None,
        php_lint: false,
        jobs: std::thread::available_parallelism().map_or(4, |n| n.get()),
        top: 20,
        quiet: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{a} needs a value"))
        };
        match a.as_str() {
            "--check-known" => o.check_known = Some(val()?.into()),
            "--strict-known" => o.strict_known = true,
            "--write-known" => o.write_known = Some(val()?.into()),
            "--php-lint" => o.php_lint = true,
            "--jobs" => {
                o.jobs = val()?
                    .parse::<usize>()
                    .map_err(|_| "bad --jobs".to_string())?
                    .max(1)
            }
            "--top" => o.top = val()?.parse().map_err(|_| "bad --top".to_string())?,
            "-q" | "--quiet" => o.quiet = true,
            "-h" | "--help" => return Err(String::new()),
            s if s.starts_with('-') => return Err(format!("unknown option {s:?}")),
            _ => o.roots.push(a.into()),
        }
    }
    if o.roots.is_empty() {
        return Err("give at least one directory to scan".into());
    }
    Ok(o)
}

/// `rhp-difftest lint ...` entry point.
pub fn main(args: &[String]) -> ExitCode {
    let o = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("rhp-difftest lint: {e}\n");
            }
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let known = match &o.check_known {
        Some(p) => match read_known(p) {
            Ok(k) => Some(k.into_iter().collect::<BTreeSet<_>>()),
            Err(e) => {
                eprintln!("rhp-difftest lint: cannot read {}: {e}", p.display());
                return ExitCode::from(2);
            }
        },
        None => None,
    };

    let mut files = Vec::new();
    for r in &o.roots {
        if let Err(e) = collect(r, &mut files) {
            eprintln!("rhp-difftest lint: cannot scan {}: {e}", r.display());
            return ExitCode::from(2);
        }
    }
    files.sort_by(|a, b| a.key.cmp(&b.key));
    // The same root given twice is scanned once; two *different* files under one key
    // (roots with the same directory name, `a/src` and `b/src`) would make
    // the known list ambiguous, so refuse that.
    if let Some(w) = files.windows(2).find(|w| {
        w[0].key == w[1].key && w[0].path.canonicalize().ok() != w[1].path.canonicalize().ok()
    }) {
        eprintln!(
            "rhp-difftest lint: {} and {} both map to the list key {:?}; \
             scan roots must have distinct directory names",
            w[0].path.display(),
            w[1].path.display(),
            w[0].key
        );
        return ExitCode::from(2);
    }
    files.dedup_by(|a, b| a.key == b.key);

    // Compiler panics are reported as refusals; keep their default
    // backtrace spew off the terminal while the workers run.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let results = scan(&files, o.jobs);
    std::panic::set_hook(prev_hook);

    let mut refused: BTreeMap<&str, (&Entry, Refusal)> = BTreeMap::new();
    let mut read_errors = Vec::new();
    for (e, r) in files.iter().zip(results) {
        match r {
            Ok(Ok(())) => {}
            Ok(Err(refusal)) => {
                refused.insert(&e.key, (e, refusal));
            }
            Err(io) => read_errors.push((&e.key, io)),
        }
    }

    // Files php rejects as well are broken (or deliberately invalid test
    // fixtures), not rhp gaps.
    let mut php_rejects = Vec::new();
    if o.php_lint && !refused.is_empty() {
        if !crate::php_available() {
            eprintln!("rhp-difftest lint: --php-lint needs `php` (set RHP_PHP)");
            return ExitCode::from(2);
        }
        let keys: Vec<&str> = refused.keys().copied().collect();
        let paths: Vec<&Path> = keys.iter().map(|k| refused[k].0.path.as_path()).collect();
        let oks = par_map(&paths, o.jobs, |p| php_lint_ok(p));
        for (k, ok) in keys.into_iter().zip(oks) {
            if ok == Some(false) {
                php_rejects.push(refused.remove(k).unwrap());
            }
        }
    }

    if !o.quiet {
        for (key, (_, r)) in &refused {
            println!("{key}: {r}");
        }
    }

    // Summary by reason.
    let mut clusters: BTreeMap<String, usize> = BTreeMap::new();
    for (_, r) in refused.values() {
        *clusters.entry(r.cluster()).or_default() += 1;
    }
    let mut clusters: Vec<_> = clusters.into_iter().collect();
    clusters.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    println!(
        "\n{} file(s) scanned, {} compile, {} refused by rhp",
        files.len(),
        files.len() - refused.len() - php_rejects.len() - read_errors.len(),
        refused.len()
    );
    if !php_rejects.is_empty() {
        println!(
            "{} file(s) rejected by `php -l` too (not counted):",
            php_rejects.len()
        );
        for (e, r) in &php_rejects {
            println!("  {}: {r}", e.key);
        }
    }
    for (k, io) in &read_errors {
        println!("unreadable: {k}: {io}");
    }
    if !clusters.is_empty() {
        println!("\ntop refusal reasons:");
        for (c, n) in clusters.iter().take(o.top) {
            println!("{n:>6}  {c}");
        }
    }

    if let Some(p) = &o.write_known {
        let mut s = String::from(
            "# Files rhp refuses to compile (rhp-difftest lint --write-known).\n\
             # `rhp-difftest lint --check-known` fails on a refused file not listed here;\n\
             # delete lines as files start compiling. Format: <file>\\t# <reason>\n\
             # Only <file> is matched; the reason is a comment and may change freely.\n",
        );
        for (key, (_, r)) in &refused {
            s.push_str(&format!("{key}\t# {r}\n"));
        }
        if let Err(e) = fs::write(p, s) {
            eprintln!("rhp-difftest lint: cannot write {}: {e}", p.display());
            return ExitCode::from(2);
        }
        println!(
            "\nwrote {} known-refused file(s) to {}",
            refused.len(),
            p.display()
        );
    }

    let Some(known) = known else {
        return ExitCode::SUCCESS;
    };
    let gha = std::env::var_os("GITHUB_ACTIONS").is_some_and(|v| v == "true");
    let scanned: BTreeSet<&str> = files.iter().map(|e| e.key.as_str()).collect();
    let new: Vec<_> = refused
        .iter()
        .filter(|(k, _)| !known.contains(**k))
        .collect();
    let fixed: Vec<&String> = known
        .iter()
        .filter(|k| scanned.contains(k.as_str()) && !refused.contains_key(k.as_str()))
        .collect();
    let missing: Vec<&String> = known
        .iter()
        .filter(|k| !scanned.contains(k.as_str()))
        .collect();
    let list = o.check_known.as_ref().unwrap().display();

    if !fixed.is_empty() {
        println!(
            "\n{} known-refused file(s) now compile — remove them from {list}:",
            fixed.len()
        );
        for k in &fixed {
            println!("  {k}");
        }
        if gha {
            println!(
                "::notice title=rhp lint::{} file(s) in {list} now compile; the list can shrink",
                fixed.len()
            );
        }
    }
    if !missing.is_empty() {
        println!(
            "\n{} known-refused file(s) no longer exist (stale entries in {list}):",
            missing.len()
        );
        for k in &missing {
            println!("  {k}");
        }
    }
    if !new.is_empty() {
        println!("\n{} NEW refused file(s), not in {list}:", new.len());
        for (k, (_, r)) in &new {
            println!("  {k}: {r}");
            if gha {
                println!("::error title=rhp lint: new refusal::{k}: {r}");
            }
        }
        return ExitCode::FAILURE;
    }
    if o.strict_known && (!fixed.is_empty() || !missing.is_empty()) {
        println!("\n--strict-known: {list} is out of date");
        return ExitCode::FAILURE;
    }
    println!("\nno new refusals ({} known)", known.len());
    ExitCode::SUCCESS
}

/// Lint every file on `jobs` threads; results are in input order. An `Err`
/// is a file that could not be read.
fn scan(files: &[Entry], jobs: usize) -> Vec<Result<Result<(), Refusal>, String>> {
    par_map(files, jobs, |e| {
        let src = fs::read(&e.path).map_err(|err| err.to_string())?;
        let file = e
            .path
            .canonicalize()
            .unwrap_or_else(|_| e.path.clone())
            .to_string_lossy()
            .into_owned();
        Ok(lint_source(&file, &src))
    })
}

/// Order-preserving parallel map on scoped threads with deep stacks (the
/// compiler recurses over the AST; vendor code nests deeply).
fn par_map<T: Sync, R: Send>(items: &[T], jobs: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<R>>> = Mutex::new((0..items.len()).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..jobs.min(items.len()).max(1) {
            std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn_scoped(s, || loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let r = f(item);
                    out.lock().unwrap()[i] = Some(r);
                })
                .expect("spawn lint worker");
        }
    });
    out.into_inner()
        .unwrap()
        .into_iter()
        .map(|r| r.expect("every item processed"))
        .collect()
}

#[cfg(test)]
mod tests;
