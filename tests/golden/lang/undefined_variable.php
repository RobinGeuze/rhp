<?php
// Reads of undefined variables (php's IS_UNDEF CVs): which fetches warn
// ("Undefined variable $x"), which stay quiet, and what is defined after.
set_error_handler(function ($no, $msg, $file, $line) {
    echo "[$no] $msg (line $line)\n";
    return true;
});

function reads() {
    echo $a;
    $b = $c . "x";
    var_dump($b, $d + 1, -$e, !$f, (int) $g, $h == 1, [$i, 'k' => $j]);
    var_dump($k[0], $l->p, "in $m string");
    unset($n);
    echo $n;
    $o = 1;
    unset($o);
    var_dump($o);
    // The value is read before the key; an undefined key variable in an
    // array literal is "" without the null-offset deprecation.
    var_dump([$kk => $vv]);
    $p = [];
    $p[$pk] = $pv;
    $p[null][null] = $pv2;
    var_dump($p);
}
reads();

function quiet() {
    var_dump(isset($a), empty($b), $c ?? 'dflt', isset($d[0]), $e?->p, isset($f->p));
    $g ??= 5;
    var_dump($g);
    $h[] = 1;
    $i['x']['y'] = 2;
    var_dump($h, $i);
}
quiet();

function rw() {
    $a .= "s";
    $b += 2;
    $c++;
    --$d;
    $e[0] .= "t";
    $f['k']++;
    unset($g[0]);
    var_dump($a, $b, $c, $d, $e, $f);
    echo $g;
}
rw();

function lists() {
    [$a, $b] = $src;
    ['x' => $c] = $src2;
    var_dump($a, $b, $c);
    foreach ($none as $v) {
    }
    foreach ($none2 as &$v) {
    }
    echo $none2;
}
lists();

function closures() {
    $f = function () use ($cap) {
        return $cap;
    };
    var_dump($f());
    $g = fn() => $out;
    var_dump($g());
    $h = function () use (&$byref) {
        return $byref;
    };
    var_dump($h(), $byref);
}
closures();

function refs() {
    $r = &$x;
    var_dump($x);
    static $s;
    global $glob;
    var_dump($s, $glob);
    settype($t, "array");
    var_dump($t);
}
refs();

class M {
    public function byVal($v) { return $v; }
    public function byRef(&$v) { $v = 'set'; }
    public static function sVal($v) { return $v; }
    public function __call($name, $args) { echo "__call $name\n"; }
}
function methods() {
    $m = new M;
    var_dump($m->byVal($a));
    $m->byRef($b);
    var_dump($b);
    var_dump(M::sVal($c));
    $m->missing($d);
    echo $d;
}
methods();

function matches() {
    echo match ($u) { 1, 2, 3, 4, 5 => 'num', default => 'none' }, "\n";
    echo match ($w) { 'a', 'b' => 'str', default => 'none' }, "\n";
    echo match ($x) { 1 => 'one', 2 => 'two', default => 'none' }, "\n";
    echo match ($y) { 1 => 'one', f1() => 'two', default => 'none' }, "\n";
    switch ($z) {
        case 1:
        case 2:
            break;
    }
}
function f1() { return 1; }
matches();

function silence() {
    echo @$a;
    $old = error_reporting(E_ALL & ~E_WARNING);
    echo $b;
    error_reporting($old);
    @error_reporting(E_ALL);
    echo $c;
}
silence();

// The global scope: $GLOBALS leaves undefined variables out, unset keeps
// the order of the symbol table.
$g1 = 1;
$g2 = null;
$g3 = 3;
unset($GLOBALS['g1']);
echo $g1;
$g1 = 'again';
foreach ($GLOBALS as $k => $v) {
    if ($k[0] === 'g') {
        echo "$k\n";
    }
}
var_dump(array_key_exists('g2', $GLOBALS), isset($GLOBALS['g4']));
echo $GLOBALS['g4'] ?? "quiet\n";
$GLOBALS['g5'] .= 'x';
var_dump($g5);

function throwing() {
    set_error_handler(function ($no, $msg) {
        throw new ErrorException($msg);
    });
    try {
        $x = $undef + 1;
    } catch (ErrorException $e) {
        echo "caught: ", $e->getMessage(), "\n";
    }
    var_dump(isset($x));
    try {
        $y++;
    } catch (ErrorException $e) {
        echo "caught: ", $e->getMessage(), "\n";
    }
    var_dump($y ?? 'still undefined');
    restore_error_handler();
}
throwing();
