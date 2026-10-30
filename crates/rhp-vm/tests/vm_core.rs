//! Unit suite part 1: scalars, arithmetic, comparisons, strings, arrays,
//! control flow, references, and variables. Exact-output assertions.
//!
//! Expected strings are php 8.5 semantics; see `common/mod.rs`.

mod common;
use common::{run, run_throws};

// ---------------------------------------------------------------------------
// Arithmetic: int/float, overflow promotion, division, modulo, power
// ---------------------------------------------------------------------------

#[test]
fn int_arithmetic() {
    assert_eq!(run("<?php echo 2 + 3 * 4 - 1;"), "13");
    assert_eq!(run("<?php echo (2 + 3) * 4;"), "20");
    assert_eq!(run("<?php echo 7 - 10;"), "-3");
    assert_eq!(run("<?php echo 2 * -3;"), "-6");
}

#[test]
fn integer_division_truncates_toward_zero() {
    assert_eq!(run("<?php echo intdiv(7, 2);"), "3");
    assert_eq!(run("<?php echo intdiv(-7, 2);"), "-3");
    assert_eq!(run("<?php echo intdiv(7, -2);"), "-3");
    assert_eq!(run("<?php echo intdiv(-7, -2);"), "3");
}

#[test]
fn modulo_sign_follows_dividend() {
    assert_eq!(run("<?php echo -7 % 3;"), "-1");
    assert_eq!(run("<?php echo 7 % -3;"), "1");
    assert_eq!(
        run("<?php echo -10 % 3, ' ', 10 % -3, ' ', -10 % -3;"),
        "-1 1 -1"
    );
    assert_eq!(run("<?php echo 10 % 3;"), "1");
}

#[test]
fn fmod_is_float_remainder() {
    assert_eq!(run("<?php echo fmod(7.5, 2);"), "1.5");
}

#[test]
fn division_promotes_to_float_only_when_not_exact() {
    assert_eq!(
        run("<?php var_dump(10 / 3);"),
        "float(3.3333333333333335)\n"
    );
    assert_eq!(run("<?php var_dump(10 / 2);"), "int(5)\n");
    assert_eq!(run("<?php var_dump(6 / 3);"), "int(2)\n");
    assert_eq!(run("<?php var_dump(10 / 4);"), "float(2.5)\n");
}

#[test]
fn int_overflow_promotes_to_float() {
    assert_eq!(
        run("<?php var_dump(PHP_INT_MAX);"),
        "int(9223372036854775807)\n"
    );
    assert_eq!(
        run("<?php var_dump(PHP_INT_MAX + 1);"),
        "float(9.223372036854776E+18)\n"
    );
    assert_eq!(
        run("<?php var_dump(9223372036854775807 + 1);"),
        "float(9.223372036854776E+18)\n"
    );
}

#[test]
fn mixed_int_float_arithmetic_types() {
    assert_eq!(run("<?php var_dump(5 + 2.0);"), "float(7)\n");
    assert_eq!(run("<?php var_dump(5 * 2);"), "int(10)\n");
    assert_eq!(run("<?php var_dump(2.5 + 2.5);"), "float(5)\n");
}

#[test]
fn power_operator() {
    assert_eq!(run("<?php echo 2 ** 10;"), "1024");
    assert_eq!(
        run("<?php var_dump(2 ** 0.5);"),
        "float(1.4142135623730951)\n"
    );
    assert_eq!(run("<?php var_dump(2 ** -1);"), "float(0.5)\n");
    assert_eq!(run("<?php echo 2 ** 3 ** 2;"), "512"); // right-associative
}

#[test]
fn bitwise_operators() {
    assert_eq!(run("<?php echo 6 & 3;"), "2");
    assert_eq!(run("<?php echo 6 | 1;"), "7");
    assert_eq!(run("<?php echo 6 ^ 3;"), "5");
    assert_eq!(run("<?php echo ~5;"), "-6");
    assert_eq!(run("<?php echo 1 << 4;"), "16");
    assert_eq!(run("<?php echo 256 >> 2;"), "64");
}

#[test]
fn increment_decrement() {
    assert_eq!(run("<?php $x = 5; echo $x++, ' ', $x;"), "5 6");
    assert_eq!(run("<?php $x = 5; echo ++$x, ' ', $x;"), "6 6");
    assert_eq!(run("<?php $x = 5; echo $x--, ' ', $x;"), "5 4");
    assert_eq!(run("<?php $x = 5; echo --$x, ' ', $x;"), "4 4");
    // numeric string increments as a number
    assert_eq!(run("<?php $n = '9'; $n++; echo $n;"), "10");
}

