<?php
// Simple interpolation `"$a[key]"`: an unquoted key is a string literal
// (never a constant); numeric keys follow php's array-key normalization.
set_error_handler(function ($no, $msg, $file, $line) {
    echo "[$no] $msg (line $line)\n";
    return true;
});
define('a', 'const');
$arr = ['a' => 'lit', 'const' => 'viaconst', 3 => 'three', -1 => 'neg', '01' => 'zero-one', 'A_B' => 'u'];
echo "$arr[a] {$arr[a]} $arr[3] $arr[-1] $arr[01] $arr[A_B]\n";
echo "missing: [$arr[zz]]\n";
$x = 'global x';
function show() { return "$GLOBALS[x]"; }
echo show(), "\n";
echo <<<EOT
heredoc: $arr[a] $GLOBALS[x]
EOT;
echo "\n";
