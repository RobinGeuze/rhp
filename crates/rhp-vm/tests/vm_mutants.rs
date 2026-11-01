//! Unit suite part 3: targeted tests that pin down behavior of stable VM
//! helper functions which mutation testing (cargo-mutants) found under-covered.
//! Each test names the helper it guards. Mutations in `run_frames` / opcode
//! dispatch and the frame machinery are deliberately NOT chased here because
//! that code is slated for rewrite (generators + diagnostics).

mod common;
use common::{run, run_throws};

// --- is_callable_value (lib.rs ~102-129): every callable shape --------------

#[test]
fn is_callable_covers_all_shapes() {
    let prelude = "<?php function sq($x){return $x;} class C{public function m(){} public function __invoke(){}} $o=new C;";
    assert_eq!(
        run(&format!("{prelude} var_dump(is_callable('sq'));")),
        "bool(true)\n"
    );
    assert_eq!(
        run(&format!("{prelude} var_dump(is_callable('strlen'));")),
        "bool(true)\n"
    ); // builtin
    assert_eq!(
        run(&format!("{prelude} var_dump(is_callable('nope'));")),
        "bool(false)\n"
    );
    assert_eq!(
        run(&format!("{prelude} var_dump(is_callable(fn()=>1));")),
        "bool(true)\n"
    );
    assert_eq!(
        run(&format!("{prelude} var_dump(is_callable([$o,'m']));")),
        "bool(true)\n"
    ); // [obj, method]
    assert_eq!(
        run(&format!("{prelude} var_dump(is_callable([$o,'nope']));")),
        "bool(false)\n"
    );
    assert_eq!(
        run(&format!("{prelude} var_dump(is_callable($o));")),
        "bool(true)\n"
    ); // __invoke
    assert_eq!(
        run(&format!("{prelude} var_dump(is_callable(42));")),
        "bool(false)\n"
    );
    // a 1-element array is not a callable (guards the `a.len() == 2` match guard)
    assert_eq!(
        run(&format!("{prelude} var_dump(is_callable([$o]));")),
        "bool(false)\n"
    );
    // a 3-element array is not a callable either (guards `== 2` against `true`)
    assert_eq!(
        run(&format!("{prelude} var_dump(is_callable([$o,'m',1]));")),
        "bool(false)\n"
    );
    // ['ClassName', 'staticMethod'] is callable (guards the Str-classname arm)
    assert_eq!(
        run("<?php class C{public static function s(){return 's';}} var_dump(is_callable(['C','s']));"),
        "bool(true)\n"
    );
}

// --- call_callable_value (lib.rs ~1472-1484): string + array callables ------

#[test]
fn call_user_func_string_callable() {
    assert_eq!(
        run("<?php function sq($x){return $x*$x;} echo call_user_func('sq', 6);"),
        "36"
    );
}

#[test]
fn call_array_callable_object_method() {
    assert_eq!(run("<?php class C{public function m($x){return $x+1;}} $o=new C; $cb=[$o,'m']; echo $cb(3);"), "4");
    assert_eq!(run("<?php class C{public function m($x){return $x+1;}} $o=new C; echo call_user_func([$o,'m'], 10);"), "11");
}

#[test]
fn call_array_callable_static_method_by_string() {
    // Guards the Str-classname arm of call_callable_value.
    assert_eq!(run("<?php class C{public static function s($x){return $x*2;}} echo call_user_func(['C','s'], 21);"), "42");
}

#[test]
fn calling_three_element_array_is_not_callable() {
    // Guards the `a.len() == 2` guard in resolve_callable against `true`:
    // php raises "Error: Array callback must have exactly two elements".
    let (_out, class) = run_throws(
        "<?php class C{public function m(){return 1;}} $o=new C; $cb=[$o,'m',1]; $cb();",
    );
    assert_eq!(class, "Error");
}

// --- define_const / get_const (lib.rs ~85-95) -------------------------------

#[test]
fn define_returns_true_then_false_on_redefine() {
    // First define succeeds; redefining the same name returns false (not an error).
    assert_eq!(
        run("<?php var_dump(define('X', 1)); var_dump(define('X', 2)); echo X;"),
        "bool(true)\nbool(false)\n1"
    );
}

#[test]
fn constant_lookup_via_constant_and_bareword() {
    assert_eq!(run("<?php const PI=3; define('E', 2); echo constant('PI'), ' ', constant('E'), ' ', PI, ' ', E;"), "3 2 3 2");
}

// --- enum_builtin_static string backing (lib.rs ~1414) ----------------------