#[test]
fn compound_assignment() {
    assert_eq!(
        run("<?php $x = 1; $x += 2; $x *= 3; $x -= 1; echo $x;"),
        "8"
    );
    assert_eq!(run("<?php $x = 10; $x /= 4; echo $x;"), "2.5");
    assert_eq!(run("<?php $x = 10; $x %= 3; echo $x;"), "1");
    assert_eq!(run("<?php $x = 2; $x **= 3; echo $x;"), "8");
    assert_eq!(run("<?php $x = 1; $x <<= 3; echo $x;"), "8");
}

#[test]
fn chained_assignment_is_right_associative() {
    assert_eq!(run("<?php $a = $b = $c = 5; echo \"$a$b$c\";"), "555");
}

// ---------------------------------------------------------------------------
// Float formatting on echo / var_dump
// ---------------------------------------------------------------------------

#[test]
fn float_echo_uses_precision_14() {
    assert_eq!(run("<?php echo 1/3;"), "0.33333333333333");
    assert_eq!(run("<?php echo 100.0;"), "100");
    assert_eq!(run("<?php echo 0.5;"), "0.5");
    assert_eq!(run("<?php echo 1e10;"), "10000000000");
    assert_eq!(run("<?php echo 3.14;"), "3.14");
}

#[test]
fn float_var_dump_uses_full_precision() {
    assert_eq!(
        run("<?php var_dump(0.1 + 0.2);"),
        "float(0.30000000000000004)\n"
    );
    assert_eq!(run("<?php var_dump(1.0);"), "float(1)\n");
    assert_eq!(run("<?php echo 0.1 + 0.2;"), "0.3");
}

// ---------------------------------------------------------------------------
// Comparisons and type juggling
// ---------------------------------------------------------------------------

#[test]
fn comparison_operators_at_boundaries() {
    assert_eq!(run("<?php var_dump(5 > 5);"), "bool(false)\n");
    assert_eq!(run("<?php var_dump(5 >= 5);"), "bool(true)\n");
    assert_eq!(run("<?php var_dump(5 < 5);"), "bool(false)\n");
    assert_eq!(run("<?php var_dump(5 <= 5);"), "bool(true)\n");
    assert_eq!(run("<?php var_dump(5 == 5);"), "bool(true)\n");
    assert_eq!(run("<?php var_dump(5 != 5);"), "bool(false)\n");
    assert_eq!(run("<?php var_dump(5 <> 5);"), "bool(false)\n");
    assert_eq!(run("<?php var_dump(4 < 5);"), "bool(true)\n");
    assert_eq!(run("<?php var_dump(6 > 5);"), "bool(true)\n");
}

#[test]
fn spaceship_operator() {
    assert_eq!(run("<?php echo 1 <=> 2, 2 <=> 2, 3 <=> 2;"), "-101");
    assert_eq!(run("<?php var_dump([1,2] <=> [1,3]);"), "int(-1)\n");
    assert_eq!(
        run("<?php echo ('a' <=> 'b'), ('b' <=> 'a'), ('a' <=> 'a');"),
        "-110"
    );
}

#[test]
fn strict_vs_loose_equality() {
    assert_eq!(run("<?php var_dump(1 === 1.0);"), "bool(false)\n");
    assert_eq!(run("<?php var_dump(1 == 1.0);"), "bool(true)\n");
    assert_eq!(run("<?php var_dump('1' === 1);"), "bool(false)\n");
    assert_eq!(run("<?php var_dump('1' == 1);"), "bool(true)\n");
}

#[test]
fn php8_numeric_string_juggling() {
    // PHP 8: non-numeric string vs int compares as string, so these are false.
    assert_eq!(run("<?php var_dump(0 == 'a');"), "bool(false)\n");
    assert_eq!(run("<?php var_dump('abc' == 0);"), "bool(false)\n");
    assert_eq!(run("<?php var_dump('10abc' == 10);"), "bool(false)\n");
    // leading-numeric comparisons that ARE numeric strings
    assert_eq!(run("<?php var_dump('1' == '01');"), "bool(true)\n");
    assert_eq!(run("<?php var_dump('10' == '1e1');"), "bool(true)\n");
    assert_eq!(run("<?php var_dump(100 == '1e2');"), "bool(true)\n");
}

