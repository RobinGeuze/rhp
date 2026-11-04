<?php
// Request shutdown: shutdown functions (in order, with args, nested
// registration, globals still visible), then destructors (globals holding the
// sole reference first, last-declared first; then the rest in creation order),
// then the output buffers are flushed. Plus destructors on release at runtime.
class D {
    public function __construct(public $n) { echo "ctor {$this->n}\n"; }
    public function __destruct() { echo "destruct {$this->n}\n"; }
}
class Holder {
    public $d;
    public function __construct($n) { $this->d = new D($n); }
}

function f() { $x = new D('local'); echo "in f\n"; }
f();
echo "after f\n";
$a = new D('a1');
$a = null;
echo "after null\n";
$b = new D('b1');
unset($b);
echo "after unset\n";
$c = new D('c1');
$c = new D('c2');
echo "after reassign\n";
$arr = [new D('arr')];
$arr = [];
echo "after arr\n";
$h = new Holder('held');
$h = null;
echo "after holder\n";
$e = new D('e');
$e2 = clone $e;
$e2->n = 'e-clone';
$e2 = null;
echo "after clone\n";
function g() { return new D('ret'); }
g();
echo "after g\n";
new D('temp');
echo "after temp\n";

$sa = new D('sa');
$sb = new D('sb');
$sc = new D('sc');
$sb2 = $sb;
$list = [new D('in-array')];
$nested = new Holder('nested');
register_shutdown_function(function ($x) {
    echo "shutdown1 $x\n";
    global $sa;
    echo "global sa: {$sa->n}\n";
    register_shutdown_function(fn() => print("nested shutdown\n"));
}, 'arg');
register_shutdown_function(function () { echo "shutdown2\n"; });
register_shutdown_function('printf', "printf %s\n", 'shutdown3');
try {
    register_shutdown_function('no_such_function_xyz');
} catch (TypeError $ex) {
    echo $ex->getMessage(), "\n";
}
try {
    register_shutdown_function([new stdClass, 'nope']);
} catch (TypeError $ex) {
    echo $ex->getMessage(), "\n";
}
ob_start();
echo "buffered at end\n";
echo "end of script\n";
