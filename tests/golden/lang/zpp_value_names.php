<?php
// The value a "…, X given" message names: zend_zval_value_name (the class
// of an object, cut at the NUL of an anonymous class's name; true/false for
// a bool) in zpp-style errors, zend_zval_type_name ("bool") in the few
// messages php builds with it.
$o = new ArrayObject([]);
$anon = new class {};
foreach ([
    fn() => strlen($anon),
    fn() => array_sum(true),
    fn() => count(true),
    fn() => count(new stdClass),
    fn() => max(false),
    fn() => pclose($o),
    fn() => proc_close(true),
    fn() => pcntl_signal(SIGUSR1, $o),
    fn() => pcntl_sigprocmask(SIG_BLOCK, [$o]),
    fn() => unserialize("i:1;", ["allowed_classes" => [true]]),
    fn() => unserialize("i:1;", ["allowed_classes" => $o]),
    fn() => unserialize("i:1;", ["allowed_classes" => [$o]]),
    fn() => libxml_set_streams_context(true),
    fn() => iconv_mime_encode("a", "b", true),
    fn() => (new XMLWriter)->toStream(false),
    fn() => xml_parse(true, "x"),
    fn() => zip_read(true),
    fn() => (new DOMDocument)->createElement("a")->C14N(false, false, ["query" => true]),
    fn() => (new DOMDocument)->createElement("a")->append(true),
    fn() => new Exception("m", 1, true),
    fn() => $anon + 1,
    fn() => true(),
] as $i => $f) {
    echo $i, ": ";
    try {
        var_dump(@$f());
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
