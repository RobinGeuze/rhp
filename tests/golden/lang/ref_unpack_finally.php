<?php
// Unpacking into by-reference parameters, `return` by reference through
// finally, and `=&` on a string offset.
set_error_handler(function ($no, $msg, $file, $line) {
    if (!(error_reporting() & $no)) {
        return true;
    }
    echo "E$no: $msg (line $line)\n";
    return true;
});

function two(&$a, &$b) { $a = 'A'; $b = 'B'; }
function rest(&...$r) { foreach ($r as $k => &$x) { $x = $k; } }

// only a plain variable is changed in place
$arr = [1, 2];
two(...$arr);
var_dump($arr);
$o = new stdClass;
$o->arr = [1, 2];
two(...$o->arr);
var_dump($o->arr);
$n = ['x' => [1, 2]];
two(...$n['x']);
var_dump($n);
$x = 1;
$o->refs = [&$x, 2];
two(...$o->refs);
var_dump($x);

// a named element landing in a by-reference variadic
$p = [1, 'k' => 2];
rest(...$p);
var_dump($p);

// the notice comes with the return, after finally
function &fin() {
    try {
        return 1;
    } finally {
        echo "finally\n";
    }
}
$r =& fin();
var_dump($r);
function &fin2(): int {
    static $s = "3";
    try {
        return $s;
    } finally {
        echo "finally\n";
    }
}
$r =& fin2();
var_dump($r);

// `$str[k] =& $v`
$s = "abc";
$v = 1;
try { $s[0] =& $v; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { foreach ([1] as &$s[1]) {} } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $s[] =& $v; } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump($s);

// include, eval and silenced call results are VAR sends: a notice, no Error
function byref(&$x) {}
byref(@include __DIR__ . '/no-such-file-here.inc');
byref(eval('return 1;'));
byref(@byval());
function byval() { return 1; }
try { byref(@$v); } catch (Error $e) { echo $e->getMessage(), "\n"; }
echo "sent\n";

// foreach by reference: the key is assigned after the value target
$c = new stdClass;
$c->a = [1, 2];
foreach ($c->a as $k => &$c->a[$k]) {}
var_dump($c->a);
