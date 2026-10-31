//! rhp CLI SAPI: `rhp [options] script.php [args...]` or `rhp [options] -r code`.
//!
//! Reads a file (or `-r` code), parses it, compiles it to bytecode, and runs it
//! on the register VM, writing output straight to stdout as it is produced (as
//! php's CLI does), and exits with the script's exit code. Parse errors,
//! compile-time refusals, and uncaught throws go to stderr with a nonzero exit.
//!
//! The php CLI options composer and friends pass when re-invoking `PHP_BINARY`
//! are accepted: `-n`, `-q`, `-H`, `-e`, `-c <path>`, `-d key[=value]` (ini
//! settings — accepted and ignored: this engine has no ini store yet), `-f
//! <file>`, `-r <code>`, and `--` to end option parsing.

use std::io::Write;
use std::process::ExitCode;

use rhp_compiler::compile_with_source;
use rhp_frontend::Arena;
use rhp_values::{Thrown, Value};
use rhp_vm::Vm;

/// What to run.
enum Script {
    File(String),
    Code(String),
}

fn usage() -> ExitCode {
    eprintln!("usage: rhp [options] <script.php> [args...] | rhp [options] -r <code> [args...]");
    ExitCode::from(1)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    // --- option parsing (php_cli.c's getopt subset) ---------------------------
    let mut i = 1;
    let mut script: Option<Script> = None;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--" => {
                i += 1;
                break;
            }
            "-n" | "-q" | "-H" | "-e" => i += 1,
            "-c" | "-d" => i += 2,
            "-f" => {
                let Some(f) = args.get(i + 1) else {
                    return usage();
                };
                script = Some(Script::File(f.clone()));
                i += 2;
                break;
            }
            "-r" => {
                let Some(c) = args.get(i + 1) else {
                    return usage();
                };
                script = Some(Script::Code(c.clone()));
                i += 2;
                break;
            }
            _ if a.starts_with("-d") || a.starts_with("-c") => i += 1,
            _ if a.starts_with("-r") && a.len() > 2 => {
                script = Some(Script::Code(a[2..].to_string()));
                i += 1;
                break;
            }
            _ if a.starts_with('-') && a.len() > 1 => {
                eprintln!("rhp: unsupported option {a}");
                return usage();
            }
            _ => break,
        }
    }
    let script = match script {
        Some(s) => {
            // `--` may still separate the script's own arguments.
            if args.get(i).map(String::as_str) == Some("--") {
                i += 1;
            }
            s
        }
        None => match args.get(i) {
            Some(p) => {
                i += 1;
                Script::File(p.clone())
            }
            None => return usage(),
        },
    };

    // --- load the source ------------------------------------------------------
    let (file, source, argv0) = match &script {
        Script::File(path) => {
            let source = match std::fs::read(path) {
                Ok(bytes) => bytes,
                Err(e) => {
                    eprintln!("rhp: cannot read {path}: {e}");
                    return ExitCode::from(1);
                }
            };
            // php reports the resolved real path in __FILE__ / __DIR__ / getFile().
            let file = std::fs::canonicalize(path)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| path.clone());
            (file, source, path.clone())
        }
        Script::Code(code) => {
            let mut src = b"<?php ".to_vec();
            src.extend_from_slice(code.as_bytes());
            (
                "Command line code".to_string(),
                src,
                "Standard input code".to_string(),
            )
        }
    };

    let arena = Arena::new();
    let parsed = arena.parse(file.as_bytes(), &source);
    if parsed.has_parse_errors() {
        eprintln!("rhp: parse error in {file}");
        return ExitCode::from(255);
    }

    // Compile to owned bytecode; the arena can be dropped after this.
    let program = match compile_with_source(parsed.program(), &source, &file) {
        Ok(p) => p,
        Err(t) => return fatal(t),
    };

    let mut vm = Vm::new();
    // php's CLI writes output unbuffered, so it interleaves with stderr and
    // with child processes that share our stdout.
    vm.set_direct_stdout(true);
    // PHP_BINARY is this engine's own executable (composer re-invokes it for
    // `@php` scripts).
    if let Ok(exe) = std::env::current_exe() {
        let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
        vm.set_constant(
            "PHP_BINARY",
            Value::str(exe.to_string_lossy().into_owned().into_bytes()),
        );
    }
    if let Script::File(_) = script {
        // The top-level script's path, so `Phar::mapPhar` can parse the running phar.
        vm.set_main_script_path(file.as_bytes().to_vec());
    }
    // $argv / $_SERVER['argv']: the script name as given, then its arguments.
    let mut argv = vec![argv0.into_bytes()];
    argv.extend(args[i..].iter().map(|s| s.as_bytes().to_vec()));
    vm.set_argv(argv);
    // Teach the VM how to compile an included file (keeps the VM compiler-agnostic).
    vm.set_includer(|file: &str, src: &[u8]| {
        let arena = Arena::new();
        let parsed = arena.parse(file.as_bytes(), src);
        if parsed.has_parse_errors() {
            return Err(Thrown::error(
                "ParseError",
                format!("syntax error in {file}"),
            ));
        }
        compile_with_source(parsed.program(), src, file)
    });
    let result = vm.run(&program);

    let _ = std::io::stdout().write_all(vm.output());
    let _ = std::io::stdout().flush();

    match result {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(t) => fatal(t),
    }
}

fn fatal(t: Thrown) -> ExitCode {
    match t {
        Thrown::Exit(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Thrown::Error { class, message } => {
            eprintln!("PHP Fatal error:  Uncaught {class}: {message}");
            ExitCode::from(255)
        }
        Thrown::Object(_) => {
            // Uncaught user exception (details rendered to stderr only).
            eprintln!("PHP Fatal error:  Uncaught exception");
            ExitCode::from(255)
        }
        Thrown::Unsupported { feature, reason } => {
            eprintln!("RHP_UNSUPPORTED: {feature} is not supported ({reason})");
            ExitCode::from(255)
        }
    }
}
