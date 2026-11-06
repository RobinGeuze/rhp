<?php
set_error_handler(function ($no, $msg) { echo "E$no: $msg
"; return true; });
// extract() and compact() in function, closure, method, generator and include scope
function modes() {
    $a = 'orig';
    $u = null; unset($u);
    $src = ['a' => 1, 'b' => 2, 'u' => 3, '1x' => 4, 7 => 5, '' => 6, 'this' => 7];
    foreach ([EXTR_OVERWRITE, EXTR_SKIP, EXTR_PREFIX_SAME, EXTR_PREFIX_ALL, EXTR_PREFIX_INVALID, EXTR_PREFIX_IF_EXISTS, EXTR_IF_EXISTS] as $m) {
        $a = 'orig'; unset($b, $u);
        try {
            $n = extract($src, $m, 'p');
        } catch (Error $e) {
            echo get_class($e), ': ', $e->getMessage(), "\n";
            $n = 'threw';
        }
        echo "mode $m: "; var_dump($n);
        $vars = get_defined_vars();
        unset($vars['src'], $vars['m'], $vars['e'], $vars['vars']);
        var_dump($vars);
    }
}
modes();

function refs() {
    $arr = ['x' => 1, 'y' => 2];
    extract($arr, EXTR_REFS);
    $x = 10;
    var_dump($arr);
    $arr['y'] = 20;
    var_dump($y);
    extract(['z' => 3], EXTR_REFS | EXTR_PREFIX_ALL, 'q');
    var_dump($q_z);
}
refs();

function compacts() {
    $a = 1; $b = [2]; $c = null;
    $r = &$a;
    var_dump(compact('a', ['b', ['c']], 'nope', 'r'));
    var_dump(compact(1, 2.5, true));
    $n = 'dyn';
    $$n = 'made';
    var_dump(compact('dyn'));
}
compacts();

$cl = function ($p) {
    $q = 2;
    extract(['w' => 9]);
    var_dump(get_defined_vars());
    return compact('p', 'q', 'w');
};
var_dump($cl(1));

function gen() {
    extract(['g' => 'in gen']);
    yield $g;
    yield compact('g');
}
foreach (gen() as $v) var_dump($v);

class K {
    public $prop = 1;
    function m($arg) {
        $local = 2;
        var_dump(get_defined_vars());
        var_dump(array_keys(compact('this', 'local')));
        try { extract(['this' => 1]); } catch (Error $e) { echo $e->getMessage(), "\n"; }
        var_dump(extract(['this' => 1], EXTR_SKIP));
        var_dump(extract(['this' => 1], EXTR_PREFIX_SAME, 'p'), $p_this);
    }
}
(new K)->m('A');

// global scope
$g1 = 1;
var_dump(extract(['g1' => 'over', 'g2' => 'new']));
var_dump($g1, $g2, $GLOBALS['g2']);
function seeglobal() { global $g2; var_dump($g2); }
seeglobal();
var_dump(compact('g1', 'g2', 'undefined_global'));
$keys = array_keys(get_defined_vars());
var_dump(array_slice($keys, -6));

// errors
try { extract([], 99); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { extract([], EXTR_PREFIX_ALL); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { extract([], EXTR_PREFIX_ALL, '1bad'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(extract(['ok' => 1], EXTR_PREFIX_ALL, ''));
var_dump($_ok);

// dynamic calls
function dyn() {
    foreach (['extract', 'compact', 'get_defined_vars', 'func_get_args', 'func_num_args'] as $f) {
        try { $f(['a']); } catch (Error $e) { echo $e->getMessage(), "\n"; }
        try { call_user_func($f, ['a']); } catch (Error $e) { echo $e->getMessage(), "\n"; }
        try { array_map($f, [['a']]); } catch (Error $e) { echo $e->getMessage(), "\n"; }
        try { $c = $f(...); $c(['a']); } catch (Error $e) { echo $e->getMessage(), "\n"; }
    }
}
dyn();
try { $f = 'func_get_args'; $f(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { array_map('func_get_args', [1]); } catch (Error $e) { echo $e->getMessage(), "\n"; }

// variable-variable names
$name = [1];
$$name = 'arr';
var_dump(${'Array'});
class S { function __toString(): string { return 'strname'; } }
${new S} = 'viaobj';
var_dump($strname);
