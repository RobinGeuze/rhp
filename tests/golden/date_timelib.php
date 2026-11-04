<?php
// The procedural date functions on timelib with php's embedded tz database.
set_error_handler(function ($no, $msg) { echo "[$no] $msg\n"; return true; });

// strtotime: grammar, DST gaps and overlaps, overflow.
$base = 1769867110;
$cases = [
    'last day of next month', '+1 week 2 days ago', '@1234567890.5', '@-1.5',
    'first monday of 2026-02', 'noon tomorrow', 'tomorrow noon', 'back of 7pm',
    'Sunday 00:00', '2026W53', '2008W27-3', '24:00', '2024-02-30', '2026-13-01',
    '+3 weekdays', '5 wednesdays ago', '+0 weekdays', 'Monday next week',
    '6.1.2009 13:00+01:00', '1 Jan 1600 US/Eastern', 'MSK', '10/Oct/2000:13:55:36 -0700',
    'Sat, 13 Mar 2010 11:22:33 +0100', '2026-10-25 02:30:00', '2026-03-29 02:30:00',
    '+99999999999999 years', '@9223372036854775807', '@99999999999999999999',
    'last sat of July 2008', 'third friday', 'midnight', 'yesterday 14:00', 'now',
    '+1 fortnight', 'first day of next month', 'last day of previous month',
    "noon\0junk", ' ', '', 'garbage', '2026-01-01 Europe/Paris', '2026-01-01 CEST',
];
foreach (['Europe/Amsterdam', 'America/New_York', 'Australia/Lord_Howe', 'Asia/Kolkata'] as $tz) {
    date_default_timezone_set($tz);
    echo "== $tz\n";
    foreach ($cases as $c) {
        echo json_encode($c), ' => ', var_export(strtotime($c, $base), true), "\n";
    }
}

// date() and gmdate(): every format character, history and far future.
$fmt = 'd D j l N S w z W F m M n t L o X x Y y a A B g G h H i s u v e I O P p T Z c r U \\Y\\\\';
foreach (['Europe/Amsterdam', 'America/St_Johns', 'Asia/Kathmandu', 'Pacific/Chatham', 'UTC'] as $tz) {
    date_default_timezone_set($tz);
    foreach ([-1500000000, -1, 0, 1700000000, 1792891799, 1792891800, 4102444800, 32503680000] as $ts) {
        echo $tz, ' ', $ts, ': ', date($fmt, $ts), "\n";
    }
}
echo gmdate($fmt, 1700000000), "\n";
var_dump(date('', 0), date('\\', 0), date('Y\\', 0));

// idate(), including php's -1 as an error.
date_default_timezone_set('Europe/Amsterdam');
foreach (str_split('BdhHiILmstUwWyYzZoNjnGg') as $c) {
    echo $c, '=', var_export(idate($c, 1700000000), true), ' ';
}
echo "\n";
var_dump(idate('y', -62198755200), idate('U', -1), idate('Q', 0), idate('YY', 0));

// mktime() and gmmktime(): normalisation, two-digit years, DST.
foreach ([[0, 0, 0, 1, 1, 2026], [25, 61, 61, 13, 32, 2025], [0, 0, 0, 3, 0, 2024],
          [2, 30, 0, 3, 29, 2026], [2, 30, 0, 10, 25, 2026], [0, 0, 0, 1, 1, 69],
          [0, 0, 0, 1, 1, 70], [0, 0, 0, 1, 1, 100], [0, 0, 0, 1, 1, 101],
          [-1, -1, -1, -1, -1, 1970]] as $a) {
    echo implode(',', $a), ' => ', mktime(...$a), ' ', gmmktime(...$a), "\n";
}
var_dump(mktime(0, 0, 0, 1, 1, PHP_INT_MAX), gmmktime(0, 0, 0, 1, 1, PHP_INT_MIN));
var_dump(checkdate(2, 29, 2024), checkdate(2, 29, 2023), checkdate(1, 1, 0),
    checkdate(12, 31, 32767), checkdate(1, 1, 32768), checkdate(13, 1, 2000));

