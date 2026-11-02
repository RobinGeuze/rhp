<?php
// XMLReader: libxml2's pull reader (node sequence, attributes, inner/outer XML).
set_error_handler(function ($no, $str) { echo "W[$no]: ", str_replace(getcwd(), "CWD", $str), "\n"; return true; });
$r = new XMLReader();
var_dump($r->XML("<?xml version=\"1.0\"?>\n<!DOCTYPE root><root xmlns:p=\"urn:p\" a=\"1\" p:b=\"2\">\n  <item id=\"x\">one &amp; two</item>\n  <empty/><p:n>t</p:n><!-- c --><?pi d?><![CDATA[cd]]>\n</root>"));
while ($r->read()) {
    printf("%d %-18s d=%d empty=%d hasval=%d val=%s attrs=%d ns=%s prefix=%s local=%s\n", $r->nodeType, $r->name, $r->depth, $r->isEmptyElement, $r->hasValue, json_encode($r->value), $r->attributeCount, json_encode($r->namespaceURI), $r->prefix, $r->localName);
    if ($r->nodeType == XMLReader::ELEMENT && $r->hasAttributes) {
        var_dump($r->getAttribute('a'), $r->getAttribute('nope'), $r->getAttributeNo(0), $r->getAttributeNs('b', 'urn:p'));
        while ($r->moveToNextAttribute()) { echo "  attr ", $r->nodeType, ' ', $r->name, '=', $r->value, ' d=', $r->depth, "\n"; }
        $r->moveToElement();
        echo "  back ", $r->name, "\n";
    }
    if ($r->name === 'item' && $r->nodeType == 1) { var_dump($r->readInnerXml(), $r->readOuterXml(), $r->readString()); }
}
var_dump($r->read(), $r->nodeType, $r->name);
$r->close();
$r = XMLReader::XML('<a><b>1</b><c>2</c><b>3</b></a>');
$r->read();
$r->read();
echo $r->name, "\n";
$r->next(); echo $r->name, "\n";
$r->next('b'); echo $r->name, ' ', $r->readString(), "\n";
var_dump($r->next());
$r = XMLReader::XML('<a><b>1</b>');
while (@$r->read()) { echo $r->nodeType, $r->name, ' '; }
echo "\n";
$r = XMLReader::XML('<a><b>1</b>');
while ($r->read()) { echo $r->nodeType, $r->name, ' '; }
echo "\n";
$r = XMLReader::XML('<r><x a="1"><y/></x></r>');
$r->read(); $r->read();
$n = $r->expand();
var_dump(get_class($n), $n->nodeName, $n->getAttribute('a'), $n->childNodes->length);
var_dump(XMLReader::ELEMENT, XMLReader::END_ELEMENT, XMLReader::SIGNIFICANT_WHITESPACE, XMLReader::SUBST_ENTITIES);
