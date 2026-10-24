<?php
function makeCounter(int $start = 0): array {
    $count = $start;
    return [
        'inc' => function () use (&$count) { return ++$count; },
        'get' => function () use (&$count) { return $count; },
        'reset' => function () use (&$count, $start) { $count = $start; },
    ];
}
$c = makeCounter(10);
echo $c['inc'](), $c['inc'](), $c['get'](), "\n";
$c['reset']();
echo $c['get'](), "\n";

$multiplier = fn($factor) => fn($x) => $x * $factor;
$double = $multiplier(2);
$triple = $multiplier(3);
echo $double(5), " ", $triple(5), "\n";

$compose = fn(callable $f, callable $g) => fn($x) => $f($g($x));
$addOne = fn($x) => $x + 1;
$sq = fn($x) => $x * $x;
$h = $compose($sq, $addOne);
echo $h(4), "\n";

$pipeline = array_reduce(
    [fn($x) => $x + 1, fn($x) => $x * 2, fn($x) => $x - 3],
    fn($acc, $f) => fn($x) => $f($acc($x)),
    fn($x) => $x
);
echo $pipeline(5), "\n";

class Calc {
    public int $base = 100;
    public function makeAdder(): Closure {
        return function (int $x): int { return $this->base + $x; };
    }
}
$adder = (new Calc())->makeAdder();
echo $adder(5), "\n";
