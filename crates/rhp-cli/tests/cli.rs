//! CLI fixtures: behaviour that needs the `rhp` binary as a process (argv,
//! stdin, `-d` and `-r`, stderr, the exit status, `pcntl_fork`). Every
//! `tests/cli/CASE/` directory holds one or more runs; each run's committed
//! stdout, stderr and exit status are php's (see docs/testing.md).
//!
//! A run `NAME` is a `NAME.stdout` file, with `NAME.stderr` (absent: empty),
//! `NAME.status` (absent: 0) and `NAME.cmd` (directives, optional) next to
//! it. A case with a single run names it `expected`. The script is
//! `{DIR}/NAME.php` if it exists, else `{DIR}/main.php`; the engine runs as
//! `BIN -n [-d INI]... [OPT]... [SCRIPT] [ARG]...` from the case directory
//! with a cleared environment (`PATH` kept, `TZ=UTC`). `{DIR}`, the case's
//! absolute path, is expanded in directives and replaces that path in the
//! output before it is compared.
//!
//! Directives, one per line, as in `tests/cgi` (`common.cmd` holds the ones
//! every run of the case shares; `#` starts a comment line; a line starting
//! with `php ` or `rhp ` applies to that engine only):
//!
//! - `ini K=V`: a `-d K=V` option.
//! - `opt A`: an option before the script (`opt -r` then `opt CODE`).
//! - `arg A`: an argument after the script.
//! - `script PATH`: the script argument, verbatim; `noscript`: none.
//! - `stdin TEXT`: append TEXT to stdin (`\r`, `\n` decoded);
//!   `stdinfile PATH`: append a file of the case.
//! - `env K=V`: an environment variable.
//! - `cwd PATH`: the working directory, relative to the case directory.
//!
//! `cargo test -p rhp-cli --test cli` runs rhp; `-- --check` also runs php
//! (`RHP_PHP`) and requires it to print the committed output; `-- --write`
//! rewrites the expectations from the reference php. `CLI_ONLY=SUBSTR`
//! narrows the runs to those whose `CASE/NAME` contains SUBSTR.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

#[derive(Default)]
struct Spec {
    ini: Vec<String>,
    opts: Vec<String>,
    args: Vec<String>,
    script: Option<String>,
    stdin: Vec<u8>,
    env: Vec<(String, String)>,
    cwd: PathBuf,
}

struct Output {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    status: String,
}

fn spec(case: &Path, name: &str, engine: &str) -> Spec {
    let dir = case.to_string_lossy().into_owned();
    let script = [format!("{name}.php"), "main.php".to_string()]
        .into_iter()
        .map(|f| case.join(f))
        .find(|p| p.is_file());
    let mut s = Spec {
        script: script.map(|p| p.to_string_lossy().into_owned()),
        cwd: case.to_path_buf(),
        ..Spec::default()
    };
    let read = |f: &str| std::fs::read_to_string(case.join(f)).unwrap_or_default();
    for line in read("common.cmd")
        .lines()
        .chain(read(&format!("{name}.cmd")).lines())
    {
        let line = line.replace("{DIR}", &dir);
        let line = match line.split_once(' ') {
            Some((e @ ("php" | "rhp"), rest)) if e == engine => rest.to_string(),
            Some(("php" | "rhp", _)) => continue,
            _ => line,
        };
        let (kw, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        let rest = rest.to_string();
        match kw {
            "ini" => s.ini.push(rest),
            "opt" => s.opts.push(rest),
            "arg" => s.args.push(rest),
            "script" => s.script = Some(rest),
            "noscript" => s.script = None,
            "stdin" => s
                .stdin
                .extend(rest.replace("\\r", "\r").replace("\\n", "\n").bytes()),
            "stdinfile" => s
                .stdin
                .extend(std::fs::read(case.join(&rest)).expect("stdinfile")),
            "env" => {
                let (k, v) = rest.split_once('=').unwrap_or((&rest, ""));
                s.env.push((k.to_string(), v.to_string()));
            }
            "cwd" => s.cwd = case.join(rest),
            "" => {}
            _ if kw.starts_with('#') => {}
            other => panic!("{}/{name}: unknown directive {other}", case.display()),
        }
    }
    s
}

fn replace(hay: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(hay.len());
    let mut i = 0;
    while i < hay.len() {
        if hay[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(hay[i]);
            i += 1;
        }
    }
    out
}

fn run(bin: &str, case: &Path, s: &Spec) -> Output {
    let mut cmd = Command::new(bin);
    cmd.env_clear().env("TZ", "UTC").arg("-n");
    if let Some(path) = std::env::var_os("PATH") {
        cmd.env("PATH", path);
    }
    for i in &s.ini {
        cmd.arg("-d").arg(i);
    }
    cmd.args(&s.opts).args(&s.script).args(&s.args);
    cmd.envs(s.env.iter().map(|(k, v)| (k, v)));
    let mut child = cmd
        .current_dir(&s.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {bin}: {e}"));
    let mut stdin = child.stdin.take().unwrap();
    let body = s.stdin.clone();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&body);
    });
    let out = child.wait_with_output().expect("wait");
    let _ = writer.join();
    let dir = case.to_string_lossy();
    let norm = |b: &[u8]| replace(b, dir.as_bytes(), b"{DIR}");
    Output {
        stdout: norm(&out.stdout),
        stderr: norm(&out.stderr),
        status: out.status.code().map_or("signal".into(), |c| c.to_string()),
    }
}

