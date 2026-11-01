<?php
declare(strict_types=1);
// Dynamic calls ($f(), [$o, 'm'](), 'C::m'(), call_user_func) check parameter
// types with the caller's strict_types; callbacks run by builtins
// (array_map, usort, ...) are coercive and have no "called in" site.
function msg(Throwable $e): void {
    $m = $e->getMessage();
    $p = strpos($m, ', called in ');
    echo get_class($e), ': ', $p === false ? $m : substr($m, 0, $p) . ' [site]', "\n";
}
function a(int $x) { var_dump($x); }
function fl(float $x) { var_dump($x); }
class K {
    static function s(int $x) { var_dump($x); }
    function m(int $x): int { return $x; }
    function bad(): int { return "x"; }
    function __invoke(string $s) { var_dump($s); }
}
$f = 'a';
try { $f("1"); } catch (TypeError $e) { msg($e); }
$f(2);
$g = 'fl'; $g(3);                       // int -> float widens even in strict mode
try { call_user_func('a', "4"); } catch (TypeError $e) { msg($e); }
try { call_user_func_array('a', ["5"]); } catch (TypeError $e) { msg($e); }
try { call_user_func_array('a', ['x' => "6"]); } catch (TypeError $e) { msg($e); }
$s = 'K::s';
try { $s("7"); } catch (TypeError $e) { msg($e); }
$s(7);
$h = [new K, 'm'];
try { $h("8"); } catch (TypeError $e) { msg($e); }
var_dump($h(8));
try { (new K)->bad(); } catch (TypeError $e) { msg($e); }
$inv = new K;
try { $inv(9); } catch (TypeError $e) { msg($e); }
$c = fn(int $x) => $x;
try { $c("10"); } catch (TypeError $e) { echo "closure TypeError\n"; }
$args = ['x' => "11"];
try { $f(...$args); } catch (TypeError $e) { msg($e); }
// builtin callers are coercive
array_map('a', ["12"]);
array_map(fn(int $x) => var_dump($x), ["13"]);
array_map([new K, '__invoke'], [14]);
try { array_map('a', ["nope"]); } catch (TypeError $e) { msg($e); }
$arr = ["3", "1", "2"];
usort($arr, fn(int $l, int $r) => $l <=> $r);
var_dump($arr);
