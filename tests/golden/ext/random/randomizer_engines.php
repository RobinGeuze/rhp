<?php
// Random\Randomizer over the seeded engines and a user engine: what the
// php-src tests leave out (clone, debug output, getFloat boundaries,
// shared state between engine and Randomizer).
use Random\Engine\{Mt19937, PcgOneseq128XslRr64, Xoshiro256StarStar};
use Random\{Randomizer, IntervalBoundary};

$r = new Randomizer(new Mt19937(42));
var_dump($r->getInt(1, 6), $r->getInt(PHP_INT_MIN, PHP_INT_MAX), $r->nextInt());
var_dump($r->nextFloat(), $r->getFloat(-1.5, 2.5));
foreach (IntervalBoundary::cases() as $b) {
    var_dump($r->getFloat(0.0, 1e-300, $b));
}
var_dump(bin2hex($r->getBytes(11)), $r->getBytesFromString('xyz', 9));
var_dump($r->shuffleArray(['a' => 1, 'b' => 2, 7 => 3]), $r->shuffleBytes('hello'));
var_dump($r->pickArrayKeys(['a' => 1, 'b' => 2, 7 => 3, 8 => 4], 3));

// The Randomizer draws from the engine object's own state.
$e = new PcgOneseq128XslRr64(7);
$r = new Randomizer($e);
$e->generate();
$copy = clone $e;
var_dump($r->getInt(0, 1000) === (new Randomizer($copy))->getInt(0, 1000));
$e->jump(5);
var_dump(bin2hex($e->generate()));

$x = new Xoshiro256StarStar(str_repeat("\1", 32));
$x->jump();
$x->jumpLong();
var_dump(bin2hex($x->generate()));
var_dump(new Mt19937(3, MT_RAND_MT19937) == new Mt19937(3));
print_r(array_slice((new Mt19937(5))->__debugInfo()['__states'], -3));
echo "\n";

final class Counter implements Random\Engine {
    private int $n = 0;
    public function generate(): string { return chr($this->n++); }
}
$r = new Randomizer(new Counter());
var_dump(bin2hex($r->getBytes(5)), $r->getInt(0, 255), $r->nextInt());
var_dump($r->pickArrayKeys(range(1, 10), 1));

foreach (['getInt' => [5, 1], 'getBytes' => [0], 'getFloat' => [1.0, NAN]] as $m => $a) {
    try {
        $r->$m(...$a);
    } catch (ValueError $e) {
        echo $e->getMessage(), "\n";
    }
}
try {
    (new Randomizer(new Mt19937(1)))->getFloat(1.0, 1.0, IntervalBoundary::OpenOpen);
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
try {
    clone new Randomizer();
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