fn expected(case: &Path, name: &str) -> Output {
    let read = |ext: &str| std::fs::read(case.join(format!("{name}.{ext}")));
    Output {
        stdout: read("stdout").unwrap_or_default(),
        stderr: read("stderr").unwrap_or_default(),
        status: read("status").map_or("0".into(), |s| {
            String::from_utf8_lossy(&s).trim().to_string()
        }),
    }
}

fn write_expected(case: &Path, name: &str, out: &Output) {
    let path = |ext: &str| case.join(format!("{name}.{ext}"));
    std::fs::write(path("stdout"), &out.stdout).expect("write stdout");
    for (ext, body, empty) in [
        ("stderr", out.stderr.clone(), out.stderr.is_empty()),
        (
            "status",
            format!("{}\n", out.status).into_bytes(),
            out.status == "0",
        ),
    ] {
        if empty {
            let _ = std::fs::remove_file(path(ext));
        } else {
            std::fs::write(path(ext), body).expect("write expectation");
        }
    }
}

fn differences(engine: &str, want: &Output, got: &Output) -> Vec<String> {
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    let mut d = Vec::new();
    for (what, w, g) in [
        ("stdout", &want.stdout, &got.stdout),
        ("stderr", &want.stderr, &got.stderr),
    ] {
        if w != g {
            d.push(format!(
                "{what}:\n--- expected\n{}\n--- {engine}\n{}",
                text(w),
                text(g)
            ));
        }
    }
    if want.status != got.status {
        d.push(format!(
            "exit: expected {}, {engine} {}",
            want.status, got.status
        ));
    }
    d
}

/// Every run, as (case directory, run name), sorted, and the expectation
/// or directive files that belong to no run.
fn runs(root: &Path) -> (Vec<(PathBuf, String)>, Vec<PathBuf>) {
    let sorted = |dir: &Path| {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        v.sort();
        v
    };
    let (mut runs, mut stray) = (Vec::new(), Vec::new());
    for case in sorted(root).into_iter().filter(|p| p.is_dir()) {
        for f in sorted(&case) {
            let ext = f.extension().map(|e| e.to_string_lossy().into_owned());
            let name = f.file_stem().unwrap().to_string_lossy().into_owned();
            match ext.as_deref() {
                Some("stdout") => runs.push((case.clone(), name)),
                Some("cmd") if name == "common" => {}
                Some("stderr" | "status" | "cmd") if !f.with_extension("stdout").exists() => {
                    stray.push(f)
                }
                _ => {}
            }
        }
    }
    (runs, stray)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let (check, write) = (
        args.contains(&"--check".into()),
        args.contains(&"--write".into()),
    );
    let php = rhp_difftest::php_bin();
    if check || write {
        match rhp_difftest::golden::php_build() {
            None => {
                eprintln!("cli: cannot run `{php} -n` (set RHP_PHP)");
                return ExitCode::FAILURE;
            }
            Some(b) if write && !b.reference => {
                eprintln!(
                    "cli: --write needs the reference php, not {} on {}",
                    b.version, b.os
                );
                return ExitCode::FAILURE;
            }
            Some(_) => {}
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli");
    let root = root.canonicalize().expect("tests/cli");
    let only = std::env::var("CLI_ONLY").ok();
    let rhp = env!("CARGO_BIN_EXE_rhp");
    let (runs, stray) = runs(&root);
    let mut failures: Vec<String> = stray
        .iter()
        .map(|f| format!("== {}: no .stdout next to it", f.display()))
        .collect();
    let mut total = 0;
    for (case, name) in runs {
        let id = format!("{}/{name}", case.file_name().unwrap().to_string_lossy());
        if only.as_deref().is_some_and(|o| !id.contains(o)) {
            continue;
        }
        total += 1;
        let mut want = expected(&case, &name);
        if write {
            want = run(&php, &case, &spec(&case, &name, "php"));
            write_expected(&case, &name, &want);
        }
        let mut engines = vec![("rhp", rhp)];
        if check {
            engines.push(("php", &php));
        }
        for (engine, bin) in engines {
            let got = run(bin, &case, &spec(&case, &name, engine));
            let d = differences(engine, &want, &got);
            if !d.is_empty() {
                failures.push(format!("== {id} ({engine})\n{}", d.join("\n")));
            }
        }
    }
    for f in &failures {
        eprintln!("{f}");
    }
    eprintln!("cli: {total} run(s), {} failure(s)", failures.len());
    assert!(total > 0 || only.is_some(), "no CLI fixtures");
    if failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
