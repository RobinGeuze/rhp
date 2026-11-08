<?php
// Closure::call runs the closure from an internal frame: traces show
// `[internal function]: A->{closure}()` called by `Closure->call(...)`.
class A { private $x = 1; }
$g = function () { throw new Exception('e'); };
try { $g->call(new A); } catch (Exception $e) {
    foreach ($e->getTrace() as $t) { echo isset($t['file']) ? basename($t['file']) : '[internal]', ' ', $t['class'] ?? '', $t['type'] ?? '', $t['function'] === 'call' ? 'call' : 'closure', ' ', count($t['args'] ?? []), "\n"; }
}
$h = function ($a) {};
try { $h->call(new A, b: 1); } catch (Error $e) {
    echo $e->getMessage(), "\n";
    foreach ($e->getTrace() as $t) { echo basename($t['file']), ' ', $t['class'], $t['type'], $t['function'], ' ', json_encode(array_keys($t['args'])), "\n"; }
}
$f = function () { return array_map(fn($t) => ($t['class'] ?? '') . ($t['type'] ?? '') . (str_starts_with($t['function'], '{closure') ? '{closure}' : $t['function']), debug_backtrace()); };
var_dump($f->call(new A));
