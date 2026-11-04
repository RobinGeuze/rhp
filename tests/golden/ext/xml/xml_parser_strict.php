<?php
declare(strict_types=1);
// zpp in strict mode: the caller's mode reaches the xml_* builtins through
// their PHP wrappers.
function t(callable $f) {
    try { var_dump($f()); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}
class O { function h($p, $d) {} }
$p = xml_parser_create();
@xml_set_object($p, new O);
foreach (['1', 1.0, true, null, 1] as $v) {
    t(fn() => xml_error_string($v));
}
t(fn() => xml_parser_get_option($p, '1'));
t(fn() => xml_parse($p, 5));
t(fn() => xml_parser_create(5));
t(fn() => xml_set_character_data_handler($p, 5));
t(fn() => @xml_set_character_data_handler($p, 'h'));
