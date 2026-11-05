<?php
// php-src ext/opcache/tests/match/001.phpt without its opcode dump
// (tests/phpt/ignored.txt): Match expression string jump table

function test($char) {
    return match ($char) {
        'a' => 'a',
        'b', 'c' => 'b, c',
        'd' => 'd',
        'e', 'f' => 'e, f',
        'g' => 'g',
        'h', 'i' => 'h, i',
    };
}

foreach (range('a', 'i') as $char) {
    var_dump(test($char));
}
