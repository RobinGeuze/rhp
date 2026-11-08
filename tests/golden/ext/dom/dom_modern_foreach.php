<?php
// Dom\ properties are internal property handlers: foreach, get_object_vars
// and json_encode see only the slots php fills ($children, $classList).
$x = Dom\XMLDocument::createFromString('<r class="a"><c/></r>');
$e = $x->documentElement;
foreach ($e as $k => $v) {
    echo "$k ";
}
echo "\n";
var_dump(get_object_vars($e), json_encode($e));
$e->classList;
foreach ($e as $k => $v) {
    echo "$k ";
}
echo "\n";
foreach ($x as $k => $v) {
    echo "$k ";
}
echo "\n";
