//! phrust CLI SAPI: `phrust script.php`.
//!
//! Reads a file, parses it, compiles it to bytecode, runs it on the register
//! VM, flushes output to stdout, and exits with the script's exit code. Parse
//! errors, compile-time refusals, and uncaught throws go to stderr with a
//! nonzero exit.

use std::io::Write;
use std::process::ExitCode;

use phrust_compiler::compile;
use phrust_frontend::Arena;
use phrust_values::Thrown;
use phrust_vm::Vm;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: phrust <script.php>");
        return ExitCode::from(1);
    };

    let source = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("phrust: cannot read {path}: {e}");
            return ExitCode::from(1);
        }
    };

    let arena = Arena::new();
    let parsed = arena.parse(path.as_bytes(), &source);
    if parsed.has_parse_errors() {
        eprintln!("phrust: parse error in {path}");
        return ExitCode::from(255);
    }

    // Compile to owned bytecode; the arena can be dropped after this.
    let program = match compile(parsed.program()) {
        Ok(p) => p,
        Err(t) => return fatal(t),
    };

    let mut vm = Vm::new();
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
            eprintln!("PHRUST_UNSUPPORTED: {feature} is not supported ({reason})");
            ExitCode::from(255)
        }
    }
}
