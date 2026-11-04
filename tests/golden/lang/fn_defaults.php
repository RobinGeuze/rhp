<?php
function power($base, $exp = 2) {
    return $base ** $exp;
}
echo power(5), "\n";
echo power(2, 10), "\n";
function greet($name, $greeting = "Hello", $punct = "!") {
    return "$greeting, $name$punct";
}
echo greet("Bob"), "\n";
echo greet("Al", "Hi"), "\n";
echo greet("X", "Yo", "?"), "\n";
