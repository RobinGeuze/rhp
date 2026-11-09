<?php
// A write fetch of a DOM property (by reference, a nested write) works on a
// temporary copy, silently: php's dom_get_property_ptr_ptr gives NULL.
$d = new DOMDocument; $d->loadXML('<r a="1">t<b/></r>'); $e = $d->documentElement;
$r = &$e->nodeValue; var_dump($r); $r = 'changed'; echo $d->saveXML();
try { $e->nodeValue[] = 1; } catch (Error $x) { echo get_class($x), ': ', $x->getMessage(), "\n"; }
try { $e->nodeValue['x'] = 1; } catch (Error $x) { echo get_class($x), ': ', $x->getMessage(), "\n"; }
function f(&$x) { $x = 'viaref'; }
f($e->nodeValue); echo $d->saveXML();
$a = [&$e->prefix]; var_dump($a);
$x = 1;
try { $e->nodeValue = &$x; } catch (Error $x) { echo $x->getMessage(), "\n"; }
$m = Dom\XMLDocument::createFromString('<r>t</r>');
f($m->documentElement->textContent); echo $m->saveXml(), "\n";
