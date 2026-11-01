<?php
function msg(Throwable $e): void {
    $m = $e->getMessage();
    $p = strpos($m, ', called in ');
    echo get_class($e), ': ', $p === false ? $m : substr($m, 0, $p) . ' [site]', "\n";
}
function a(int $x) { var_dump($x); }
function b(?bool $x) { var_dump($x); }
class K { function m(int|string $x) { var_dump($x); } static function f(float $x) { var_dump($x); } }
$f = 'a';
$f("1");
$f(2.0);
try { $f("x"); } catch (TypeError $e) { msg($e); }
try { $f(1e30); } catch (TypeError $e) { msg($e); }
call_user_func('a', "3");
try { call_user_func('a', []); } catch (TypeError $e) { msg($e); }
$b = 'b'; $b(0); $b(null);
$m = [new K, 'm']; $m(1.0); $m(true);
try { $m([]); } catch (TypeError $e) { msg($e); }
$sf = 'K::f'; $sf("2.5"); $sf(3);
try { $sf("z"); } catch (TypeError $e) { msg($e); }
