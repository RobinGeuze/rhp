<?php
// Calls through a variable: a function's name, a closure, a
// first-class callable, call_user_func and array_map with a name.
function Twice($x) { return $x * 2; }
$t = 0; $n = 'twice'; $N = 'Twice'; $c = fn($x) => $x + 1; $fc = Twice(...); $a = [1, 2, 3];
for ($i = 0; $i < 1000000; $i++) {
    $t = ($t + $n($i & 7) + $N($t & 3) + $c($i) + $fc(1)) & 0xffff;
    $t = ($t + call_user_func($n, 3)) & 0xffff;
}
for ($i = 0; $i < 100000; $i++) { $t += array_sum(array_map('Twice', $a)) & 7; }
echo $t, "\n";
