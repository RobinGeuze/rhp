<?php
// html_errors=1 formatting of displayed errors (php-cgi's default),
// switched on at runtime under the CLI. The output is buffered and the
// script path replaced before printing.
ob_start();
ini_set('html_errors', '1');
trigger_error('plain <notice> & "quotes"', E_USER_NOTICE);
trigger_error("it's a warning", E_USER_WARNING);
trigger_error('deprecated', E_USER_DEPRECATED);
$a = [];
echo $a['missing'];
echo intdiv(1, 1), "\n";
ini_set('html_errors', '0');
trigger_error('back to text', E_USER_NOTICE);
$out = ob_get_clean();
echo str_replace(__FILE__, 'FILE', $out);
echo "done\n";
