<?php
// php-src ext/opcache/tests/opt/block_pass_007.phpt without its opcode dump
// (tests/phpt/ignored.txt): Block Pass 007: BOOL + TYPE_CHECK
$f = random_int(1, 2);

var_dump(!$f === true);
var_dump(!$f === false);
var_dump(!!$f === true);
var_dump(!!$f === false);
?>
