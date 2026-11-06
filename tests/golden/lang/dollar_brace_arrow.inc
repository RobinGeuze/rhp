<?php
// An arrow function captures the variables its "${name}" parts read.
function f() {
    $a = 'v'; $i = 'k'; $arr = ['k' => 'K'];
    $g = fn() => "${a}|${arr[$i]}|" . (fn() => "${a}")();
    var_dump($g());
}
f();
