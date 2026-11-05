<?php
declare(strict_types=1);
// zpp_class_names.php from a strict_types file.
// zpp's "C" (get_class_vars' $class, simplexml_*'s $class_name) converts
// its argument to a string in either typing mode (an array warns, an
// object goes through __toString, null is "" unless the type allows
// null), then looks the class up.
set_error_handler(function ($no, $msg) {
    echo $no === E_DEPRECATED ? "Deprecated" : "Warning", ": $msg\n";
    return true;
});
class S extends SimpleXMLElement {}
class Named { function __toString(): string { return "S"; } }
class Plain {}
$xml = '<a><b>1</b></a>';
$dom = new DOMDocument;
$dom->loadXML($xml);
$show = function (string $label, callable $f) {
    echo $label, ": ";
    try {
        $r = $f();
        echo is_array($r) ? count($r) : get_class($r), "\n";
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
};
foreach ([1, null, 1.5, true, [], new Named, new Plain, "S", "Nope", "stdClass"] as $v) {
    echo "-- ", get_debug_type($v), "\n";
    $show("get_class_vars", fn() => get_class_vars($v));
    $show("simplexml_load_string", fn() => simplexml_load_string($xml, $v));
    $show("simplexml_import_dom", fn() => simplexml_import_dom($dom, $v));
}
// The class check comes before a later parameter's zpp.
$show("bad class, bad options", fn() => simplexml_load_string($xml, "Nope", "x"));
$show("class, bad options", fn() => simplexml_load_string($xml, "S", "x"));
// simplexml_import_dom checks $node before converting $class_name.
$show("bad node, array class", fn() => simplexml_import_dom(1, []));
