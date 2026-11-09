<?php
// foreach over a class get_iterator iterator (DOM lists, DatePeriod,
// WeakMap) runs its funcs directly: no InternalIterator frames, the
// same object ids as php, nested loops and break.
$doc = new DOMDocument; $doc->loadXML('<a><b/><c/></a>');
foreach ($doc->documentElement->childNodes as $k => $n) { echo $k, $n->nodeName, spl_object_id($n), "\n"; }
foreach ($doc->documentElement->attributes as $k => $n) { echo $k; }
$p = new DatePeriod(new DateTime('2020-01-01'), new DateInterval('P1D'), 2);
foreach ($p as $k => $d) echo $k, $d->format('d'), "\n";
$w = new WeakMap; $o = new stdClass; $w[$o] = 5; foreach ($w as $k => $v) { var_dump($k === $o, $v); }
ob_start(fn ($s) => str_replace(__FILE__, "F", $s));
function g() { foreach ((new DOMDocument)->childNodes as $x) {} throw new Exception("x"); }
try { g(); } catch (Exception $e) { echo $e->getTraceAsString(), "\n"; }
$l = $doc->getElementsByTagName('*');
foreach ($l as $x) { foreach ($l as $y) { echo $x->nodeName, $y->nodeName, " "; } } echo "\n";
foreach ($l as $x) { if ($x->nodeName == 'b') break; } echo "ok\n";
$x = new stdClass; echo spl_object_id($x), "\n";
