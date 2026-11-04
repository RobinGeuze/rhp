//! The compiled prelude is cached per thread: every program compiled with it
//! gets the same prelude, whatever the user file contains (or whether the
//! previous compile failed), and include programs carry none of it.

use std::rc::Rc;

use rhp_bytecode::Program;
use rhp_compiler::parse::Arena;
use rhp_compiler::{compile_include, compile_with_source};

fn main_prog(src: &str) -> Program {
    let arena = Arena::new();
    let parsed = arena.parse(b"t.php", src.as_bytes());
    compile_with_source(parsed.program(), src.as_bytes(), "t.php").expect("compile")
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
    let arena = Arena::new();
    let bad = "<?php\n$x = `ls`;\n";
    let parsed = arena.parse(b"bad.php", bad.as_bytes());
    assert!(compile_with_source(parsed.program(), bad.as_bytes(), "bad.php").is_err());
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
    let arena = Arena::new();
    let parsed = arena.parse(b"inc.php", src.as_bytes());
    let p = compile_include(parsed.program(), src.as_bytes(), "inc.php").expect("compile");
    assert_eq!(p.num_prelude_classes, 0);
    assert_eq!(p.num_prelude_functions, 0);
    assert_eq!(p.classes.len(), 1);
    assert_eq!(p.functions.len(), 1);
    assert_eq!(p.closures.len(), 1);
}
