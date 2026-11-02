<?php
// Variable-variables as write targets beyond plain assignment: ++/--,
// unset(), `=&`, `global $$n`, list() and foreach targets (php stores the
// foreach value target before the key), plus a destructor running when the
// last by-name holder is unset.
set_error_handler(function ($no, $str) { echo "[$no] $str\n"; return true; });
$n = 'foo';
$$n = '1x';
$$n++;
var_dump($foo);
$c = 'cnt'; $cnt = 5; --$$c; var_dump($cnt, ${'c' . 'nt'}++, $cnt);

$a = 1; $an = 'a'; unset($$an); var_dump(isset($a));
$dn = 'dynglob'; $$dn = 5; unset($$dn); var_dump(isset($$dn));
function f() {
    $x = 1; $n = 'x'; unset($$n); var_dump(isset($x));
    $d = 'dd'; $$d = [1]; unset($$d); var_dump(isset($$d));
    $src = 1; $r = 'alias'; $$r = &$src; $src = 2; var_dump($$r);
    $$r = 3; var_dump($src);
    $arr = [1, 2]; $$r = &$arr[1]; $$r = 'two'; var_dump($arr[1]);
}
f();
$src = 1; $r = 'alias'; $$r = &$src; $src = 2; var_dump($alias);
$p = 'plain'; $plain = 1; $$p = &$src; $src = 9; var_dump($plain);

class D { function __destruct() { echo "dtor\n"; } }
$o = 'obj'; $$o = new D; unset($$o); echo "after unset\n";
function g() { $o = 'obj2'; $$o = new D; unset($$o); echo "after unset g\n"; }
g();

function h() { global $hg; $n = 'hg'; $$n = 'set via var-var'; }
h(); var_dump($hg);
function k() { $n = 'kk'; global $$n; $kk = 'K'; }
k(); var_dump($kk);

$k = 'kv';
list($$k, $b) = [1, 2]; var_dump($kv);
[$$k] = [5]; var_dump($kv);
foreach ([1, 2] as $$k) {}
var_dump($kv);
foreach (['x1' => 1, 'x2' => 2] as $kname => $kval) { $$kname = $kval; }
var_dump($x1 + $x2);
$prev = 'first';
foreach (['second' => 'v'] as $prev => ${$prev}) {}
var_dump($first, $prev);
$objx = new stdClass; foreach ([1, 2] as $objx->last) {} var_dump($objx->last);
$cont = []; foreach ([1, 2] as $cont[]) {} var_dump($cont);
$keyed = []; foreach (['a' => 1, 'b' => 2] as $kk2 => $keyed[$kk2 ?? 'none']) {} var_dump($keyed);