#[test]
fn null_juggling() {
    assert_eq!(run("<?php var_dump(null == false);"), "bool(true)\n");
    assert_eq!(run("<?php var_dump(null == 0);"), "bool(true)\n");
    assert_eq!(run("<?php var_dump(null == '');"), "bool(true)\n");
    assert_eq!(run("<?php var_dump(null === null);"), "bool(true)\n");
    assert_eq!(run("<?php var_dump(null == 'x');"), "bool(false)\n");
}

#[test]
fn numeric_string_arithmetic() {
    assert_eq!(run("<?php var_dump('10' + 5);"), "int(15)\n");
    assert_eq!(run("<?php var_dump('1e2' + 0);"), "float(100)\n");
    assert_eq!(run("<?php var_dump('3.5' + '1.5');"), "float(5)\n");
}

#[test]
fn string_comparison_lexicographic() {
    assert_eq!(run("<?php var_dump('a' < 'b');"), "bool(true)\n");
    assert_eq!(run("<?php var_dump('abc' < 'abd');"), "bool(true)\n");
    assert_eq!(run("<?php var_dump('Z' < 'a');"), "bool(true)\n");
    assert_eq!(
        run("<?php echo strcmp('a','b'), ' ', strcmp('b','a'), ' ', strcmp('a','a');"),
        "-1 1 0"
    );
}

// ---------------------------------------------------------------------------
// Casts and type predicates
// ---------------------------------------------------------------------------

#[test]
fn int_casts() {
    assert_eq!(run("<?php var_dump((int)3.9);"), "int(3)\n");
    assert_eq!(run("<?php var_dump((int)'12abc');"), "int(12)\n");
    assert_eq!(run("<?php var_dump((int)true);"), "int(1)\n");
    assert_eq!(run("<?php var_dump((int)'abc');"), "int(0)\n");
}

#[test]
fn float_casts() {
    assert_eq!(run("<?php var_dump((float)'3.14xyz');"), "float(3.14)\n");
    assert_eq!(run("<?php var_dump((float)5);"), "float(5)\n");
}

#[test]
fn bool_casts_falsy_and_truthy() {
    assert_eq!(run("<?php var_dump((bool)0);"), "bool(false)\n");
    assert_eq!(run("<?php var_dump((bool)'');"), "bool(false)\n");
    assert_eq!(run("<?php var_dump((bool)'0');"), "bool(false)\n");
    assert_eq!(run("<?php var_dump((bool)'0.0');"), "bool(true)\n"); // only '0' is falsy
    assert_eq!(run("<?php var_dump((bool)[]);"), "bool(false)\n");
    assert_eq!(run("<?php var_dump((bool)[0]);"), "bool(true)\n");
}

#[test]
fn gettype_names() {
    assert_eq!(
        run("<?php echo gettype(1), ' ', gettype(1.5), ' ', gettype('s'), ' ', gettype(true), ' ', gettype(null), ' ', gettype([]);"),
        "integer double string boolean NULL array"
    );
}

// ---------------------------------------------------------------------------
// Strings: concat, interpolation, builtins
// ---------------------------------------------------------------------------

#[test]
fn concatenation_coerces_operands() {
    assert_eq!(run("<?php echo 1 . 2 . 3;"), "123");
    assert_eq!(run("<?php echo true . 'x' . false . 'y' . null;"), "1xy");
    assert_eq!(run("<?php $s = 'x'; $s .= 'y'; $s .= 'z'; echo $s;"), "xyz");
}

#[test]
fn double_quote_interpolation() {
    assert_eq!(
        run("<?php $n = 'World'; echo \"Hello $n!\";"),
        "Hello World!"
    );
    assert_eq!(run("<?php $a = [1,2]; echo \"x={$a[1]}\";"), "x=2");
    assert_eq!(run("<?php $a = [1,2]; echo \"y=$a[0]\";"), "y=1");
}

#[test]
fn single_quotes_are_literal() {
    assert_eq!(run("<?php $n = 'World'; echo 'Hello $n';"), "Hello $n");
}

#[test]
fn heredoc_interpolates_nowdoc_does_not() {
    assert_eq!(
        run("<?php $n='World'; echo <<<EOT\nHello $n\nLine2\nEOT;\n"),
        "Hello World\nLine2"
    );
    assert_eq!(
        run("<?php $n='World'; echo <<<'EOT'\nHello $n\nEOT;\n"),
        "Hello $n"
    );
}

