<?php
// memory_limit: the settings, the fatal error once a loop outgrows the
// limit, and a shutdown function after it. No memory sizes are printed
// (they depend on the platform).
ini_set('display_errors', '0');
register_shutdown_function(function () {
    $e = error_get_last();
    echo $e['type'], ' ', $e['line'], ' ', preg_replace('/\d+ bytes\)$/', 'N bytes)', $e['message']), "\n";
    var_dump(ini_get('memory_limit'), $GLOBALS['i'] > 1000);
});
var_dump(ini_set('memory_limit', '8M'));
var_dump(@ini_set('memory_limit', '1M'), ini_get('memory_limit'));
var_dump(@ini_set('memory_limit', 'x'), ini_get('memory_limit'));
var_dump(memory_get_usage() > 0, memory_get_usage(true) >= memory_get_usage());
var_dump(memory_get_peak_usage() >= memory_get_usage(), memory_get_peak_usage(true) >= memory_get_usage(true));
var_dump(memory_reset_peak_usage());
echo "filling\n";
$a = [];
for ($i = 0; ; $i++) {
    $a[] = str_repeat('x', 100);
}
