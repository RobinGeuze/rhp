<?php
// Engine diagnostics reach the user error handler (composer turns them
// into ErrorException); messages per php 8.5.
set_error_handler(function ($l, $m, $f, $n) {
    echo "[$l] $m @", basename($f), ":$n\n";
    return true;
});
$a = ['x' => 1]; $n = null; $b = false; $i = 5; $fl = 1.5; $s = "abc";
var_dump($a['y'], $a[3], $n['k'], $b[0], $i[0], $fl[0]);
var_dump($s[5], $s[-5], $s['1'], $s['1x'], $s[1.7]);
try { var_dump($s['x']); } catch (\Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { $o = new stdClass; var_dump($o['a']); } catch (\Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
[$p, $q] = [1]; var_dump($q);
[$p] = null; [$p] = 5; [$p] = 'abc';
['k' => $z] = ['j' => 1];
var_dump($a['x']['y'], $a['m']['n']);
var_dump(isset($a['m']['n']), empty($a['m']), $a['m']['n'] ?? 3, $s[9] ?? 'd', $s[1] ?? 'd');
var_dump($a[true], $a[null], $a[1.5]);
// properties
class A { public $d; }
$o = new A;
var_dump($o->nope, $n->x, $b->x, $i->x);
unset($o->d); var_dump($o->d, $o->nope ?? 'dflt');
$o->dyn = 1;
#[AllowDynamicProperties] class B {}
$ob = new B; $ob->q = 1;
try { $n->foo = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
// conversions
$arr = [1];
echo $arr, "\n"; $t = "v=$arr"; var_dump($t);
// null to non-nullable internal parameters; zpp TypeErrors
var_dump(strlen(null), str_replace('a', 'b', null), array_map('trim', [null]));
try { strlen([]); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { array_keys('x'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
// composer's ErrorHandler pattern
set_error_handler(function ($l, $m, $f, $n) { throw new ErrorException($m, 0, $l, $f, $n); });
try { $r = $a['missing']; echo "not reached\n"; } catch (ErrorException $e) { echo "caught: ", $e->getMessage(), "\n"; }
restore_error_handler(); restore_error_handler();
var_dump(@$a['silenced'], error_get_last()['message']);
