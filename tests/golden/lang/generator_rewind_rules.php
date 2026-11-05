<?php
// Generator rewind / re-iteration rules and by-reference yields of values
// that are not references.
set_error_handler(function ($no, $msg, $file, $line) {
    echo "[$no] $msg (line $line)\n";
    return true;
});
function attempt(callable $f) {
    try {
        $f();
    } catch (Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
function gen() { yield 1; yield 2; }
function &refgen() { $x = 1; yield $x; yield 2; }

attempt(function () { $g = gen(); $g->next(); foreach ($g as $v) { echo $v, "\n"; } });
attempt(function () { $g = gen(); foreach ($g as $v) {} foreach ($g as $v) {} });
attempt(function () { $g = gen(); foreach ($g as $v) {} $g->rewind(); });
attempt(function () { $g = gen(); $g->current(); $g->rewind(); foreach ($g as $v) { echo $v, "\n"; } });
attempt(function () { $g = gen(); foreach ($g as &$v) {} });
attempt(function () { $g = refgen(); $g->next(); foreach ($g as &$v) {} });
attempt(function () { $g = refgen(); foreach ($g as &$v) { echo $v, "\n"; } foreach ($g as &$v) {} });

function &lit() { yield 1 + 1; yield; }
foreach (lit() as &$v) {
    var_dump($v);
}
function plain() { return 'p'; }
function &viaCall() { yield plain(); }
foreach (viaCall() as &$v) {
    var_dump($v);
}
