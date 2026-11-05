<?php
// The xml_* functions' arguments (zpp `l`, typed parameters, named
// arguments), XMLParser comparison, and xml_set_object's handler swap.
set_error_handler(function ($no, $str) { echo "E$no: $str\n"; return true; });
function t(callable $f) {
    try { var_dump($f()); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}

echo "-- comparison\n";
$a = xml_parser_create();
$b = xml_parser_create();
var_dump($a == $b, $a <=> $b, $a == $a, $a != $b, $a < $b);

echo "-- int parameters\n";
foreach (['abc', 1.5, '1.5', ' 2', '2 ', '1abc', '0x1A', null, true, 2.0, 1e30, NAN, [], new stdClass] as $v) {
    t(fn() => xml_error_string($v));
}
$p = xml_parser_create();
t(fn() => xml_parser_set_option($p, "x", 1));
t(fn() => xml_parser_set_option($p, 99, 1));
t(fn() => xml_parser_set_option($p, "4", 1));
t(fn() => xml_parser_get_option($p, "4"));
t(fn() => xml_parser_get_option($p, 1.5));
t(fn() => xml_parser_get_option($p, null));

echo "-- named arguments\n";
$p = xml_parser_create_ns(encoding: 'UTF-8', separator: '#');
var_dump(xml_parser_set_option(parser: $p, option: XML_OPTION_CASE_FOLDING, value: 0));
var_dump(xml_parser_get_option(parser: $p, option: XML_OPTION_CASE_FOLDING));
xml_set_element_handler(parser: $p, start_handler: function ($p, $n, $a) { echo "S $n\n"; }, end_handler: null);
xml_set_character_data_handler(parser: $p, handler: function ($p, $d) { echo "C $d\n"; });
var_dump(xml_parse(parser: $p, data: '<a xmlns="u">t</a>', is_final: true));
var_dump(xml_get_error_code(parser: $p), xml_error_string(error_code: 4));

echo "-- typed parameters\n";
t(fn() => xml_parse(xml_parser_create(), []));
t(fn() => xml_parse(xml_parser_create(), null));
t(fn() => xml_parse("p", "x"));
t(fn() => xml_parser_create([]));
t(fn() => xml_set_element_handler("p", "x", "y"));
t(fn() => xml_set_character_data_handler(xml_parser_create(), []));
t(fn() => xml_parser_free("x"));
t(fn() => xml_set_object("x", new stdClass));

echo "-- xml_set_object\n";
class H {
    function s($p, $n, $a) { echo "H::s $n\n"; }
    function e($p, $n) { echo "H::e $n\n"; }
}
class H2 {
    function s($p, $n, $a) { echo "H2::s $n\n"; }
}
$p = xml_parser_create();
xml_set_object($p, new H);
xml_set_element_handler($p, 's', 'e');
t(fn() => xml_set_object($p, new H2));
var_dump(xml_parse($p, '<a>y</a>', true));
