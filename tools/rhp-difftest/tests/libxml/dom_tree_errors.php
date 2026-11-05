<?php
// libxml2 tree-module errors (XML_TREE_UNTERMINATED_ENTITY) from content
// given in escaped form: a warning naming the running function, or a
// LibXMLError record under libxml_use_internal_errors; serialization refusal
// messages of the XML classes; DOMDocument::save() to an unwritable path.
set_error_handler(function ($no, $msg) { echo "E$no: $msg\n"; return true; });
$d = new DOMDocument;
$e = $d->createElement("a");
$e->nodeValue = "p&q";
function f($e) { $e->nodeValue = "r&s"; }
f($e);
$d->createElement("a", "x&y");
$d->createElementNS("urn:x", "a", "x&y");
libxml_use_internal_errors(true);
$d->createElement("a", "x&y");
$e->nodeValue = "p&q";
foreach (libxml_get_errors() as $er) var_dump($er->level, $er->code, $er->line, $er->column, $er->message, $er->file);
libxml_use_internal_errors(false);
$t = $d->createElement("t", "a&amp;b&lt;&#x263A;&nbsp;");
echo $d->saveXML($t), "\n";
foreach ([new DOMDocument, new DOMElement("a"), new DOMXPath(new DOMDocument), new SimpleXMLElement("<a/>"), xml_parser_create()] as $o) {
    try { serialize($o); } catch (Exception $ex) { echo $ex->getMessage(), "\n"; }
}
foreach (['O:11:"DOMDocument":0:{}', 'O:8:"DOMXPath":0:{}'] as $s) {
    try { unserialize($s); } catch (Exception $ex) { echo $ex->getMessage(), "\n"; }
}
$d->loadXML('<a/>');
var_dump($d->save('/nonexistent-dir/x.xml'));
