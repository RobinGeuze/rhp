//! rhp CLI SAPI: `rhp script.php`.
//!
//! Reads a file, parses it, compiles it to bytecode, runs it on the register
//! VM, flushes output to stdout, and exits with the script's exit code. Parse
//! errors, compile-time refusals, and uncaught throws go to stderr with a
//! nonzero exit.

use std::io::Write;
use std::process::ExitCode;

use rhp_compiler::compile_with_source;
use rhp_frontend::Arena;
use rhp_values::Thrown;
use rhp_vm::{IncludeError, Vm};

fn main() -> ExitCode {
    let all: Vec<String> = std::env::args().collect();
    // php-style options before the script: `-n` (no php.ini: always the case
    // here) and `-d key=value` / `-dkey=value` ini settings.
    let mut ini: Vec<(String, String)> = Vec::new();
    let mut i = 1;
    while i < all.len() {
        let a = &all[i];
        let setting = if a == "-n" {
            i += 1;
            continue;
        } else if a == "-d" {
            i += 1;
            all.get(i).cloned()
        } else if let Some(kv) = a.strip_prefix("-d") {
            Some(kv.to_string())
        } else {
            break;
        };
        if let Some(kv) = setting {
            let (k, v) = kv.split_once('=').unwrap_or((&kv, "1"));
            ini.push((k.trim().to_string(), v.trim().to_string()));
        }
        i += 1;
    }
    let args = &all[i.saturating_sub(1)..];
    let Some(path) = args.get(1) else {
        eprintln!("usage: rhp [-n] [-d key=value]... <script.php>");
        return ExitCode::from(1);
    };

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

    let arena = Arena::new();
    let parsed = arena.parse(file.as_bytes(), &source);
    if let Some((message, line)) = parsed.parse_error() {
        let mut vm = Vm::new();
        for (k, v) in &ini {
            vm.set_ini(k.as_bytes(), v.as_bytes());
        }
        vm.set_direct_stdout(true);
        vm.report_parse_error(&message, &file, line);
        vm.flush_stdout();
        let _ = std::io::stdout().flush();
        return ExitCode::from(255);
    }

    // Compile to owned bytecode; the arena can be dropped after this.
    let program = match compile_with_source(parsed.program(), &source, &file) {
        Ok(p) => p,
        Err(t) => return fatal(t),
    };

    let mut vm = Vm::new();
    // Like the php CLI, write output unbuffered so it interleaves with stderr
    // and with the output of child processes in the order it was produced.
    vm.set_stdout_passthrough(true);
    for (k, v) in &ini {
        vm.set_ini(k.as_bytes(), v.as_bytes());
    }
    // The top-level script's path, so `Phar::mapPhar` can parse the running phar.
    vm.set_main_script_path(file.as_bytes().to_vec());
    // $argv / $_SERVER['argv']: the script name as given, then its arguments.
    vm.set_argv(args[1..].iter().map(|s| s.as_bytes().to_vec()).collect());
    // Teach the VM how to compile an included file (keeps the VM compiler-agnostic).
    vm.set_includer(|file: &str, src: &[u8]| {
        let arena = Arena::new();
        let parsed = arena.parse(file.as_bytes(), src);
        if let Some((message, line)) = parsed.parse_error() {
            return Err(IncludeError::Parse { message, line });
        }
        Ok(compile_with_source(parsed.program(), src, file)?)
    });
    // Unbuffered stdout reaches the process before each stderr write.
    vm.set_direct_stdout(true);
    // Run, report an uncaught throwable (per display_errors / log_errors /
    // error_reporting), call the shutdown functions, flush output buffers.
    let result = vm.run_main(&program);
    vm.flush_stdout();
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
