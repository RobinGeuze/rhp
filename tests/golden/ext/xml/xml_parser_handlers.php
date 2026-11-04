<?php
// xml_parse handlers: method-name handlers via xml_set_object, and the
// stack trace of an exception thrown inside a handler (the handler is
// called from xml_parse itself, with no engine frames in between).
class H {
    function s($p, $n, $a) { echo "S $n ", json_encode($a), "\n"; }
    function e($p, $n) { echo "E $n\n"; }
    function pi2($p, $t, $d) { echo "PI $t $d\n"; }
    function def($p, $d) { echo "D ", json_encode($d), "\n"; }
}
$p = xml_parser_create();
@xml_set_object($p, new H);
@xml_set_element_handler($p, 's', 'e');
@xml_set_processing_instruction_handler($p, 'pi2');
@xml_set_default_handler($p, 'def');
xml_parse($p, '<?xml version="1.0"?><!DOCTYPE a><a><?x y?><!--c--></a>', true);
$p = xml_parser_create();
function boom($p, $n, $a) { throw new RuntimeException("boom $n"); }
xml_set_element_handler($p, "boom", null);
try {
    xml_parse($p, '<q/>', true);
} catch (Exception $e) {
    echo $e->getMessage(), "\n";
    foreach ($e->getTrace() as $i => $f) {
        echo $i, ' ', $f['function'], ' ', isset($f['file']) ? basename($f['file']) . ':' . $f['line'] : '[internal]', "\n";
    }
}
function thrower($p, $d) { throw new LogicException("x $d"); }
$p = xml_parser_create();
xml_set_character_data_handler($p, 'thrower');
try {
    xml_parse($p, '<a>t</a>', true);
} catch (LogicException $e) {
    foreach ($e->getTrace() as $i => $f) {
        echo $i, ' ', $f['function'], ' ', isset($f['file']) ? basename($f['file']) . ':' . $f['line'] : '[internal]', "\n";
    }
}
