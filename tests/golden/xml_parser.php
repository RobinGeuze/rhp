<?php
// ext/xml: the expat-compatible SAX parser php builds on libxml2.
set_error_handler(function ($no, $str) { echo "W[$no]: ", str_replace(getcwd(), "CWD", $str), "\n"; return true; });
function run($xml, $opts = [], $chunks = null) {
    $p = xml_parser_create();
    foreach ($opts as $k => $v) xml_parser_set_option($p, $k, $v);
    xml_set_element_handler($p, function ($p, $n, $a) { echo "S:$n ", json_encode($a), "\n"; }, function ($p, $n) { echo "E:$n\n"; });
    xml_set_character_data_handler($p, function ($p, $d) { echo "C:", json_encode($d), "\n"; });
    xml_set_processing_instruction_handler($p, function ($p, $t, $d) { echo "PI:$t ", json_encode($d), "\n"; });
    xml_set_default_handler($p, function ($p, $d) { echo "D:", json_encode($d), "\n"; });
    if ($chunks) { foreach ($chunks as $i => $c) { $r = xml_parse($p, $c, $i == count($chunks) - 1); echo "r=$r\n"; } }
    else { $r = xml_parse($p, $xml, true); echo "r=$r\n"; }
    if (!$r) echo "err ", xml_get_error_code($p), ' ', xml_error_string(xml_get_error_code($p)), ' L', xml_get_current_line_number($p), ' C', xml_get_current_column_number($p), ' B', xml_get_current_byte_index($p), "\n";
    xml_parser_free($p);
}
run("<?xml version=\"1.0\"?>\n<root a=\"1\" B='x'>\n  <item>x\ny &amp; z &#65;</item>\n  <!-- c --><?pi data?><![CDATA[cd <x>]]><e/>\n</root>\n");
run("<a>\xc3\xa9t\xc3\xa9</a>");
run("<a><b>t</b>", [], ["<a>", "<b>t</b>", "</a>"]);
run("<a><b></a>");
run("<a>x</a><b/>");
run("<a/>", [XML_OPTION_CASE_FOLDING => 0]);
run("<root>\n <x> </x>\n</root>", [XML_OPTION_SKIP_WHITE => 1, XML_OPTION_CASE_FOLDING => false]);
$p = xml_parser_create();
xml_parse_into_struct($p, "<para><note>simple note</note><b a='1'>x<i>y</i>z</b></para>", $vals, $index);
var_dump($vals, $index);
var_dump(xml_parser_get_option($p, XML_OPTION_CASE_FOLDING), xml_parser_get_option($p, XML_OPTION_TARGET_ENCODING));
var_dump(xml_error_string(4), xml_error_string(76), XML_ERROR_SYNTAX, XML_SAX_IMPL);
$p = xml_parser_create_ns();
xml_set_element_handler($p, function ($p, $n, $a) { echo "S:$n ", json_encode($a), "\n"; }, function ($p, $n) { echo "E:$n\n"; });
xml_parse($p, '<a xmlns="urn:x" xmlns:p="urn:p"><p:b p:c="1"/></a>', true);
class H { function s($p, $n, $a) { echo "obj S:$n\n"; } function e($p, $n) { echo "obj E:$n\n"; } }
$p = xml_parser_create(); $h = new H; xml_set_object($p, $h); xml_set_element_handler($p, 's', 'e'); xml_parse($p, '<q/>', true);
var_dump(get_class($p));