// localtime() and getdate().
date_default_timezone_set('America/New_York');
var_dump(localtime(1720000000), localtime(1700000000, true), getdate(-1500000000));

// date_parse() and date_parse_from_format().
foreach (['2026-02-30 25:61', 'next monday 10:00 Europe/Paris', '2006-12-12 10:00:00.5 +1 week +1 hour',
          'last day of next month', 'first day of', '+3 weekdays', 'Tue, 01 Jan 2030 12:00:00 EST',
          '12:00 GMT+0230', '@1234567890', 'x', '', "now\0junk", '2026-01-01 Mars/Phobos',
          '10/Oct/2000:13:55:36 -0700', '1 Jan 1600 US/Eastern'] as $s) {
    echo json_encode($s), ': ', json_encode(date_parse($s)), "\n";
}
foreach ([['Y-m-d H:i', '2026-01-05 10:11 junk'], ['d/m/Y', '31/02/2026'], ['!D, d M Y', 'Mon, 05 Jan 2026'],
          ['U.u', '1234567890.123456'], ['Y-m-d e', '2026-01-01 Europe/Amsterdam'], ['H\h i\m', '10h 15m'],
          ['Y-m-d|', '2026-01-01'], ['z Y', '45 2026'], ['Y-m-d T', '2026-01-01 CEST'], ["Y\0m", '2026']] as [$f, $s]) {
    echo json_encode([$f, $s]), ': ', json_encode(date_parse_from_format($f, $s)), "\n";
}
try {
    date_parse_from_format('Y', "20\x0026");
} catch (ValueError $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}

// The default timezone and the timezone_* functions.
var_dump(date_default_timezone_set('europe/AMSTERDAM'), date_default_timezone_get());
var_dump(date_default_timezone_set('Mars/Phobos'), date_default_timezone_set(''), date_default_timezone_get());
var_dump(date_default_timezone_set("UTC\0junk"), date_default_timezone_get());
var_dump(timezone_version_get());
var_dump(count(timezone_identifiers_list()), count(timezone_identifiers_list(DateTimeZone::ALL_WITH_BC)));
var_dump(timezone_identifiers_list(DateTimeZone::PER_COUNTRY, 'NL'), timezone_identifiers_list(DateTimeZone::UTC));
var_dump(count(timezone_identifiers_list(DateTimeZone::EUROPE | DateTimeZone::ARCTIC)));
var_dump(timezone_identifiers_list(DateTimeZone::PER_COUNTRY, "N\0"), timezone_identifiers_list(DateTimeZone::PER_COUNTRY, 'nl'));
try {
    timezone_identifiers_list(DateTimeZone::PER_COUNTRY);
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
var_dump(timezone_name_from_abbr('CET'), timezone_name_from_abbr('', 3600, 0), timezone_name_from_abbr('', 3600, 1),
    timezone_name_from_abbr('EDT'), timezone_name_from_abbr('utc'), timezone_name_from_abbr('xyz'));
$abbr = timezone_abbreviations_list();
var_dump(count($abbr), array_key_first($abbr), $abbr['cest'][0], $abbr['utc']);
ini_set('date.timezone', 'Asia/Tokyo');
date_default_timezone_set('UTC');
var_dump(date_default_timezone_get(), ini_set('date.timezone', 'Nowhere/Land'), ini_get('date.timezone'));

// Constants, the deprecated ones through the error handler.
var_dump(DATE_ISO8601_EXPANDED, DATE_RFC3339_EXTENDED, DATE_RFC7231, SUNFUNCS_RET_DOUBLE);
echo date(DATE_ISO8601_EXPANDED, -62198755200), ' ', date(DATE_RFC3339_EXTENDED, 1700000000), "\n";
