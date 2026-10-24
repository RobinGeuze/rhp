<?php
$users = [
    ['name' => 'Alice', 'age' => 30, 'tags' => ['admin', 'dev']],
    ['name' => 'Bob', 'age' => 25, 'tags' => ['dev']],
    ['name' => 'Carol', 'age' => 35, 'tags' => ['admin']],
    ['name' => 'Dave', 'age' => 28, 'tags' => []],
];
$adults = array_filter($users, fn($u) => $u['age'] >= 28);
$names = array_map(fn($u) => strtoupper($u['name']), $adults);
echo implode(", ", $names), "\n";
$totalAge = array_reduce($users, fn($c, $u) => $c + $u['age'], 0);
echo "avg age: ", round($totalAge / count($users), 2), "\n";
$byTag = [];
foreach ($users as $u) {
    foreach ($u['tags'] as $t) {
        $byTag[$t][] = $u['name'];
    }
}
ksort($byTag);
foreach ($byTag as $tag => $ns) {
    echo "$tag: " . implode(",", $ns) . "\n";
}
usort($users, fn($a, $b) => $a['age'] <=> $b['age']);
echo implode(",", array_column($users, 'name')), "\n";
