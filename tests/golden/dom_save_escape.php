<?php
// libxml2 2.9 serializer escaping: a document without an encoding goes
// through xmlEscapeEntities (hex char refs; an invalid byte drops the whole
// text node with a diagnostic), a node dump or an encoded document through
// content escaping (raw bytes, CR as &#13;).
$d = new DOMDocument;
$r = $d->createElement('r');
$d->appendChild($r);
$r->appendChild($d->createElement('a', "y\rz\tq"));
$r->appendChild($d->createElement('b', "ü>&<"));
$r->appendChild($d->createElement('c', "x\x01y"));
$r->appendChild($d->createElement('d', "\xC3("));
$r->appendChild($d->createElement('e', "after"));
$r->appendChild($d->createCDATASection("c\x01d"));
echo $d->saveXML();
echo $d->saveXML($r), "\n";
echo $d->saveXML($r->firstChild->firstChild), "\n";
$l = new DOMDocument; $l->loadXML('<?xml version="1.0" encoding="ISO-8859-1"?><r a="ü">ü&#13;</r>');
echo $l->saveXML(), $l->saveXML($l->documentElement), "\n";
echo $r->C14N(), "\n";
$d = new DOMDocument;
$r = $d->createElement('r');
$d->appendChild($r);
$r->appendChild($d->createElement('a', "x\x80y"));
$r->appendChild($d->createElement('b', "x\xF8y"));
$r->appendChild($d->createElement('c', "x\x01y"));
$r->appendChild($d->createElement('e', "x\xEF\xBF\xBEy"));
$r->appendChild($d->createElement('f', "x\xC3"));
$r->appendChild($d->createElement('g', "ok \xF0\x9F\x98\x80"));
echo $d->saveXML();
libxml_use_internal_errors(true);
echo $d->saveXML();
foreach (libxml_get_errors() as $e) {
    echo "$e->level $e->code $e->line $e->column ", json_encode($e->message), "\n";
}
$last = libxml_get_last_error();
echo $last->code, "\n";
libxml_clear_errors();
echo simplexml_import_dom($d)->asXML();
echo count(libxml_get_errors()), "\n";
