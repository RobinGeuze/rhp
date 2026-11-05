<?php
function sum(...$nums) {
    return array_sum($nums);
}
echo sum(1, 2, 3), "\n";
echo sum(10, 20, 30, 40), "\n";
echo sum(), "\n";
function tagged($tag, ...$items) {
    return $tag . ": " . count($items);
}
echo tagged("list", "a", "b", "c"), "\n";
