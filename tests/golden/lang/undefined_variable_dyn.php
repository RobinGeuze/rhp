<?php
// Undefined variables passed through a dynamic call ($f($x)): the resolved
// callee decides (by-value warns, by-reference creates the variable), and
// $GLOBALS keeps only references something else still shares.
set_error_handler(function ($no, $msg, $file, $line) {
    echo "[$no] $msg (line $line)\n";
    return true;
});

function byval($a) { return $a; }
function byref(&$a) {}
class K {
    static function s($a) {}
    static function r(&$a) {}
    function m($a) {}
}

function dyn() {
    $f = 'byval'; $f($a);
    $f = 'byref'; $f($b); var_dump($b);
    $f = 'K::s'; $f($c);
    $f = 'K::r'; $f($d); var_dump($d);
    $f = [new K, 'm']; $f($e);
    $f = 'strlen'; var_dump($f($g));
    $f = 'sort';
    try { $f($h); } catch (TypeError $t) { echo $t->getMessage(), "\n"; }
    var_dump($h);
    $f = 'str_replace'; $f('a', 'b', 'c', $n); var_dump($n);
    $f = [new ArrayObject([]), 'offsetExists']; var_dump($f($k));
}
dyn();

$x = 1;
$y = &$x;
function bind() { global $z; $z = 2; }
bind();
$w = 3;
$v = &$w;
unset($v);
$g = $GLOBALS;
var_dump($g['x'], $g['y'], $g['z'], $g['w']);
$g['x'] = 10;
$g['z'] = 20;
$g['w'] = 30;
var_dump($x, $z, $w);
