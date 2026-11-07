<?php
// explode(): every limit branch, a missing separator, an empty string.
$cases = [
    [',', 'a,b,,c', null], [',', 'a,b,,c', 2], [',', 'a,b,,c', 1],
    [',', 'a,b,,c', 0], [',', 'a,b,,c', -1], [',', 'a,b,,c', -4],
    [',', 'a,b,,c', -10], [',', 'abc', -1], [',', 'abc', 3], [',', '', -1],
    [',', '', 0], ['ab', 'xabyabab', null], ['ab', 'xabyabab', -2],
    ['long separator', 'x', null], [',', ',', null], [',', ',', -1],
];
foreach ($cases as [$sep, $s, $limit]) {
    $r = $limit === null ? explode($sep, $s) : explode($sep, $s, $limit);
    echo json_encode([$sep, $s, $limit]), ' => ', json_encode($r), "\n";
}
// The separator may be longer than the string (explode_bug.phpt's case).
var_dump(explode(str_repeat('A', 1000), '1'));
try {
    explode('', 'x');
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
