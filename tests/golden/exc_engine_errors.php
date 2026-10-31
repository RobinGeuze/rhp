<?php
// Engine-thrown errors carry the throw point's file/line and a php-shaped trace.
function rel($s) { return str_replace(__DIR__ . '/', '', $s); }
function show(Throwable $e) {
    echo get_debug_type($e), " | ", get_class($e), " | ", $e::class, "\n";
    echo basename($e->getFile()), ":", $e->getLine(), " code=", $e->getCode(), "\n";
    echo rel($e->getMessage()), "\n";
    foreach ($e->getTrace() as $i => $f) {
        echo "  #$i ", implode(",", array_keys($f)), " ", basename($f['file'] ?? '-'), ":", $f['line'] ?? '-', "\n";
    }
    var_dump($e->getPrevious());
    echo rel($e->getTraceAsString()), "\n---\n";
}

class Lib {
    public function undefinedFn() {
        return nope_fn(1);
    }
    public function div($a) {
        return intdiv($a, 0);
    }
    public function argc() {
        return strlen();
    }
    public function tooMany() {
        return str_repeat("x", 2, 3);
    }
    public function onNull() {
        $x = null;
        return $x->foo();
    }
    public static function noClass() {
        return new NoSuchClass();
    }
    public function typed(int $i, string ...$rest) {
        return $i;
    }
    public function needs($a, $b, $c = 1) {
        return $a;
    }
}

$l = new Lib();
foreach (['undefinedFn', 'div', 'argc', 'tooMany', 'onNull'] as $m) {
    try {
        $l->$m(5, "str");
    } catch (Throwable $e) {
        show($e);
    }
}
try { Lib::noClass(); } catch (Error $e) { show($e); }
try { $l->typed("abc"); } catch (TypeError $e) { show($e); }
try { $l->needs(1); } catch (ArgumentCountError $e) { show($e); }
try { 1 % 0; } catch (DivisionByZeroError $e) { show($e); }
try { nOpe_Mixed_Case(); } catch (Error $e) { show($e); }

// A user function invoked from a builtin callback: "[internal function]".
function cb($v) {
    if ($v > 1) {
        throw new RuntimeException("cb $v");
    }
    return $v;
}
try { array_map('cb', [1, 2, 3]); } catch (RuntimeException $e) { show($e); }
try { array_map(fn($v) => cb($v + 1), [1]); } catch (RuntimeException $e) { show($e); }
// call_user_func is compiled inline by php: no frame of its own.
try { call_user_func('cb', 9); } catch (RuntimeException $e) { show($e); }
try { call_user_func_array([new Lib(), 'needs'], [1]); } catch (ArgumentCountError $e) { show($e); }
