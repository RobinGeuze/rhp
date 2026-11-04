<?php
// Exceptions thrown from libxml2 callbacks in the middle of a call.
set_error_handler(function ($no, $msg) { throw new Exception($msg); });
for ($i = 0; $i < 100; $i++) {
    try {
        simplexml_load_string('<a><b></a>');
    } catch (Exception $e) {
    }
    try {
        (new DOMDocument())->loadXML('<a><b></a>');
    } catch (Exception $e) {
    }
    $doc = new DOMDocument();
    $doc->loadXML('<r><a>1</a><a>2</a></r>');
    $xp = new DOMXPath($doc);
    $xp->registerNamespace('php', 'http://php.net/xpath');
    $xp->registerPhpFunctions();
    try {
        $xp->query('//a[php:function("intdiv", 1, 0)]');
    } catch (Throwable $e) {
    }
    try {
        $r = XMLReader::fromString('<a><b></a>');
        while ($r->read()) {
        }
    } catch (Exception $e) {
    }
}
restore_error_handler();
echo "ok\n";
