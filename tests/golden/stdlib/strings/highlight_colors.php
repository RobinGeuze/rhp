<?php
// highlight_string() with the highlight.* colours changed at run time, and
// php_strip_whitespace()'s heredoc handling.
ini_set('highlight.keyword', '#123456');
ini_set('highlight.string', 'red');
ini_set('highlight.comment', 'green');
$code = <<<'CODE'
<html><?php /* c */ echo "a $b {$c->d}", 'x' . PHP_EOL; // e
$h = <<<EOT
  x $y
  EOT;
function f(int $a = 1.5e3, ?string $s = null): void { return; } ?>
tail <?= 1 ?>
CODE;
var_dump(highlight_string($code, true));
highlight_string("<?php\n\tif (1 < 2 && \"&\") {}\n");
echo "\n";
$f = sys_get_temp_dir() . '/rhp_strip_' . getmypid() . '.php';
file_put_contents($f, $code);
var_dump(php_strip_whitespace($f));
unlink($f);
// Directives with equal values are one colour (php compares the interned
// strings' addresses).
ini_set('highlight.keyword', '#0000BB');
ini_set('highlight.default', '#0000BB');
ini_set('highlight.html', '#0000BB');
var_dump(highlight_string('x<?php $a = 1; ?>y', true));
