<?php
for ($i = 0; $i < 4; $i++) {
    echo "i=$i ";
}
echo "\n";
$total = 0;
for ($j = 1; $j <= 10; $j++) {
    if ($j == 5) continue;
    if ($j == 8) break;
    $total += $j;
}
echo "total=$total\n";
for ($a = 0, $b = 10; $a < $b; $a++, $b--) {
    echo "$a-$b ";
}
echo "\n";
