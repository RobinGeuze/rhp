<?php
// highlight_file and php_strip_whitespace skip a shebang line and keep
// scanning after __halt_compiler (no tokenizer special case).
$f = __DIR__ . '/highlight_file_scanner.inc';
echo highlight_file($f, true), "\n";
echo php_strip_whitespace($f), "\n";
echo highlight_string(file_get_contents($f), true), "\n";
