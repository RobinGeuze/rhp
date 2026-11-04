<?php
// php creates a script's compiled variables (CVs) when its compiler first
// reaches them, which is not always source order. The CV order is the order
// of $GLOBALS, and globals holding the only reference to an object are
// destroyed at shutdown last-created first.
class D
{
    public function __construct(public string $n) {}
    public function __destruct() { echo "~{$this->n}\n"; }
}
class C
{
    public $p = 1;
    public static $sp = 2;
    public static function sm($a = 0) { return 3; }
}
function id($v) { return $v; }
function make($n) { return new D($n); }
function globals_keys()
{
    $skip = ['GLOBALS', '_GET', '_POST', '_COOKIE', '_FILES', '_SERVER', '_ENV', '_REQUEST', 'argv', 'argc'];
    $out = [];
    foreach ($GLOBALS as $k => $_) {
        if (!in_array($k, $skip, true)) {
            $out[] = $k;
        }
    }
    echo implode(',', $out), "\n";
}

// list(): the right-hand side first
[$la, $lb] = $lsrc ?? [1, 2];
list($lc, , $ld) = id($lsrc2 ?? [3, 4, 5]);
['x' => $le, 'y' => $lf] = ['x' => $lg ?? 6, 'y' => 7];
[$lh, [$li, $lj]] = [$lk ?? 8, [9, 10]];
// a callee before its arguments
$cr = ($cf ?? 'id')($ca ?? 11);
$cs = ($cg ?? ['f' => 'id'])['f']($cb ?? 12);
$fc = ($fo ?? 'id')(...);
// dynamic member names after the object
$o = new C;
$o->{$pn ?? 'p'} = 1;
$mr = $o->{$pv ?? 'p'};
$sr = ($sc ?? 'C')::{$smn ?? 'sm'}($sa ?? 0);
// closures bind their use() variables, arrow functions what they capture
$clo = function () use (&$cu) {};
$arr = fn() => $au;
$nest = fn() => fn() => $an;
$ncl = fn() => function () use ($anc) {};
$ac = new class($acx ?? 1) {
    public function __construct($v) {}
};
// variable variables; a constant name in ${} is that CV
$vv = ${$vn ?? 'o'};
${'lit'} = 1;
$vl = ${"lit2"} ?? 0;
$lfa = fn() => ${'lit3'};
// C::$$x reads $x, also inside an arrow function
$spf = fn() => C::$$spn;
$spg = fn() => C::${$spe ?? 'sp'};
// while and for compile the body before the condition
while ($wc ?? false) {
    $wb = 1;
}
for ($fi = 0; $fi < ($fn ?? 0); $fi += ($fs ?? 1)) {
    $fb = 1;
}
// switch and match: every case condition before the bodies
switch ($sw ?? 1) {
    case ($s1 ?? 0):
        $sb1 = 1;
        break;
    case ($s2 ?? 1):
        $sb2 = 1;
        break;
    default:
        $sd = 1;
}
$mt = match ($ms ?? 1) {
    ($m1 ?? 0) => $mb1 ?? 0,
    ($m2 ?? 1) => $mb2 ?? 0,
    default => $md ?? 0,
};
// declare() bodies, static initializers
declare(ticks=1) {
    $dt = 1;
}
static $st = $si ?? 1;
$lsrc = $lsrc2 = $lg = $lk = $cf = $ca = $cg = $cb = $fo = $pn = $pv = $sc = $smn = $sa = 0;
$cu = $au = $an = $anc = $acx = $vn = $wc = $wb = $fn = $fs = $fb = 0;
$sw = $s1 = $s2 = $sb1 = $sb2 = $sd = $ms = $m1 = $m2 = $mb1 = $mb2 = $md = $dt = $si = 0;
$lit2 = $lit3 = $spn = $spe = 0;
globals_keys();
$spn2 = 'sp';
$lit4 = 5;
echo (fn() => C::$$spn2)(), ' ', (fn() => ${'lit4'})(), "\n";

// at shutdown
[$xa, $xb] = $xpair ?? [new D('xa'), new D('xb')];
$xr = ($xfn ?? 'make')($xarg ?? 'xr');
while (!isset($xw)) {
    $xwb = new D('xwb');
    $xw = new D('xw');
}
switch ($xsw ?? 1) {
    case 0:
        $xs0 = new D('xs0');
        break;
    case ($xc ?? 1):
        $xs1 = new D('xs1');
}
$xf = fn() => $xcap;
$xpair = new D('xpair');
$xfn = new D('xfn');
$xarg = new D('xarg');
$xsw = new D('xsw');
$xc = new D('xc');
$xcap = new D('xcap');
echo "end\n";
