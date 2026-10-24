<?php
function multiply(array $a, array $b): array {
    $n = count($a); $m = count($b[0]); $p = count($b);
    $c = [];
    for ($i = 0; $i < $n; $i++) {
        for ($j = 0; $j < $m; $j++) {
            $sum = 0;
            for ($k = 0; $k < $p; $k++) $sum += $a[$i][$k] * $b[$k][$j];
            $c[$i][$j] = $sum;
        }
    }
    return $c;
}
$a = [[1, 2], [3, 4]];
$b = [[5, 6], [7, 8]];
$c = multiply($a, $b);
foreach ($c as $row) echo implode(" ", $row), "\n";

function transpose(array $m): array {
    return array_map(null, ...$m);
}
print_r(transpose([[1, 2, 3], [4, 5, 6]]));

$id = [];
for ($i = 0; $i < 3; $i++)
    for ($j = 0; $j < 3; $j++)
        $id[$i][$j] = ($i === $j) ? 1 : 0;
echo json_encode($id), "\n";
