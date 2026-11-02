<?php
// Variables an included file uses in the global scope are globals, even when
// the including file never mentions them.
include __DIR__ . '/include_global_scope_a.inc';
include __DIR__ . '/include_global_scope_b.inc';

// Arrow functions auto-capture variables used by include/require/eval/exit.
function runIt(callable $f) { return $f(); }
foreach ([__DIR__ . '/include_global_scope_ret.inc'] as $file) {
    var_dump(runIt(static fn(): mixed => require_once $file));
    var_dump((fn() => include $file)());
    $code = 'return 7;';
    var_dump((fn() => eval($code))());
}
$status = 0;
$quit = fn() => exit($status);
echo "end\n";
$quit();
echo "not reached\n";
