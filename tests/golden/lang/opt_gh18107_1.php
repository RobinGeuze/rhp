<?php
// php-src ext/opcache/tests/opt/gh18107_1.phpt without its opcode dump
// (tests/phpt/ignored.txt): GH-18107 (Opcache CFG jmp optimization with
// try-finally breaks the exception table)
ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));

if (!isset($badvar)) {
    throw new Exception("Should happen");
}
try {
    while (true) { }
} finally {
    throw new Exception("Should not happen");
}

?>
