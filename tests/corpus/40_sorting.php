<?php
$people = [
    ['name' => 'Alice', 'dept' => 'Eng', 'salary' => 100],
    ['name' => 'Bob', 'dept' => 'Eng', 'salary' => 100],
    ['name' => 'Carol', 'dept' => 'Sales', 'salary' => 90],
    ['name' => 'Dave', 'dept' => 'Eng', 'salary' => 110],
];
// multi-key sort: dept asc, salary desc, name asc
usort($people, function ($a, $b) {
    return [$a['dept'], -$b['salary'], $a['name']] <=> [$b['dept'], -$a['salary'], $b['name']];
});
foreach ($people as $p) echo "{$p['dept']} {$p['salary']} {$p['name']}\n";

$nums = [3, 1, 4, 1, 5, 9, 2, 6];
rsort($nums);
echo implode(",", $nums), "\n";
$words = ["banana", "apple", "cherry", "Date"];
sort($words, SORT_STRING | SORT_FLAG_CASE);
echo implode(",", $words), "\n";
natsort($arr = ["img10", "img2", "img1", "img12"]);
echo implode(",", $arr), "\n";
$mixed = ["10", "9", "2", "1"];
sort($mixed, SORT_NUMERIC);
echo implode(",", $mixed), "\n";
sort($mixed, SORT_STRING);
echo implode(",", $mixed), "\n";
