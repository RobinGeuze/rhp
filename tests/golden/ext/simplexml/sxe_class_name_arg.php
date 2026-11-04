<?php
// simplexml's $class_name is a class argument: it autoloads, an autoloader's
// exception propagates, and it must name SimpleXMLElement or a subclass.
spl_autoload_register(function ($c) {
    echo "autoload $c\n";
    if ($c === 'App\Xml\El') {
        eval('namespace App\Xml; class El extends \SimpleXMLElement {}');
        return;
    }
    if ($c === 'Boom') {
        throw new Exception("cannot load $c");
    }
});
function show(callable $f) {
    try {
        $r = $f();
        echo is_object($r) ? get_class($r) : var_export($r, true), "\n";
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
show(fn() => simplexml_load_string('<a/>', 'App\Xml\El'));
show(fn() => simplexml_load_string('<a/>', '\App\Xml\El'));
show(fn() => simplexml_load_string('<a/>', 'simplexmliterator'));
show(fn() => simplexml_load_string('<a/>', null));
show(fn() => simplexml_load_string('<a/>', 'Boom'));
show(fn() => simplexml_load_string('<a/>', 'Missing'));
show(fn() => simplexml_load_string('<a/>', 'stdClass'));
// The class is checked before the document is parsed or read.
show(fn() => simplexml_load_string('<<not xml', 'stdClass'));
show(fn() => simplexml_load_file('/nonexistent/doc.xml', 'stdClass'));
$d = new DOMDocument();
$d->loadXML('<r a="1"><c/></r>');
show(fn() => simplexml_import_dom($d, 'App\Xml\El'));
show(fn() => simplexml_import_dom($d, 'stdClass'));
show(fn() => simplexml_import_dom($d, 'Boom'));
show(fn() => simplexml_import_dom(new stdClass()));
set_error_handler(function ($no, $msg) { echo "warning: $msg\n"; return true; });
show(fn() => simplexml_import_dom($d->documentElement->getAttributeNode('a')));
