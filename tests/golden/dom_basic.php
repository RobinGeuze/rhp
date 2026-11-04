<?php
// DOMDocument parsing, navigation, building and libxml2-exact serialization.
$d = new DOMDocument;
$d->loadXML('<a b="1"><c>t</c></a>');
echo $d->documentElement->getAttribute('b'), $d->getElementsByTagName('c')->item(0)->textContent, "\n";
$d->formatOutput = true;
echo $d->saveXML();

var_dump(extension_loaded('dom'), extension_loaded('libxml'), extension_loaded('SimpleXML'), extension_loaded('xmlwriter'));
var_dump(class_exists('DOMDocument'), class_exists('DOMXPath'), class_exists('SimpleXMLElement'), class_exists('XMLWriter'), class_exists('LibXMLError'));

// whitespace handling and formatOutput
$src = "<a>\n  <b>t</b>  <c/>\n  <!-- c -->\n  <d>  </d>\n<e><f/>x</e><g><![CDATA[ <&> ]]><h/></g></a>";
foreach ([true, false] as $pw) {
    $d = new DOMDocument;
    $d->preserveWhiteSpace = $pw;
    $d->loadXML($src);
    echo $d->saveXML();
    $d->formatOutput = true;
    echo $d->saveXML();
    echo $d->saveXML($d->documentElement), "\n";
}

// escaping: no encoding declared vs UTF-8 (control characters and
// non-ASCII text without an encoding: tools/rhp-difftest/tests/libxml.rs)
$d = new DOMDocument;
$d->loadXML("<?xml version=\"1.0\" encoding=\"UTF-8\"?><a x='\u{e9}'>\u{e9}</a>");
echo $d->saveXML();
$d->loadXML("<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a x='\xe9'>\xe9</a>");
echo bin2hex($d->saveXML()), "\n", $d->saveXML($d->documentElement), "\n";
$d->loadXML('<?pi data?><!-- top --><!DOCTYPE a [<!ENTITY e "v">]><a>&e;&amp;<?p2 x ?></a><!-- end -->');
echo $d->saveXML();
var_dump($d->documentElement->textContent, $d->doctype->name, $d->doctype->internalSubset);
$d->loadXML('<a xmlns="urn:u" xmlns:x="urn:v" b="1" x:c="2"><x:d/></a>', LIBXML_NOBLANKS);
echo $d->saveXML();
$r = $d->documentElement;
var_dump($r->namespaceURI, $r->firstChild->prefix, $r->firstChild->localName, $r->firstChild->namespaceURI, $r->getAttribute('x:c'), $r->getAttributeNS('urn:v', 'c'), $r->getAttribute('xmlns'), $r->hasAttribute('xmlns:x'), $r->lookupNamespaceURI('x'), $r->lookupPrefix('urn:v'));

// building
$d = new DOMDocument('1.0', 'UTF-8');
$root = $d->createElement('root');
$d->appendChild($root);
$root->setAttribute('name', 'x & "y" <z>');
$root->appendChild($d->createElement('child', 'a &amp; b'));
$root->appendChild($d->createTextNode('t<x>'));
$root->appendChild($d->createComment('cm'));
$root->appendChild($d->createCDATASection('cd]]>x'));
$root->appendChild($d->createProcessingInstruction('pi', 'dd'));
$e = $d->createElement('e');
$root->insertBefore($e, $root->firstChild);
echo $d->saveXML();
$d->formatOutput = true;
$x = $d->createElement('x');
$root->replaceChild($x, $e);
$x->appendChild($d->createElement('y'))->setAttribute('k', 'v');
$root->removeChild($root->lastChild);
echo $d->saveXML();
var_dump($root->childNodes->length, $root->firstChild->nodeName, $root->lastChild->nodeType, $x->parentNode === $root, $x->ownerDocument === $d);
foreach ($root->childNodes as $i => $n) echo $i, ':', get_class($n), ':', $n->nodeName, "\n";
foreach ($root->attributes as $k => $a) echo "$k=", $a->value, "\n";
var_dump($root->getAttribute('nope'), $root->hasAttribute('name'), $root->removeAttribute('name'), $root->hasAttributes());

// node values
$el = $d->createElement('v');
$el->nodeValue = 'p &amp; q'; echo $d->saveXML($el), "\n";
$el->textContent = 'p &amp; q'; echo $d->saveXML($el), "\n";
var_dump($d->saveXML($d->createElement('w', '')), $d->saveXML($d->createElement('w')));

// live node lists
$d = new DOMDocument;
$d->loadXML('<r><i n="1"/><i n="2"/><j><i n="3"/></j></r>');
$list = $d->getElementsByTagName('i');
var_dump($list->length, count($list));
$d->documentElement->appendChild($d->createElement('i'));
var_dump($list->length, $list->item(3)->nodeName, $list->item(9));
foreach ($d->getElementsByTagName('*') as $n) echo $n->nodeName, ' ';
echo "\n";

// exceptions
try { $d->createElement('1bad'); } catch (DOMException $ex) { echo get_class($ex), ': ', $ex->getMessage(), ' ', $ex->getCode(), "\n"; }
try { $d->documentElement->removeChild($d->createElement('z')); } catch (DOMException $ex) { echo $ex->getMessage(), ' ', $ex->getCode(), "\n"; }
$other = new DOMDocument;
try { $d->documentElement->appendChild($other->createElement('z')); } catch (DOMException $ex) { echo $ex->getMessage(), ' ', $ex->getCode(), "\n"; }
try { $d->loadXML(''); } catch (ValueError $ex) { echo get_class($ex), ': ', $ex->getMessage(), "\n"; }

// import / clone / detached nodes
$n = new DOMElement('free', 'val');
var_dump($n->ownerDocument, $n->textContent);
$d->documentElement->appendChild($n);
var_dump($n->ownerDocument === $d);
$imp = $other->importNode($d->documentElement, true);
$other->appendChild($imp);
echo $other->saveXML();
$cl = $d->documentElement->cloneNode(false);
var_dump($cl->parentNode, $d->saveXML($cl));
var_dump($n->getNodePath(), $n->firstChild->getNodePath(), $d->documentElement->getLineNo());

// fragments
$f = $d->createDocumentFragment();
$f->appendXML('<k>1</k><k>2</k>');
$d->documentElement->appendChild($f);
echo $d->saveXML($d->documentElement), "\n";
var_dump($d->documentElement->childElementCount, $d->documentElement->lastElementChild->nodeName);
