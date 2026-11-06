<?php
// highlight_file and php_strip_whitespace skip a shebang line and keep
// scanning after __halt_compiler (no tokenizer special case).
$f = __DIR__ . '/highlight_file_scanner.inc';
echo highlight_file($f, true), "\n";
echo php_strip_whitespace($f), "\n";
echo highlight_string(file_get_contents($f), true), "\n";
// A shebang line without "\n" runs to the end: the file scans as nothing.
$t = sys_get_temp_dir() . '/rhp_hl_shebang_' . getmypid() . '.php';
foreach (["#!x\r<?php 1;", "#!x", "#!x\r\n<?php 2;"] as $src) {
    file_put_contents($t, $src);
    var_dump(php_strip_whitespace($t), highlight_file($t, true));
}
unlink($t);
