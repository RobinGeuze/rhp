<?php
// mb_ereg* corners the phpt corpus leaves open.
var_dump(function_exists('mb_ereg'), mb_regex_encoding());

// Callback: unmatched groups are "", named groups follow the numbered ones.
echo mb_ereg_replace_callback('(?<a>x)|(?<b>y)', function ($m) {
    return json_encode($m);
}, 'xy'), "\n";
echo @mb_ereg_replace_callback('a', fn ($m) => [1], 'bab'), "\n";

// Limits.
var_dump(mb_split(',', 'a,b,c', 0), mb_split(',', 'a,b,c', 1), mb_split(',', 'a,b,c', 2));
var_dump(mb_split(',', ''));

// Options: the previous ones come back; letters add to eregi's case flag.
var_dump(mb_regex_set_options('ix'), mb_regex_set_options('z'), mb_regex_set_options());
mb_regex_set_options('pr');
var_dump(mb_eregi_replace('A B', '-', 'a b ab', 'x'));
var_dump(mb_ereg_replace('a', '\k<1>\k<x>\\', 'a'));

// Another encoding.
var_dump(mb_regex_encoding('EUC-JP'), mb_regex_encoding());
var_dump(mb_ereg("\xa4\xa2", "x\xa4\xa2y", $m), bin2hex($m[0]));
var_dump(mb_ereg('a', "\xff\xff"));
var_dump(@mb_ereg("\xff", 'a'));
var_dump(mb_regex_encoding('utf8'), mb_regex_encoding());

// The search pattern recompiled with other options is gone.
mb_ereg_search_init('abc', 'b');
mb_ereg_replace('b', 'x', 'abc', 'i');
try {
    mb_ereg_search();
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
var_dump(mb_ereg_search_getregs());
mb_ereg_search_init('aXbX', 'X');
var_dump(mb_ereg_search_pos(), mb_ereg_search_getregs(), mb_ereg_search_pos(), mb_ereg_search_pos());
try {
    mb_ereg_match('a', 'a', 'q');
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}

// The limits parse as quantities and warn about what is no number.
set_error_handler(function ($no, $msg) {
    echo "E$no: $msg\n";
    return true;
});
var_dump(ini_set('mbstring.regex_retry_limit', 'abc'), ini_get('mbstring.regex_retry_limit'));