#[test]
fn string_builtins() {
    assert_eq!(
        run("<?php echo strtoupper('abc'), ' ', strtolower('ABC'), ' ', ucfirst('hi');"),
        "ABC abc Hi"
    );
    assert_eq!(run("<?php echo substr('hello', 1, 3);"), "ell");
    assert_eq!(run("<?php echo strpos('hello', 'l');"), "2");
    assert_eq!(run("<?php echo str_replace('l', 'L', 'hello');"), "heLLo");
    assert_eq!(
        run("<?php echo trim('  x  '), '|', str_repeat('ab', 3);"),
        "x|ababab"
    );
    assert_eq!(
        run("<?php echo strrev('abc'), ' ', ucwords('hi there');"),
        "cba Hi There"
    );
    assert_eq!(run("<?php echo strlen('hello');"), "5");
}

#[test]
fn string_search_predicates() {
    assert_eq!(
        run("<?php var_dump(str_contains('hello','ell'));"),
        "bool(true)\n"
    );
    assert_eq!(
        run("<?php var_dump(str_starts_with('hello','he'));"),
        "bool(true)\n"
    );
    assert_eq!(
        run("<?php var_dump(str_ends_with('hello','lo'));"),
        "bool(true)\n"
    );
    assert_eq!(
        run("<?php var_dump(str_contains('hello','zzz'));"),
        "bool(false)\n"
    );
}

#[test]
fn str_pad_variants() {
    assert_eq!(run("<?php echo str_pad('5', 3, '0', STR_PAD_LEFT);"), "005");
    assert_eq!(run("<?php echo str_pad('5', 3);"), "5  ");
}

#[test]
fn string_index_read() {
    assert_eq!(run("<?php $s = 'hello'; echo $s[0], $s[1], $s[4];"), "heo");
}

#[test]
fn printf_and_sprintf_format_specs() {
    assert_eq!(
        run("<?php printf('%d %05.2f %s %x', 42, 3.14159, 'hi', 255);"),
        "42 03.14 hi ff"
    );
    assert_eq!(
        run("<?php echo sprintf(\"[%'*10s]\", 'r');"),
        "[*********r]"
    );
    assert_eq!(run("<?php echo sprintf('%b %o', 5, 8);"), "101 10");
}

// ---------------------------------------------------------------------------
// Arrays: literals, read, write, COW, builtins
// ---------------------------------------------------------------------------

#[test]
fn array_read_nested_and_assoc() {
    assert_eq!(
        run("<?php $a = [10,20,30]; echo $a[0], $a[1], $a[2];"),
        "102030"
    );
    assert_eq!(
        run("<?php $m = ['x'=>1,'y'=>2]; echo $m['x'], $m['y'];"),
        "12"
    );
    assert_eq!(
        run("<?php $n = [[1,2],[3,4]]; echo $n[0][1], $n[1][0];"),
        "23"
    );
}

#[test]
fn array_missing_index_coalesces() {
    assert_eq!(run("<?php $a = [10]; echo $a[99] ?? 'missing';"), "missing");
}

#[test]
fn array_copy_on_write_is_value_semantics() {
    assert_eq!(
        run("<?php $a=[1,2,3]; $b=$a; $b[0]=99; echo $a[0], ' ', $b[0];"),
        "1 99"
    );
    assert_eq!(
        run("<?php $a=[[1,2],[3,4]]; $b=$a; $b[0][0]=99; echo $a[0][0], ' ', $b[0][0];"),
        "1 99"
    );
}

#[test]
fn array_passed_by_value_to_function() {
    assert_eq!(
        run("<?php function m($x){$x[0]=7; return $x;} $c=[1]; $d=m($c); echo $c[0], ' ', $d[0];"),
        "1 7"
    );
}

#[test]
fn array_compound_assign_on_elements() {
    assert_eq!(
        run("<?php $a=[10,20,30]; $a[0]+=5; $a[1]*=2; $a[2]++; echo implode(',', $a);"),
        "15,40,31"
    );
}

#[test]
fn array_autovivification() {
    assert_eq!(run("<?php $a=[]; $a['x']['y']=1; echo $a['x']['y'];"), "1");
    assert_eq!(
        run("<?php $b=[]; $b[]=1; $b[]=2; echo implode(',', $b);"),
        "1,2"
    );
}

#[test]
fn array_append_counter_pattern() {
    assert_eq!(
        run("<?php $c=[]; $c['x']=0; $c['x']++; $c['x']++; echo $c['x'];"),
        "2"
    );
}

#[test]
fn negative_and_sparse_keys_do_not_autorebase() {
    assert_eq!(
        run("<?php $a=[1,2,3]; $a[-1]=9; echo $a[-1], ' ', count($a);"),
        "9 4"
    );
}

