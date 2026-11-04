<?php
// An exception handler runs on {main}'s scope: globals stay reachable from
// the handler and from shutdown functions, locals of unwound frames are
// destructed before the handler, globals only after shutdown.
class D { function __construct(public $n) {} function __destruct() { echo "destruct {$this->n}\n"; } }
$cfg = "global-value";
$g = new D('global');
set_exception_handler(function ($e) {
    global $cfg;
    echo "EH: ", $e->getMessage(), " cfg=", var_export($cfg, true), "\n";
});
register_shutdown_function(function () {
    global $cfg;
    echo "shutdown sees: ", var_export($cfg, true), "\n";
});
function f() { $l = new D('local'); throw new RuntimeException("boom"); }
f();
