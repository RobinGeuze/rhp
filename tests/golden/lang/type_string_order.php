<?php
declare(strict_types=1);
// TypeError messages print types in php's canonical order: class names, then
// static|callable|object|array|string|int|float|bool|false|true|void|never,
// null last (or ?T); iterable is Traversable|array.
interface I {} interface J {}
function msg(Throwable $e): void {
    $m = $e->getMessage();
    $p = strpos($m, ', called in ');
    echo $p === false ? $m : substr($m, 0, $p), "\n";
}
function a(int|string $x) {} function b(?iterable $x) {} function c(null|bool|float|array|int $x) {}
function d(true|null $x) {} function e(I&J $x) {} function f(int|I|null $x) {} function g(callable|object|false $x) {}
function h(?int $x) {} function i(iterable|string $x) {} function j(): int|float { return "s"; }
function k(float|int|string|bool $x) {} function l(J|I $x) {}
try { a(1.5); } catch (TypeError $e) { msg($e); }
try { b(1); } catch (TypeError $e) { msg($e); }
try { c("s"); } catch (TypeError $e) { msg($e); }
try { d(1); } catch (TypeError $e) { msg($e); }
try { e(1); } catch (TypeError $e) { msg($e); }
try { f("s"); } catch (TypeError $e) { msg($e); }
try { g(1); } catch (TypeError $e) { msg($e); }
try { h("s"); } catch (TypeError $e) { msg($e); }
try { i(1); } catch (TypeError $e) { msg($e); }
try { j(); } catch (TypeError $e) { msg($e); }
try { k([]); } catch (TypeError $e) { msg($e); }
try { l(1); } catch (TypeError $e) { msg($e); }
class P { public int|string $u = 1; public static float|bool|null $s = null; }
class Q { public int|string $u = R::F; } class R { const F = 1.5; }
try { new Q; } catch (TypeError $e) { msg($e); }