#[test]
fn array_push_pop_shift_unshift() {
    assert_eq!(
        run("<?php $a=[1,2,3]; echo array_pop($a); echo '|'; echo implode(',', $a);"),
        "3|1,2"
    );
    assert_eq!(
        run("<?php $a=[1,2]; array_push($a,8,9); echo implode(',', $a);"),
        "1,2,8,9"
    );
    assert_eq!(
        run("<?php $a=[1,2,3]; echo array_shift($a); echo '|'; echo implode(',', $a);"),
        "1|2,3"
    );
    assert_eq!(
        run("<?php $a=[2,3]; array_unshift($a,0,1); echo implode(',', $a);"),
        "0,1,2,3"
    );
}

#[test]
fn array_keys_values_merge_reverse() {
    assert_eq!(
        run("<?php echo implode(',', array_keys(['x'=>1,'y'=>2]));"),
        "x,y"
    );
    assert_eq!(
        run("<?php echo implode(',', array_values(['x'=>1,'y'=>2]));"),
        "1,2"
    );
    assert_eq!(
        run("<?php echo implode(',', array_merge([1,2],[3,4]));"),
        "1,2,3,4"
    );
    assert_eq!(
        run("<?php echo implode(',', array_reverse([10,20,30]));"),
        "30,20,10"
    );
}

#[test]
fn array_merge_string_keys_override_numeric_reindex() {
    assert_eq!(
        run("<?php print_r(array_merge(['a'=>1],['a'=>2,'b'=>3]));"),
        "Array\n(\n    [a] => 2\n    [b] => 3\n)\n"
    );
}

#[test]
fn range_and_sum() {
    assert_eq!(run("<?php echo implode(',', range(1,5));"), "1,2,3,4,5");
    assert_eq!(run("<?php echo implode(',', range(3,1));"), "3,2,1");
    assert_eq!(run("<?php echo array_sum(range(1,5));"), "15");
}

#[test]
fn array_slice_positive_and_negative() {
    assert_eq!(
        run("<?php echo implode(',', array_slice([1,2,3,4,5], 1, 2));"),
        "2,3"
    );
    assert_eq!(
        run("<?php echo implode(',', array_slice([1,2,3,4,5], -2));"),
        "4,5"
    );
}

#[test]
fn in_array_and_search() {
    assert_eq!(run("<?php var_dump(in_array(2, [1,2,3]));"), "bool(true)\n");
    assert_eq!(
        run("<?php var_dump(in_array(9, [1,2,3]));"),
        "bool(false)\n"
    );
    assert_eq!(run("<?php echo array_search(2, [1,2,3]);"), "1");
}

#[test]
fn array_key_exists_vs_isset() {
    // key present with null value: isset false, array_key_exists true
    assert_eq!(
        run("<?php $a=['x'=>null]; var_dump(isset($a['x']));"),
        "bool(false)\n"
    );
    assert_eq!(
        run("<?php $a=['x'=>null]; var_dump(array_key_exists('x', $a));"),
        "bool(true)\n"
    );
    assert_eq!(
        run("<?php $a=['x'=>null]; var_dump(array_key_exists('y', $a));"),
        "bool(false)\n"
    );
}

#[test]
fn array_map_filter_reduce() {
    assert_eq!(
        run("<?php echo implode(',', array_map(fn($x)=>$x*2, [1,2,3]));"),
        "2,4,6"
    );
    assert_eq!(
        run("<?php echo implode(',', array_filter([1,2,3,4], fn($x)=>$x%2==0));"),
        "2,4"
    );
    assert_eq!(
        run("<?php echo array_reduce([1,2,3,4], fn($c,$x)=>$c+$x, 0);"),
        "10"
    );
}

#[test]
fn array_column_unique_flip_combine() {
    assert_eq!(
        run("<?php echo implode(',', array_column([['n'=>'a'],['n'=>'b']], 'n'));"),
        "a,b"
    );
    // array_unique preserves original keys
    assert_eq!(
        run("<?php print_r(array_unique([1,2,2,3,3,3]));"),
        "Array\n(\n    [0] => 1\n    [1] => 2\n    [3] => 3\n)\n"
    );
    assert_eq!(
        run("<?php print_r(array_flip(['x'=>1,'y'=>2]));"),
        "Array\n(\n    [1] => x\n    [2] => y\n)\n"
    );
    assert_eq!(
        run("<?php print_r(array_combine(['a','b'], [1,2]));"),
        "Array\n(\n    [a] => 1\n    [b] => 2\n)\n"
    );
}

