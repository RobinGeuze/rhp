<?php
$len = strlen(...);
echo $len("hello"), "\n";
var_dump($len instanceof Closure);
echo implode(",", array_map(strtoupper(...), ["a", "b", "c"])), "\n";

function add($a, $b) { return $a + $b; }
$a = add(...);
echo $a(3, 4), "\n";

class Calc {
    public int $base = 100;
    public function addBase(int $x): int { return $this->base + $x; }
    public static function mul(int $a, int $b): int { return $a * $b; }
}
$c = new Calc();
$f = $c->addBase(...);
echo $f(5), "\n";
$m = Calc::mul(...);
echo $m(6, 7), "\n";
echo implode(",", array_map($c->addBase(...), [1, 2, 3])), "\n";

$cb = [new Calc(), 'addBase'];
echo $cb(50), "\n";
