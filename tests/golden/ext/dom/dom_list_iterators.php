<?php
$doc = new DOMDocument; $doc->loadXML('<a x="1" y="2"><b/><c/><d/><e/></a>');
$a = $doc->documentElement;
foreach ($a->childNodes as $k => $n) { echo $k, $n->nodeName, ' '; if ($n->nodeName === 'c') $a->removeChild($n); }
echo "\n", $doc->saveXML();
foreach ($a->attributes as $k => $n) echo $k, '=', $n->value, ' ';
echo "\n";
$it = $a->childNodes->getIterator(); $it->rewind(); $it->rewind();
var_dump(spl_object_id($it), $it->key(), $it->current()->nodeName); $it->next(); var_dump($it->key());
try { $it->rewind(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
foreach ($a->childNodes as $x) foreach ($a->childNodes as $y) echo $x->nodeName, $y->nodeName, ' ';
echo "\n";
try { foreach ($a->childNodes as &$r) {} } catch (Error $e) { echo $e->getMessage(), "\n"; }
$xp = new DOMXPath($doc); foreach ($xp->query('//*') as $k => $n) echo $k, $n->nodeName, ' ';
echo "\n";
foreach ($doc->getElementsByTagName('*') as $k => $n) echo $k, $n->nodeName, ' ';
echo "\n";
$m = Dom\XMLDocument::createFromString('<!DOCTYPE r [<!ENTITY e "x"><!NOTATION n SYSTEM "y">]><r a="1" b:c="2" xmlns:b="u"><s/>t<u/></r>');
foreach ($m->doctype->entities as $k => $v) echo $k, get_class($v), ' ';
foreach ($m->doctype->notations as $k => $v) echo $k, get_class($v), ' ';
foreach ($m->documentElement->attributes as $k => $v) echo $k, '=', $v->value, ' ';
foreach ($m->documentElement->children as $k => $v) echo $k, $v->nodeName, ' ';
foreach ($m->documentElement->childNodes as $k => $v) echo $k, $v->nodeName, ' ';
echo "\n";
foreach (iterator_to_array($m->documentElement->childNodes) as $k => $v) echo $k, $v->nodeName, spl_object_id($v), ' ';
echo "\n";
var_dump(iterator_count($a->childNodes), spl_object_id(new stdClass));
$g = (function() use ($a) { yield from $a->childNodes; })(); foreach ($g as $k => $v) echo $k, $v->nodeName;
echo "\n";
var_dump($a->childNodes->getIterator());
// The iterator takes its object id before the first item.
$o = [new stdClass, new stdClass, new stdClass]; $o = null;
foreach ($m->documentElement->childNodes as $k => $v) echo $k, $v->nodeName, spl_object_id($v), ' ';
echo "\n";
foreach (['Dom\NodeList', 'Dom\HTMLCollection', 'Dom\NamedNodeMap', 'Dom\DtdNamedNodeMap', 'DOMNodeList', 'DOMNamedNodeMap'] as $c) echo (new ReflectionMethod($c, 'getIterator'))->getReturnType(), ' ';
echo "\n";
