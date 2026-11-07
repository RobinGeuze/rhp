<?php
// A copy of a hash with holes (zend_array_dup) moves a foreach-by-reference
// iterator that sits on a hole to the next element; the internal pointer
// moves only when it is on an element.
$a = ['a' => 1, 'b' => 2, 'c' => 3, 'd' => 4];
foreach ($a as $k => &$v) {
    echo "$k\n";
    if ($k === 'b') {
        unset($a['a'], $a['c']);
        $c = $a;
        $a['z'] = 1;
    }
}
unset($v);

$a = ['a' => 1, 'b' => 2, 'c' => 3, 'd' => 4, 'e' => 5];
foreach ($a as $k => &$v) {
    echo "$k\n";
    if ($k === 'b') {
        unset($a['a'], $a['c'], $a['d']);
        $c = $a;
        $a['f'] = 6;
    }
}
unset($v);

$a = ['a' => 1, 'b' => 2, 'c' => 3, 'd' => 4];
next($a);
next($a);
unset($a['a'], $a['c']);
$b = $a;
$b['x'] = 9;
var_dump(key($b), current($b), key($a));
