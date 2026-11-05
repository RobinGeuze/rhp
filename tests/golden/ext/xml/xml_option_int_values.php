<?php
// xml_parser_set_option reads an int option with zval_get_long: a float
// that is no int warns (8.5), a numeric-leading string reads its number,
// an array is 0 or 1, an object warns and is 1.
set_error_handler(function ($no, $msg) { echo "Warning: $msg\n"; return true; });
$p = xml_parser_create();
foreach ([1e20, NAN, "12abc", "1e30", [], [1], 3.7, new stdClass] as $v) {
    var_dump(xml_parser_set_option($p, XML_OPTION_SKIP_TAGSTART, $v));
    var_dump(xml_parser_get_option($p, XML_OPTION_SKIP_TAGSTART));
}
