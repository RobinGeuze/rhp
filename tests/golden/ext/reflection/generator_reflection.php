<?php
// ReflectionGenerator: function, executing file/line, $this, the running
// generator and the trace of a `yield from` chain, before, during and
// after the generators run.
ob_start(fn ($s) => str_replace(__DIR__ . "/", "", $s));
class K {
    function inner($x) {
        yield 1;
        yield 2;
    }
    function outer() {
        yield 0;
        yield from $this->inner(5);
    }
}
function show(ReflectionGenerator $r) {
    if ($r->isClosed()) {
        echo "closed; function ", $r->getFunction()->getName(), "\n";
        try { $r->getExecutingLine(); } catch (ReflectionException $e) { echo $e->getMessage(), "\n"; }
        return;
    }
    $f = $r->getFunction();
    echo get_class($f), ' ', $f->getName(), ' line ', $r->getExecutingLine(), ' ', basename($r->getExecutingFile()), "\n";
    echo 'this: ', get_debug_type($r->getThis()), "\n";
    echo 'executing: ', (new ReflectionGenerator($r->getExecutingGenerator()))->getFunction()->getName(), "\n";
    foreach ($r->getTrace() as $i => $t) {
        echo "  #$i ", $t['class'] ?? '', $t['type'] ?? '', $t['function'],
            isset($t['line']) ? ' called at line ' . $t['line'] : '',
            ' args ', json_encode($t['args'] ?? null), isset($t['object']) ? ' +object' : '', "\n";
    }
}
$g = (new K)->outer();
$r = new ReflectionGenerator($g);
show($r);
foreach ($g as $v) {
    echo "-- at $v\n";
    show($r);
}
show($r);

$c = (function ($n) {
    $r = yield;
    show($r);
    yield $n;
})(3);
$c->current();
$c->send(new ReflectionGenerator($c));
var_dump(count((new ReflectionGenerator($c))->getTrace(DEBUG_BACKTRACE_IGNORE_ARGS)[0]));
