<?php
// php-src ext/opcache/tests/opt/sccp_032.phpt without its opcode dump
// (tests/phpt/ignored.txt): SCCP 032: Yield from optimizations
function test(): Generator {
    $result = yield from [];
    $a = [];
    yield from $a;
    yield $result;
    $a[] = 3;
    yield from $a;
}
foreach (test() as $x) {
    var_export($x);
    echo "\n";
}
?>
