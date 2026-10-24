<?php
function quicksort(array $a): array {
    if (count($a) <= 1) return $a;
    $pivot = $a[intdiv(count($a), 2)];
    $less = $eq = $more = [];
    foreach ($a as $x) {
        if ($x < $pivot) $less[] = $x;
        elseif ($x > $pivot) $more[] = $x;
        else $eq[] = $x;
    }
    return array_merge(quicksort($less), $eq, quicksort($more));
}
$data = [5, 3, 8, 1, 9, 2, 7, 4, 6, 0, 3, 8];
print_r(quicksort($data));
echo implode(",", quicksort([3.1, 2.2, 5.5, 1.0])), "\n";
