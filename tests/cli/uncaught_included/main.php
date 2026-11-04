<?php
require __DIR__ . '/lib.inc';
function go() { return require __DIR__ . '/inc.inc'; }
$e = go();
echo basename($e->getFile()), ':', $e->getLine(), "\n";
echo str_replace(__DIR__, '', $e->getTraceAsString()), "\n";
$ev = eval('return new Exception("x");');
echo basename($ev->getFile()), ':', $ev->getLine(), "\n";
echo str_replace(__DIR__, '', $ev->getTraceAsString()), "\n";
(new Lib())->run();
