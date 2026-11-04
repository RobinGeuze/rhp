<?php
// XMLReader on malformed documents: libxml2's reader reports the error on
// the read that reaches it, in words that differ between releases.
set_error_handler(function ($no, $str) { echo "W[$no]: ", str_replace(getcwd(), "CWD", $str), "\n"; return true; });
foreach (['<a><b>1</b>', '<a><b></a>', '<a>&foo;</a>', '<a x="1" x="2"/>', '<a/><b/>'] as $doc) {
    $r = XMLReader::XML($doc);
    while (@$r->read()) { echo $r->nodeType, $r->name, ' '; }
    echo "\n";
    $r = XMLReader::XML($doc);
    while ($r->read()) { echo $r->nodeType, $r->name, ' '; }
    echo "\n";
}
libxml_use_internal_errors(true);
$r = XMLReader::XML('<a><b></a>');
while ($r->read());
foreach (libxml_get_errors() as $e) { echo $e->level, ' ', $e->code, ' ', $e->line, ' ', trim($e->message), "\n"; }
libxml_clear_errors();
$r = XMLReader::XML('<a><b/></a>');
$r->setSchema(null);
var_dump($r->setRelaxNGSchemaSource('<element name="a" xmlns="http://relaxng.org/ns/structure/1.0"><empty/></element>'));
while ($r->read());
var_dump($r->isValid());
foreach (libxml_get_errors() as $e) { echo $e->level, ' ', $e->code, ' ', trim($e->message), "\n"; }
