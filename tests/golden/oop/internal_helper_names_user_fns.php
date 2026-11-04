<?php
// php has no functions named like rhp's engine helpers, so user code may
// declare them; the built-in classes keep using their own helpers.
function __reflect(...$a) { return "user __reflect"; }
function __xml_fn(...$a) { return "user __xml_fn"; }
function __sxe(...$a) { return "user __sxe"; }
function __dom(...$a) { return "user __dom"; }
function __dom_throw($a, $b) { return "user __dom_throw"; }
echo (new ReflectionClass('ArrayObject'))->getName(), "\n";
echo (new ReflectionFunction('strlen'))->getNumberOfParameters(), "\n";
$p = xml_parser_create();
var_dump(xml_parse_into_struct($p, '<a>x</a>', $vals), $vals[0]['tag']);
echo simplexml_load_string('<a><b>t</b></a>')->b, "\n";
$d = new DOMDocument();
try {
    $d->createElement('1bad');
    echo "no exception\n";
} catch (DOMException $e) {
    echo get_class($e), ": ", $e->getMessage(), " (", $e->getCode(), ")\n";
}
echo __reflect(), ", ", __xml_fn(), ", ", __sxe(), ", ", __dom(), ", ", __dom_throw(1, 2), "\n";
echo call_user_func('__dom_throw', 1, 2), "\n";
var_dump(function_exists('__reflect'), function_exists('\__dom_throw'), is_callable('__sxe'));
