<?php
// get_defined_constants
define('MY_CONST', 42);
define('OTHER', 'x');
$all = get_defined_constants(true);
var_dump($all['user']);
var_dump($all['pcre']['PREG_BACKTRACK_LIMIT_ERROR'], $all['pcre']['PREG_SPLIT_NO_EMPTY']);
var_dump($all['Core']['E_ALL'], $all['Core']['PHP_INT_SIZE']);
var_dump(array_key_exists('PHP_EOL', $all['Core']));
var_dump(isset($all['standard']['PATHINFO_BASENAME']), isset($all['json']['JSON_PRETTY_PRINT']));
foreach ($all['pcre'] as $k => $v) {
    if (str_starts_with($k, 'PREG_') && str_ends_with($k, '_ERROR')) {
        echo "$k=$v\n";
    }
}
$flat = get_defined_constants();
var_dump($flat['MY_CONST'], $flat['OTHER'], $flat['E_WARNING']);
var_dump(array_key_last($flat));
var_dump(array_key_first($flat));

// ini
var_dump(ini_get('precision'), ini_get('nonexistent.setting'));
var_dump(ini_set('precision', '10'));
var_dump(ini_get('precision'));
var_dump(ini_set('precision', 14));
var_dump(ini_get('precision'));
var_dump(ini_set('nonexistent.setting', '1'));
var_dump(ini_set('allow_url_fopen', '0'));
var_dump(ini_get('allow_url_fopen'));
var_dump(ini_get('pcre.backtrack_limit'));
var_dump(ini_get_all('pcre'));
var_dump(ini_get_all('pcre', false));
var_dump(@ini_get_all('nope'));
var_dump(ini_get('error_log'));
ini_set('pcre.jit', '0');
var_dump(ini_get_all('pcre', false)['pcre.jit']);
ini_restore('pcre.jit');
var_dump(ini_get('pcre.jit'));
$all = ini_get_all(null, false);
var_dump(is_array($all), array_key_exists('memory_limit', $all), array_key_exists('display_errors', $all));

// timezone
var_dump(date_default_timezone_get());
var_dump(date_default_timezone_set('Europe/Amsterdam'));
var_dump(date_default_timezone_get());
var_dump(@date_default_timezone_set('Not/AZone'));
var_dump(date_default_timezone_get());
var_dump(date_default_timezone_set('utc'));
var_dump(date_default_timezone_get());
var_dump(date_default_timezone_set('UTC'));

// sleep family
var_dump(usleep(1));
var_dump(sleep(0));
try {
    usleep(-1);
} catch (ValueError $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
var_dump(error_log('to file', 3, '/dev/null'));
