<?php
// Variables that appear only inside constructs the CV pre-pass must still
// see (exit/die operands, closure `use` lists, yield, include operands), and
// arrow functions capturing variables used in include/require/eval-free
// operands, casts, instanceof, yield and list targets.
$tmp = sys_get_temp_dir() . '/rhp_cv_scan_' . getmypid() . '.php';
file_put_contents($tmp, '<?php return 42;');

$f = static fn() => require $tmp;
var_dump($f());
$g = fn() => include $tmp;
var_dump($g());
$c = 5;
$h = fn() => (string)$c . ($c instanceof stdClass ? 'y' : 'n');
var_dump($h());
$y = 3;
$gen = fn() => yield $y;
foreach ($gen() as $v) var_dump($v);
$arr = [[1, 2]];
$d = fn() => [[$p, $q]] = $arr;
var_dump($d());
$k = 'z';
$mk = fn() => [$k => 1];
var_dump($mk());
$q1 = 1;
$nest = fn() => fn() => $q1 + 1;
var_dump($nest()());
$w = 2;
$cl = fn() => function () use ($w) { return $w; };
var_dump($cl()());
unlink($tmp);

function byref() {
    $f = function () use (&$only) { $only = 'set by closure'; };
    $f();
    return $only;
}
var_dump(byref());

function gen2() { $r = yield; echo "got $r\n"; }
$gg = gen2(); $gg->current(); $gg->send('sent');

function yields_in_args() { $x = strtoupper(yield 'k'); return $x; }
$yi = yields_in_args(); var_dump($yi->current()); $yi->send('v'); var_dump($yi->getReturn());

class App { function run($argv) { return "ran with " . count($argv) . " arg(s)\n"; } }
exit((new App)->run(['a']));