#[test]
fn string_backed_enum_from_and_tryfrom() {
    assert_eq!(
        run("<?php enum S:string{case A='a'; case B='b';} echo S::from('b')->name;"),
        "B"
    );
    assert_eq!(
        run("<?php enum S:string{case A='a'; case B='b';} var_dump(S::tryFrom('a'));"),
        "enum(S::A)\n"
    );
    assert_eq!(
        run("<?php enum S:string{case A='a'; case B='b';} var_dump(S::tryFrom('z'));"),
        "NULL\n"
    );
}

// --- is_enum_class (lib.rs ~82): a plain object dumps as object, not enum ---

#[test]
fn plain_object_var_dump_shape() {
    assert_eq!(
        run("<?php class C{public $x=1; public $y='hi';} var_dump(new C());"),
        "object(C)#1 (2) {\n  [\"x\"]=>\n  int(1)\n  [\"y\"]=>\n  string(2) \"hi\"\n}\n"
    );
}

// --- class_parent_name (lib.rs ~76-79): inheritance chain -------------------

#[test]
fn inherited_instanceof_and_get_parent_class() {
    assert_eq!(
        run("<?php class A{} class B extends A{} var_dump((new B()) instanceof A);"),
        "bool(true)\n"
    );
    assert_eq!(
        run("<?php class A{} class B extends A{} echo get_parent_class(new B());"),
        "A"
    );
    assert_eq!(
        run("<?php class A{} var_dump(get_parent_class(new A()));"),
        "bool(false)\n"
    );
}

// --- type_lc (lib.rs ~1878): used in the member-call-on-non-object error ----

#[test]
fn member_call_on_scalar_names_the_type() {
    assert_eq!(
        run("<?php $x=5; try{$x->foo();}catch(\\Error $e){echo $e->getMessage();}"),
        "Call to a member function foo() on int"
    );
    assert_eq!(
        run("<?php $x=null; try{$x->foo();}catch(\\Error $e){echo $e->getMessage();}"),
        "Call to a member function foo() on null"
    );
}

// --- dim_fetch (lib.rs ~1727-1737): negative + out-of-bounds string index ---

#[test]
fn string_negative_index_reads_from_end() {
    assert_eq!(run("<?php $s='abc'; echo $s[-1], $s[-3];"), "ca");
}

#[test]
fn string_out_of_bounds_index_is_empty_string() {
    // Guards the `i >= 0 && i < len` bounds check: a `<=` or `||` mutation would
    // index past the end and panic instead of returning "".
    // (php warns "Uninitialized string offset"; `@` keeps the output to the value.)
    assert_eq!(run("<?php $s='abc'; var_dump(@$s[3]);"), "string(0) \"\"\n");
    assert_eq!(
        run("<?php $s='abc'; var_dump(@$s[-5]);"),
        "string(0) \"\"\n"
    );
}

// --- autovivify (lib.rs ~1764-1776): empty string vivifies, scalar errors ---

#[test]
fn append_to_empty_string_autovivifies_array() {
    assert_eq!(
        run("<?php $x=''; $x[]=5; print_r($x);"),
        "Array\n(\n    [0] => 5\n)\n"
    );
}

#[test]
fn append_to_nonempty_string_is_type_error() {
    let (_out, class) = run_throws("<?php $s='hi'; $s[]='x';");
    assert_eq!(class, "TypeError");
}

// --- bind_params / required_params (lib.rs ~1901-1939): arg count error ------

#[test]
fn too_few_arguments_reports_required_count() {
    // f has two required params (a, b) and one optional (c): required_params == 2.
    assert_eq!(
        run("<?php function f($a,$b,$c=1){return $a;} try{f(1);}catch(\\ArgumentCountError $e){echo $e->getMessage();}"),
        "Too few arguments to function f(), 1 passed and at least 2 expected"
    );
}

#[test]
fn argument_count_error_class() {
    let (_out, class) = run_throws("<?php function f($a,$b){} f(1);");
    assert_eq!(class, "ArgumentCountError");
}

// --- eval_cmp NotIdentical (lib.rs ~1946): the `!==` operator ----------------

#[test]
fn not_identical_operator() {
    assert_eq!(run("<?php var_dump(1 !== 1);"), "bool(false)\n");
    assert_eq!(run("<?php var_dump(1 !== 2);"), "bool(true)\n");
    assert_eq!(run("<?php var_dump(1 !== '1');"), "bool(true)\n"); // different types
    assert_eq!(run("<?php var_dump('a' !== 'a');"), "bool(false)\n");
}