#[test]
fn explode_implode_str_split() {
    assert_eq!(
        run("<?php echo implode(',', explode(',', 'a,b,c'));"),
        "a,b,c"
    );
    assert_eq!(run("<?php echo implode('-', [1,2,3]);"), "1-2-3");
    assert_eq!(run("<?php echo implode(',', str_split('abc'));"), "a,b,c");
}

#[test]
fn sort_functions() {
    assert_eq!(
        run("<?php $a=[3,1,2]; sort($a); echo implode(',', $a);"),
        "1,2,3"
    );
    assert_eq!(
        run("<?php $a=[3,1,2]; rsort($a); echo implode(',', $a);"),
        "3,2,1"
    );
    assert_eq!(
        run("<?php $a=['b'=>2,'a'=>1]; ksort($a); echo implode(',', array_keys($a));"),
        "a,b"
    );
    assert_eq!(
        run("<?php $a=[2,1,3]; usort($a, fn($x,$y)=>$y-$x); echo implode(',', $a);"),
        "3,2,1"
    );
}

#[test]
fn math_builtins() {
    assert_eq!(run("<?php echo max(1,5,3), ' ', min(4,2,8);"), "5 2");
    assert_eq!(run("<?php echo max([3,7,1]), ' ', abs(-5);"), "7 5");
    assert_eq!(
        run("<?php echo round(3.14159, 2), ' ', floor(3.7), ' ', ceil(3.2);"),
        "3.14 3 4"
    );
}

#[test]
fn json_encode_output() {
    assert_eq!(
        run("<?php echo json_encode(['a'=>1,'b'=>[2,3]]);"),
        "{\"a\":1,\"b\":[2,3]}"
    );
    assert_eq!(run("<?php echo json_encode([1,2,3]);"), "[1,2,3]");
}

// ---------------------------------------------------------------------------
// var_dump / print_r / var_export shapes
// ---------------------------------------------------------------------------

#[test]
fn var_dump_scalars() {
    assert_eq!(run("<?php var_dump(1);"), "int(1)\n");
    assert_eq!(run("<?php var_dump(1.5);"), "float(1.5)\n");
    assert_eq!(run("<?php var_dump('s');"), "string(1) \"s\"\n");
    assert_eq!(run("<?php var_dump(true);"), "bool(true)\n");
    assert_eq!(run("<?php var_dump(false);"), "bool(false)\n");
    assert_eq!(run("<?php var_dump(null);"), "NULL\n");
}

#[test]
fn var_dump_nested_array() {
    assert_eq!(
        run("<?php var_dump([1,2]);"),
        "array(2) {\n  [0]=>\n  int(1)\n  [1]=>\n  int(2)\n}\n"
    );
}

#[test]
fn print_r_nested_array() {
    assert_eq!(
        run("<?php print_r(['x'=>1,'y'=>2]);"),
        "Array\n(\n    [x] => 1\n    [y] => 2\n)\n"
    );
    assert_eq!(
        run("<?php print_r(['x'=>['y'=>1]]);"),
        "Array\n(\n    [x] => Array\n        (\n            [y] => 1\n        )\n\n)\n"
    );
}

#[test]
fn var_export_shapes() {
    assert_eq!(
        run("<?php var_export([1,'two',3.5,true,null]);"),
        "array (\n  0 => 1,\n  1 => 'two',\n  2 => 3.5,\n  3 => true,\n  4 => NULL,\n)"
    );
}

// ---------------------------------------------------------------------------
// Boolean logic and short-circuit
// ---------------------------------------------------------------------------

#[test]
fn short_circuit_and_or() {
    assert_eq!(run("<?php function t(){echo 'T'; return true;} function f(){echo 'F'; return false;} var_dump(f() && t());"), "Fbool(false)\n");
    assert_eq!(
        run("<?php function t(){echo 'T'; return true;} var_dump(t() || t());"),
        "Tbool(true)\n"
    );
}

#[test]
fn logical_not_and_bool_echo() {
    assert_eq!(run("<?php echo !true, '|', !false;"), "|1");
    assert_eq!(run("<?php echo true, '|', false;"), "1|");
    assert_eq!(run("<?php var_dump(1 && 0);"), "bool(false)\n");
    assert_eq!(run("<?php var_dump(1 || 0);"), "bool(true)\n");
    assert_eq!(run("<?php var_dump(1 xor 1);"), "bool(false)\n");
}

// ---------------------------------------------------------------------------
// Ternary / elvis / null-coalescing
// ---------------------------------------------------------------------------

