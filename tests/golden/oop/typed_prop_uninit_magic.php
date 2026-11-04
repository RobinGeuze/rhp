<?php
// An uninitialized typed property bypasses the magic accessors (php's
// IS_PROP_UNINIT); only after unset() does an absent slot defer to them.
class M
{
    public function __set($n, $v) { echo "__set($n)\n"; }
    public function __get($n) { echo "__get($n)\n"; return 42; }
    public function __isset($n) { echo "__isset($n)\n"; return true; }
    public function __unset($n) { echo "__unset($n)\n"; }
}
class K extends M
{
    private int $p;
    public function __construct() { $this->p = 1; var_dump($this->p); }
}
new K;

class Q extends M
{
    public int $q;
    public array $arr;
    public ?int $n = 5;
    public function t() { unset($this->q); $this->q = 2; }
}
(new Q)->t();

$q = new Q;
var_dump(isset($q->q), empty($q->q), $q->q ?? 'dflt');
try {
    var_dump($q->q);
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
unset($q->q);   // uninitialized: clears the flag, no __unset
unset($q->q);   // now __unset
var_dump(isset($q->q), $q->q);
$q->arr[] = 1;
var_dump($q->arr);
unset($q->n);
var_dump($q->n);
$q->n = 3;
var_dump(isset($q->n));
$c = clone $q;
var_dump($c->n);

class P { public int $x; }
$p = new P;
unset($p->x);
try {
    var_dump($p->x);
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
$p->x = 9;
var_dump($p->x);

// asymmetric visibility: an uninitialized property errors, an unset one
// defers to __set
class Asym
{
    public private(set) string $bar;
    public function setBar($b) { $this->bar = $b; }
    public function unsetBar() { unset($this->bar); }
    public function __set($n, $v) { echo "Asym::__set($n)\n"; }
}
$a = new Asym;
try {
    $a->bar = 'x';
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
$a->setBar('y');
$a->unsetBar();
$a->bar = 'z';
