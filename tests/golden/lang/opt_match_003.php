<?php
// php-src ext/opcache/tests/match/003.phpt without its opcode dump
// (tests/phpt/ignored.txt): Match expression long jump table

function test($char) {
    return match ($char) {
        1 => '1',
        2, 3 => '2, 3',
        4 => '4',
        5, 6 => '5, 6',
        7 => '7',
        8, 9 => '8, 9',
        default => 'default'
    };
}

foreach (range(0, 10) as $char) {
    var_dump(test($char));
}

?>
