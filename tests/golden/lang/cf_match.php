<?php
$x = 3;
$r = match($x) {
    1, 2 => "low",
    3, 4 => "mid",
    default => "high",
};
echo $r, "\n";
$t = "b";
echo match($t) { "a" => "Apple", "b" => "Banana", "c" => "Cherry" }, "\n";
$n = 0;
echo match(true) { $n > 0 => "pos", $n < 0 => "neg", default => "zero" }, "\n";
