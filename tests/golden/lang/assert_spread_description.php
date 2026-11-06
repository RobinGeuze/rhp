<?php
// assert()'s added description goes after a spread argument.
ob_start(function ($s) { return str_replace(__FILE__, 'F', $s); });
eval('echo "never\n";
$a = [false];
assert(...$a);');
