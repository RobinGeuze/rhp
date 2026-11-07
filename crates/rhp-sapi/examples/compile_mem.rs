//! Memory of compiling one file, phase by phase: the parse arena, php's
//! grammar check, the compile pass on top, and the finished program.
//!
//! ```sh
//! cargo run --release -p rhp-sapi --example compile_mem -- file.php
//! ```

use std::time::Instant;

use rhp_compiler::parse::Arena;
use rhp_values::mem;

#[global_allocator]
static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;

fn mb(n: usize) -> String {
    format!("{:.1} MB", n as f64 / 1048576.0)
}

fn main() {
    let path = std::env::args().nth(1).expect("file");
    let src = std::fs::read(&path).unwrap();
    // Prelude and lazy tables outside the counted window.
    let _ = rhp_sapi::compile_file("warm.php", b"<?php echo 1;", rhp_compiler::Mode::Include);
    mem::begin_request();
    let m0 = mem::usage(false);
    let t = Instant::now();
    {
        let arena = Arena::new();
        let _parsed = arena.parse(path.as_bytes(), &src);
        let parse_t = t.elapsed();
        let a = mem::usage(false) - m0;
        println!("parse arena: {} ({:?})", mb(a), parse_t);
    }
    mem::reset_peak();
    let m1 = mem::usage(false);
    let t = Instant::now();
    let _ = rhp_builtins::zend::syntax_error_ex(&src, false);
    println!(
        "php grammar check: peak +{} ({:?})",
        mb(mem::peak_usage(false) - m1),
        t.elapsed()
    );
    mem::reset_peak();
    let m2 = mem::usage(false);
    let t = Instant::now();
    let p = rhp_sapi::compile_file(&path, &src, rhp_compiler::Mode::Include);
    let dt = t.elapsed();
    let program = mem::usage(false) - m2;
    println!(
        "compile_file: peak +{}, program {} ({:?})",
        mb(mem::peak_usage(false) - m2),
        mb(program),
        dt
    );
    if let Ok(p) = &p {
        breakdown(p);
    }
    drop(p);
    mem::end_request();
}

/// Bytes of the program's main per-proto vectors (capacity), by part.
fn breakdown(p: &rhp_bytecode::Program) {
    use rhp_bytecode::{FunctionProto, Instr, ParamInfo};
    use std::mem::size_of;
    let mut parts = [0usize; 9];
    let mut n = 0;
    let mut instrs = 0;
    let mut f = |fp: &FunctionProto| {
        n += 1;
        instrs += fp.code.len();
        parts[0] += size_of::<FunctionProto>();
        parts[1] += fp.code.capacity() * size_of::<Instr>();
        parts[2] += fp.consts.capacity() * 16;
        parts[3] += fp.names.capacity() * 8;
        parts[4] += fp.debug.lines.capacity() * 8;
        parts[5] += fp.params.capacity() * size_of::<ParamInfo>();
        parts[6] += fp.cv_var_names.capacity() * 8;
        parts[7] += fp.exception_table.capacity() * 16
            + fp.discard_calls.capacity() * 4
            + fp.new_ranges.capacity() * 12
            + fp.send_kinds.capacity() * 8
            + fp.superglobals.capacity() * 16;
    };
    f(&p.main);
    p.functions
        .iter()
        .chain(p.closures.iter())
        .for_each(|x| f(x));
    for c in p.classes.iter().chain(p.cond_classes.iter()) {
        for m in &c.methods {
            f(&m.proto);
        }
    }
    let names = [
        "proto structs",
        "code",
        "consts",
        "names",
        "lines",
        "params",
        "cv names",
        "other vecs",
    ];
    println!("{n} protos, {instrs} instructions");
    for (k, v) in names.iter().zip(parts) {
        println!("  {k:14} {}", mb(v));
    }
}
