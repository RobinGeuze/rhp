<?php
// php-src ext/opcache/tests/opt/dce_009.phpt without its opcode dump
// (tests/phpt/ignored.txt): DCE 009: Foreach over empty array is a no-op
class Loop {
    const VALUES = [];
    public static function test() {
        echo "Start\n";
        $y = [];
        foreach ($y as $x) {
        }
        echo "Done\n";
    }
    public static function test2() {
        foreach (self::VALUES as $x) {
        }
    }
    public static function test3() {
        foreach ([] as $k => &$v) {
        }
    }
}
Loop::test();
Loop::test2();
Loop::test3();
?>
