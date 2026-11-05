<?php
function greet($name) {
    return "Hello, $name!";
}
echo greet("world"), "\n";
function add($a, $b) {
    return $a + $b;
}
echo add(3, 4), "\n";
echo add(add(1, 2), add(3, 4)), "\n";
