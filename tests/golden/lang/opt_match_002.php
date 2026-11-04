<?php
// php-src ext/opcache/tests/match/002.phpt without its opcode dump
// (tests/phpt/ignored.txt): Test match jump table optimizer

function test() {
    $x = '2';
    echo match($x) {
        1, 2, 3, 4, 5 => throw new RuntimeException(),
        default => "No match\n",
    };
}
test();

function test2() {
    $x = 2;
    echo match($x) {
        '1', '2', '3', '4', '5' => throw new RuntimeException(),
        default => "No match\n",
    };
}
test2();

?>
