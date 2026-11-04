<?php
// Arrays of plain literals compile to one constant; each evaluation still
// yields an independent value.
function data() {
    return [1, 'a' => [true, null, -5, -1.5], '7' => 'x', 'b', '08' => 2,
        9223372036854775806 => 1, 'c', -3 => 'neg', 0x10 => 'hex'];
}
$a = data();
$a['a'][] = 9;
$a['z'] = 'added';
var_dump($a, data());
$big = array();
for ($i = 0; $i < 3; $i++) {
    $row = ['id' => 0, 'tags' => ['x', 'y']];
    $row['id'] = $i;
    $row['tags'][] = "t$i";
    $big[] = $row;
}
var_dump($big);
