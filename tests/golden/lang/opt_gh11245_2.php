<?php
// php-src ext/opcache/tests/opt/gh11245_2.phpt without its opcode dump
// (tests/phpt/ignored.txt): GH-11245: In some specific cases SWITCH with one
// default statement will cause segfault (TMP variation)
ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
class X {
    // Chosen to test for a memory leak.
    static $prop = "aa";
}
switch (++X::$prop) {
    default:
        if (empty($xx)) {return;}
}
?>
