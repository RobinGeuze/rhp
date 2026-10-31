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
use rhp_vm::Vm;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: rhp <script.php>");
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
    // The top-level script's path, so `Phar::mapPhar` can parse the running phar.
    vm.set_main_script_path(file.as_bytes().to_vec());
    // $argv / $_SERVER['argv']: the script name as given, then its arguments.
    vm.set_argv(args[1..].iter().map(|s| s.as_bytes().to_vec()).collect());
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
