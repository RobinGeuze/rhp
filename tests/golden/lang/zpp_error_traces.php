<?php
// zpp errors are raised in the builtin's frame, so the trace lists the
// call. Its arguments show the conversions zpp makes in place (to a string,
// a number, a class name) but not those into an int, float or bool, and
// none for a frameless call. (The handler keeps the checkout's path out.)
set_error_handler(function ($no, $msg) {
    echo $no === E_DEPRECATED ? "Deprecated" : "Warning", ": $msg\n";
    return true;
});
class S { function __toString(): string { return "s"; } }
function t(callable $c) {
    try {
        $c();
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
        echo str_replace(__DIR__ . '/', '', $e->getTraceAsString()), "\n";
    }
}
function f() { str_repeat("x", []); }
t(fn() => f());
t(fn() => str_repeat(5, []));
t(fn() => str_repeat(new S, []));
t(fn() => str_pad(5, "x"));
t(fn() => intdiv("4", "x"));
t(fn() => array_slice([1], "1", []));
t(fn() => round("1.5", "x"));
t(fn() => fwrite(STDOUT, 5, "x"));
t(fn() => fread(STDIN, "x"));
t(fn() => substr(5, []));
t(fn() => strpos(5, "x", "y"));
$f = "substr";
t(fn() => $f(5, []));
t(fn() => lcg_value(1));