#[test]
fn ternary_and_elvis() {
    assert_eq!(run("<?php $x=5; echo $x>3 ? 'big' : 'small';"), "big");
    assert_eq!(run("<?php $x=5; echo $x ?: 'zero';"), "5");
    assert_eq!(run("<?php $y=0; echo $y ?: 'zero';"), "zero");
    assert_eq!(
        run("<?php $x=1; echo $x==1 ? 'a' : ($x==2 ? 'b' : 'c');"),
        "a"
    );
}

#[test]
fn null_coalesce() {
    assert_eq!(run("<?php $a=['x'=>null]; echo $a['x'] ?? 'd1';"), "d1");
    assert_eq!(run("<?php $a=['x'=>null]; echo $a['y'] ?? 'd2';"), "d2");
    assert_eq!(
        run("<?php $a=['x'=>null]; echo $a['y']['z'] ?? 'd3';"),
        "d3"
    );
    assert_eq!(run("<?php $a=['x'=>5]; echo $a['x'] ?? 'd';"), "5");
}

#[test]
fn null_coalesce_assign_on_variable() {
    assert_eq!(run("<?php $a=null; $a ??= 5; echo $a;"), "5");
    assert_eq!(run("<?php $b=3; $b ??= 9; echo $b;"), "3");
}

// ---------------------------------------------------------------------------
// Control flow
// ---------------------------------------------------------------------------

#[test]
fn if_elseif_else() {
    assert_eq!(run("<?php function g($n){if($n>0)return 'pos'; elseif($n<0)return 'neg'; else return 'zero';} echo g(5),g(-5),g(0);"), "posnegzero");
}

#[test]
fn while_and_break_continue() {
    assert_eq!(
        run("<?php $i=0; while(true){if($i>=3)break; echo $i; $i++;}"),
        "012"
    );
    assert_eq!(
        run("<?php $i=0;$s=0; while($i<5){$i++; if($i%2==0)continue; $s+=$i;} echo $s;"),
        "9"
    );
}

#[test]
fn do_while_runs_body_once() {
    assert_eq!(run("<?php $i=0; do{echo $i;$i++;}while($i<3);"), "012");
    assert_eq!(run("<?php $j=5; do{echo $j;}while($j<3);"), "5");
}

#[test]
fn for_loop_multi_expr() {
    assert_eq!(run("<?php for($i=0;$i<3;$i++){echo $i;}"), "012");
    assert_eq!(
        run("<?php for($i=0,$j=10;$i<$j;$i++,$j--){echo \"$i-$j \";}"),
        "0-10 1-9 2-8 3-7 4-6 "
    );
}

#[test]
fn break_n_and_continue_n() {
    assert_eq!(
        run(
            "<?php for($i=0;$i<3;$i++){for($j=0;$j<3;$j++){if($i+$j==2) break 2; echo \"$i$j \";}}"
        ),
        "00 01 "
    );
    assert_eq!(
        run(
            "<?php for($i=0;$i<3;$i++){for($j=0;$j<3;$j++){if($j==1) continue 2; echo \"$i$j \";}}"
        ),
        "00 10 20 "
    );
}

#[test]
fn switch_fallthrough_and_default() {
    let prog = "<?php function t($x){switch($x){case 1: echo 'one'; case 2: echo 'two'; break; case 3: echo 'three'; default: echo 'def';}}";
    assert_eq!(run(&format!("{prog} t(1);")), "onetwo");
    assert_eq!(run(&format!("{prog} t(3);")), "threedef");
    assert_eq!(run(&format!("{prog} t(9);")), "def");
}

#[test]
fn switch_default_before_cases() {
    assert_eq!(
        run("<?php function t($x){switch($x){default: return 'd'; case 1: return 'one';}} echo t(1), t(5);"),
        "oned"
    );
}

#[test]
fn switch_on_strings() {
    assert_eq!(
        run("<?php function t($x){switch($x){case 'a':return 1;case 'b':return 2;default:return 0;}} echo t('a'),t('b'),t('z');"),
        "120"
    );
}

#[test]
fn match_exact_and_multi_condition() {
    assert_eq!(
        run("<?php echo match(3){1,2=>'lo',3,4=>'hi',default=>'d'};"),
        "hi"
    );
    assert_eq!(run("<?php echo match(true){false=>'f',default=>'t'};"), "t");
    assert_eq!(run("<?php echo match(5){1=>'a',default=>'z'};"), "z");
}

