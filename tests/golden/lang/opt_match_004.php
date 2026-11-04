<?php
// php-src ext/opcache/tests/match/004.phpt without its opcode dump
// (tests/phpt/ignored.txt): Match expression mixed jump table

function test($value) {
    return match ($value) {
        1 => '1 int',
        '1' => '1 string',
        2 => '2 int',
        '2' => '2 string',
        3 => '3 int',
        '3' => '3 string',
        4 => '4 int',
        '4' => '4 string',
        5 => '5 int',
        '5' => '5 string',
        default => 'default',
    };
}

foreach (range(0, 6) as $number) {
    var_dump(test($number));
    var_dump(test((string) $number));
}

?>
