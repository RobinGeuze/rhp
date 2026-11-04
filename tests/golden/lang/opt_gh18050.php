<?php
// php-src ext/opcache/tests/gh18050.phpt without its opcode dump
// (tests/phpt/ignored.txt): GH-18050: Frameless calls break IN_ARRAY
// optimization

function test($v) {
    $ary = ['x', 'y'];
    var_dump(in_array($v, $ary));
    var_dump(in_array($v, $ary, false));
    var_dump(IN_ARRAY($v, $ary, true));

    if (in_array($v, $ary, true)) {
        echo "True\n";
    }
}
test('x');
test('z');

?>