#[test]
fn match_is_strict() {
    // match uses === so '1' does not match 1
    assert_eq!(
        run("<?php echo match('1'){1=>'int',default=>'str'};"),
        "str"
    );
}

#[test]
fn match_unhandled_throws() {
    let (out, class) = run_throws("<?php echo match(5){1=>'a',2=>'b'};");
    assert_eq!(out, "");
    assert_eq!(class, "UnhandledMatchError");
}

// ---------------------------------------------------------------------------
// References
// ---------------------------------------------------------------------------

#[test]
fn reference_binding_between_variables() {
    assert_eq!(run("<?php $a=1; $b=&$a; $b=5; echo $a;"), "5");
    assert_eq!(run("<?php $a=1; $b=&$a; $a=7; echo $b;"), "7");
}

#[test]
fn reference_parameter_writes_through() {
    assert_eq!(
        run("<?php function inc(&$x){$x++;} $n=5; inc($n); echo $n;"),
        "6"
    );
    // through an existing alias
    assert_eq!(
        run("<?php $a=5; $b=&$a; function inc(&$x){$x++;} inc($b); echo $a, ' ', $b;"),
        "6 6"
    );
}

#[test]
fn reference_parameter_appends_to_array() {
    assert_eq!(
        run("<?php function push(&$arr,$v){$arr[]=$v;} $a=[1]; push($a,2); push($a,3); echo implode(',', $a);"),
        "1,2,3"
    );
}

#[test]
fn foreach_by_reference_mutates_source() {
    assert_eq!(
        run("<?php $a=[1,2,3]; foreach($a as &$v){$v*=2;} unset($v); echo implode(',', $a);"),
        "2,4,6"
    );
}

// ---------------------------------------------------------------------------
// foreach value / key
// ---------------------------------------------------------------------------

#[test]
fn foreach_value_and_key_value() {
    assert_eq!(
        run("<?php $s=0; foreach([10,20,30] as $v){$s+=$v;} echo $s;"),
        "60"
    );
    assert_eq!(
        run("<?php foreach(['x'=>1,'y'=>2] as $k=>$v){echo \"$k=$v \";}"),
        "x=1 y=2 "
    );
}

#[test]
fn foreach_list_destructuring() {
    assert_eq!(
        run("<?php foreach([[1,2],[3,4]] as [$a,$b]){echo \"$a-$b \";}"),
        "1-2 3-4 "
    );
}

// ---------------------------------------------------------------------------
// Destructuring assignment
// ---------------------------------------------------------------------------

#[test]
fn list_destructuring_variants() {
    assert_eq!(run("<?php [$a,$b,$c]=[1,2,3]; echo \"$a$b$c\";"), "123");
    assert_eq!(run("<?php list($x,,$z)=[10,20,30]; echo \"$x$z\";"), "1030");
    assert_eq!(
        run("<?php ['name'=>$n,'age'=>$a]=['name'=>'Bob','age'=>42]; echo \"$n:$a\";"),
        "Bob:42"
    );
    assert_eq!(
        run("<?php [[$p,$q],[$r,$s]]=[[1,2],[3,4]]; echo \"$p$q$r$s\";"),
        "1234"
    );
    assert_eq!(run("<?php [,$b,,$d]=[1,2,3,4]; echo \"$b$d\";"), "24");
}

#[test]
fn destructuring_swap() {
    assert_eq!(
        run("<?php $a=1;$b=2; [$a,$b]=[$b,$a]; echo \"$a $b\";"),
        "2 1"
    );
}

// ---------------------------------------------------------------------------
// Spread
// ---------------------------------------------------------------------------

#[test]
fn array_spread_numeric_and_string_keys() {
    assert_eq!(
        run("<?php $a=[1,2]; $b=[0,...$a,3]; echo implode(',', $b);"),
        "0,1,2,3"
    );
    assert_eq!(
        run("<?php $c=['x'=>1]; $d=[...$c,'y'=>2]; echo \"{$d['x']}{$d['y']}\";"),
        "12"
    );
    assert_eq!(
        run("<?php $a=['x'=>1,'y'=>2]; $b=[...$a,'z'=>3]; echo implode(',', array_keys($b));"),
        "x,y,z"
    );
}

#[test]
fn spread_as_call_arguments() {
    assert_eq!(
        run("<?php function add(...$n){return array_sum($n);} $a=[1,2,3]; echo add(...$a);"),
        "6"
    );
    assert_eq!(
        run("<?php function add(...$n){return array_sum($n);} echo add(1, ...[2,3]);"),
        "6"
    );
}
