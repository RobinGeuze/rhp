<?php
// Dom\import_simplexml keeps libxml's serializer (php's document
// handlers); a Dom\XMLDocument's SimpleXML view uses the modern one.
$s = simplexml_load_string('<s><t>1</t></s>');
$m = Dom\import_simplexml($s);
var_dump($m->ownerDocument->saveXml());
var_dump($m->ownerDocument->saveXml($m->firstChild));
$c = clone $m->ownerDocument;
var_dump($c->saveXml());
$x = Dom\XMLDocument::createFromString('<r xmlns:a="urn:a"><a:q/></r>');
var_dump($x->saveXml());
var_dump(simplexml_import_dom($x)->asXML());
var_dump(simplexml_import_dom($x)->children('urn:a')[0]->asXML());
try {
    $x->importNode(null);
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
