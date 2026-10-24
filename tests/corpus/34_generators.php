<?php
function gen(): Generator {
    yield 1;
    yield 2;
    yield 3;
}
foreach (gen() as $v) echo $v;
echo "\n";
function keyed(): Generator {
    yield 'a' => 1;
    yield 'b' => 2;
}
foreach (keyed() as $k => $v) echo "$k=$v ";
echo "\n";
function fibGen(int $n): Generator {
    [$a, $b] = [0, 1];
    for ($i = 0; $i < $n; $i++) {
        yield $a;
        [$a, $b] = [$b, $a + $b];
    }
}
echo implode(",", iterator_to_array(fibGen(10))), "\n";
function rangeGen($s, $e) {
    for ($i = $s; $i <= $e; $i++) yield $i;
}
echo array_sum(iterator_to_array(rangeGen(1, 100))), "\n";
