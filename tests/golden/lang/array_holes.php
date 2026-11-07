<?php
$a = [1,2,3,4,5];
next($a); next($a);
unset($a[2]);
var_dump(current($a), key($a));
prev($a); var_dump(current($a));
end($a); unset($a[4]); var_dump(current($a), key($a));
$a[] = 9; var_dump(current($a), key($a));
debug_zval_dump($a);
$b = ['x'=>1,'y'=>2,'z'=>3];
next($b); unset($b['y']); var_dump(key($b));
debug_zval_dump($b);
$c = [3=>1, 1=>2]; debug_zval_dump($c);
$d = []; $d[5] = 1; $d[6] = 2; debug_zval_dump($d);
$e = range(1, 20); unset($e[3]); $e[] = 1; debug_zval_dump($e);
$f = [1,2,3]; unset($f[1]); $f[1] = 5; var_dump($f);
$g = [1,2,3]; foreach ($g as $k => &$v) { if ($k == 0) { unset($g[1]); $g[] = 4; } echo "$k=>$v\n"; } unset($v);
$h = [1,2,3,4]; array_shift($h); debug_zval_dump($h); var_dump(array_pop($h)); $h[] = 'n'; var_dump($h);
$i = ['a'=>1, 5=>2, 9=>3]; array_shift($i); var_dump($i); $i[] = 'q'; var_dump($i);
$j = [5=>'a', 'b']; array_unshift($j, 'z'); var_dump($j);
$k = [3,1,2]; asort($k); debug_zval_dump($k); sort($k); debug_zval_dump($k);
$l = []; for ($q = 0; $q < 100; $q++) { $l["k$q"] = $q; } for ($q = 0; $q < 90; $q++) unset($l["k$q"]); for ($q = 0; $q < 100; $q++) { $l["n$q"] = $q; } var_dump(count($l), array_key_first($l), array_key_last($l));
$m = [0,1,2,3]; unset($m[3], $m[2]); $m[] = 'x'; var_dump($m);
$n = [0,1,2,3]; unset($n[3], $n[2]); $n[1] = 'y'; $n[2] = 'z'; var_dump($n); $n[] = 'w'; var_dump($n);
$o = [1,2,3]; $p = $o; unset($p[0]); var_dump(current($p), key($p));
$w = [1,2,3,4,5]; array_walk($w, function(&$v, $k) use (&$w) { if ($k == 1) unset($w[3]); echo "$k "; }); echo "\n";
$s = [PHP_INT_MAX => 1]; try { $s[] = 2; } catch (Error $ex) { echo $ex->getMessage(), "\n"; }
$t = [-5 => 'a']; $t[] = 'b'; var_dump($t);
var_dump(array_splice($e, 2, 3, ['x','y']), count($e));
// Holes seen by the readers.
$u = [0, 1, 2, 3, 4, 5]; unset($u[1], $u[4]);
foreach ($u as $k => $v) echo "$k:$v ";
echo "\n";
var_dump(array_is_list($u), array_values($u), array_slice($u, 1, 2), json_encode($u));
echo serialize($u), "\n";
var_export($u); echo "\n";
$x = ['a' => 1, 'b' => 2, 'c' => 3, 'd' => 4]; unset($x['b']);
var_dump(array_slice($x, 1), array_search(4, $x), array_keys($x), in_array(2, $x));
$y = $x; $y['e'] = 5; unset($y['a']); var_dump($x, $y);
$z = []; for ($i = 0; $i < 20; $i++) { $z["k$i"] = $i; if ($i % 2) unset($z["k" . ($i - 1)]); }
echo implode(',', array_keys($z)), "\n";
$p = [1, 2, 3]; unset($p[0]); array_unshift($p, 0); var_dump($p);
$q = [1, 2, 3, 4]; unset($q[1]); var_dump(array_pop($q), array_shift($q), $q);
$r = [3 => 'c', 1 => 'a', 2 => 'b']; ksort($r); debug_zval_dump($r); $r[] = 'd'; var_dump($r);
$big = []; $big[7] = 1; $big[15] = 2; debug_zval_dump($big);
$big2 = []; $big2[0] = 0; $big2[3] = 1; $big2[12] = 2; debug_zval_dump($big2);
debug_zval_dump(unserialize('a:2:{i:0;i:1;i:1;i:2;}'));
