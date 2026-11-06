<?php
// Equal tokens on one line stay independent values.
$t = token_get_all('<?php $a = $b + $c;');
$t[2][1] = "\t";
$t[4][1] = '  ';
foreach ($t as &$tok) {
    if (is_array($tok) && $tok[0] === T_WHITESPACE) {
        $tok[2] = 9;
    }
}
unset($tok);
foreach ($t as $tok) {
    echo is_array($tok) ? token_name($tok[0]) . ' ' . json_encode($tok[1]) . " $tok[2]" : $tok, "\n";
}

// A big file stays within a generous bound (php: about 170 bytes a token).
$src = str_repeat("<?php \$a = foo(\$b, \"x\") + 1; // c\n", 5000);
$m = memory_get_usage();
$t = token_get_all($src);
var_dump(count($t), memory_get_usage() - $m < 300 * count($t));
