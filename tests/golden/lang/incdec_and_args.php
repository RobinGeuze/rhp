<?php
// ++/-- (increment_function / decrement_function): Perl-style string
// increment, the deprecations and warnings, overflow to float, on every
// kind of target. Then call arguments: a variable's value is taken when its
// argument is evaluated, and a closure called through a variable binds its
// by-reference parameters.
set_error_handler(function ($no, $str) { echo "[$no] $str\n"; return true; });
$a = ['1x', 'a', 'Az', 'zz', '', null, 1.5, '5', ' 5', '5.5', true];
foreach ($a as $k => $_) { $a[$k]++; }
var_dump($a);
$b = ['a', null, '', '5', false];
foreach ($b as $k => $_) { $b[$k]--; }
var_dump($b);
foreach (['a9', 'Zz', 'z!', '!z', 'a-z', '9', 'zZ9', 'A99', '1e2', ' 1', '1 ', 'INF'] as $x) {
    $y = $x;
    $y++;
    var_dump($y);
}
$o = new stdClass; $o->p = 'a'; $o->p++; var_dump($o->p);
class P { public static $c = 'x'; }
P::$c++; var_dump(P::$c);
$s = 'Zz'; ++$s; var_dump($s);
try { $arr = []; $arr++; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { $arr = [1]; --$arr; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$i = PHP_INT_MAX; $i++; var_dump($i);
$i = PHP_INT_MIN; $i--; var_dump($i);
$ss = (string) PHP_INT_MAX; $ss++; var_dump($ss);
$ss = (string) PHP_INT_MIN; $ss--; var_dump($ss);

$i = 4; var_dump($i, $i++, $i);
$j = 1; var_dump($j, $j = 5);
function f(...$a) { var_dump($a); }
$k = 1; f($k, $k++, $k);
$v = 'orig';
$set = fn(&$ref) => $ref = 'changed';
$set($v); var_dump($v);
$mixed = function ($a, &$b) { $a = 'A'; $b = 'B'; };
$x = 'x'; $y = 'y'; $mixed($x, $y); var_dump($x, $y);
$byval = function ($a) { $a[] = 9; return $a; };
$arr = [1]; var_dump($byval($arr), $arr);
$f2 = 'strtoupper'; var_dump($f2($v));
