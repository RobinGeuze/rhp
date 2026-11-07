//! The compiled prelude is cached per thread and per set of builtins: every
//! program compiled with it gets the same prelude, whatever the user file
//! contains (or whether the previous compile failed), and include programs
//! carry none of it.

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use rhp_bytecode::Program;
use rhp_compiler::parse::Arena;
use rhp_compiler::Builtins;
use rhp_sapi::{compile_file, Mode};
use rhp_values::Value;

fn main_prog(src: &str) -> Program {
    compile_file("t.php", src.as_bytes(), Mode::Main).unwrap_or_else(|_| panic!("compile"))
}

fn names(p: &Program, n: usize) -> Vec<String> {
    p.classes[..n]
        .iter()
        .map(|c| String::from_utf8_lossy(c.name.as_bytes()).into_owned())
        .collect()
}

#[test]
fn prelude_is_identical_across_compiles() {
    // A user file whose strict_types, namespace and by-ref method signatures
    // could have leaked into a prelude compiled in its context.
    let a = main_prog(
        "<?php declare(strict_types=1);\nnamespace N;\n\
         class X { function current(&$a) {} function offsetGet(&$k) {} }\n\
         $f = function() { return 1; };\n",
    );
    // A failed compile in between must not disturb the cache.
    let bad = "<?php\ntry {} catch (int $e) {}\n";
    assert!(compile_file("bad.php", bad.as_bytes(), Mode::Main).is_err());
    let b = main_prog("<?php\n$g = fn() => 2;\n");

    assert!(a.num_prelude_classes > 0);
    assert_eq!(a.num_prelude_classes, b.num_prelude_classes);
    assert_eq!(a.num_prelude_functions, b.num_prelude_functions);
    assert_eq!(
        names(&a, a.num_prelude_classes),
        names(&b, b.num_prelude_classes)
    );
    for (ca, cb) in a.classes.iter().zip(&b.classes).take(a.num_prelude_classes) {
        assert_eq!(ca.methods.len(), cb.methods.len());
        for (ma, mb) in ca.methods.iter().zip(&cb.methods) {
            assert!(Rc::ptr_eq(&ma.proto, &mb.proto), "method protos are shared");
        }
    }
    for (fa, fb) in a
        .functions
        .iter()
        .zip(&b.functions)
        .take(a.num_prelude_functions)
    {
        assert!(Rc::ptr_eq(fa, fb));
    }
    // The prelude's closures come first, and user closures follow them.
    let pre = b.closures.len() - 1;
    assert_eq!(a.closures.len(), pre + 1);
    for (x, y) in a.closures.iter().zip(&b.closures).take(pre) {
        assert!(Rc::ptr_eq(x, y));
    }
    assert!(!Rc::ptr_eq(&a.closures[pre], &b.closures[pre]));
}

#[test]
fn include_programs_carry_no_prelude() {
    let src = "<?php\nclass Y extends Exception {}\nfunction h() {}\n$c = function() {};\n";
    let p = compile_file("inc.php", src.as_bytes(), Mode::Include)
        .unwrap_or_else(|_| panic!("compile"));
    assert_eq!(p.num_prelude_classes, 0);
    assert_eq!(p.num_prelude_functions, 0);
    assert_eq!(p.classes.len(), 1);
    assert_eq!(p.functions.len(), 1);
    assert_eq!(p.closures.len(), 1);
}

#[test]
fn prelude_is_compiled_per_source() {
    static A: Builtins = Builtins {
        prelude: || "<?php\nclass A {}\n",
        native_function: |_| false,
        constant: |_| None,
        cwd: || None,
        assertions: || 1,
    };
    static B: Builtins = Builtins {
        prelude: || "<?php\nclass B {}\nclass C {}\n",
        native_function: |_| false,
        constant: |_| None,
        cwd: || None,
        assertions: || 1,
    };
    let compile = |b: &Builtins| {
        let src = b"<?php\nclass D {}\n";
        let arena = Arena::new();
        let parsed = arena.parse(b"t.php", src);
        rhp_compiler::compile(b, parsed.program(), src, "t.php", Mode::Main).expect("compile")
    };
    let all = |p: &Program| names(p, p.classes.len());
    assert_eq!(all(&compile(&A)), ["A", "D"]);
    assert_eq!(all(&compile(&B)), ["B", "C", "D"]);
    let a = compile(&A);
    assert_eq!(all(&a), ["A", "D"]);
    assert_eq!(a.num_prelude_classes, 1);
}

#[test]
fn prelude_is_compiled_per_builtins() {
    // The same prelude source with another `constant` (B) or
    // `native_function` (C) than A's: the prelude compiles again and asks
    // them (for in_array's strict flag, which is folded, and for strlen's
    // stub) rather than reuse what it compiled with A's.
    static ASKED_B: AtomicBool = AtomicBool::new(false);
    static ASKED_C: AtomicBool = AtomicBool::new(false);
    fn prelude() -> &'static str {
        "<?php\nfunction p($a) { return in_array($a, ['x'], FOO) || strlen($a); }\n"
    }
    fn no_function(_: &str) -> bool {
        false
    }
    fn no_constant(_: &[u8]) -> Option<Value> {
        None
    }
    static A: Builtins = Builtins {
        prelude,
        native_function: no_function,
        constant: no_constant,
        cwd: || None,
        assertions: || 1,
    };
    static B: Builtins = Builtins {
        prelude,
        native_function: no_function,
        constant: |name| {
            ASKED_B.fetch_or(name == b"FOO", Ordering::Relaxed);
            None
        },
        cwd: || None,
        assertions: || 1,
    };
    static C: Builtins = Builtins {
        prelude,
        native_function: |name| {
            ASKED_C.fetch_or(name == "strlen", Ordering::Relaxed);
            false
        },
        constant: no_constant,
        cwd: || None,
        assertions: || 1,
    };
    let compile = |b: &Builtins| {
        let src = b"<?php\n";
        let arena = Arena::new();
        let parsed = arena.parse(b"t.php", src);
        rhp_compiler::compile(b, parsed.program(), src, "t.php", Mode::Main).expect("compile")
    };
    compile(&A);
    compile(&B);
    assert!(ASKED_B.load(Ordering::Relaxed), "a new constant");
    compile(&A);
    compile(&C);
    assert!(ASKED_C.load(Ordering::Relaxed), "a new native_function");
}
