<?php
// Building report documents (junit/checkstyle style) with DOM: escaping,
// formatOutput, node saves, XPath over the result, importNode.
$d = new DOMDocument('1.0', 'UTF-8');
$d->formatOutput = true;
$root = $d->createElement('testsuites');
$d->appendChild($root);
$s = $d->createElement('testsuite');
$s->setAttribute('name', 'A & B <"x"> \'y\'' . "\t\n\r");
$s->setAttribute('tests', (string) 3);
$s->setAttribute('time', sprintf('%F', 0.123456));
$root->appendChild($s);
$tc = $d->createElement('testcase');
$tc->setAttribute('name', "ünïcödé ✓");
$f = $d->createElement('failure', htmlspecialchars("Failed asserting that 1 is 2.\n\n/x/y.php:12\n", ENT_XML1));
$f->setAttribute('type', 'PHPUnit\\Framework\\ExpectationFailedException');
$tc->appendChild($f);
$tc->appendChild($d->createElement('system-out', "a\x01b"));
$s->appendChild($tc);
$e = $d->createElement('error');
$e->appendChild($d->createCDATASection("x ]]> y\n<z>"));
$s->appendChild($e);
$s->appendChild($d->createComment(' c -- d '));
$s->appendChild($d->createElement('empty'));
$s->appendChild($d->createElement('ws', '   '));
$mixed = $d->createElement('mixed');
$mixed->appendChild($d->createTextNode('t'));
$mixed->appendChild($d->createElement('in'));
$s->appendChild($mixed);
echo $d->saveXML();
echo $d->saveXML($s), "\n";
$d->formatOutput = false;
echo $d->saveXML();
echo $d->documentElement->firstChild->getAttribute('name'), "|\n";
echo $d->documentElement->firstChild->attributes->length, "\n";
foreach ($d->documentElement->firstChild->attributes as $k => $a) echo "$k=", $a->value, "\n";
echo $d->documentElement->firstChild->childNodes->length, "\n";
foreach ($d->getElementsByTagName('*') as $n) echo $n->nodeName, ' ', $n->nodeType, ' ', strlen($n->textContent), "\n";
$x = new DOMXPath($d);
foreach ($x->query('//testcase[@name]/failure/@type') as $n) echo $n->nodeValue, "\n";
var_dump($x->evaluate('count(//*)'), $x->evaluate('string(//testsuite/@tests)'), $x->evaluate('sum(//testsuite/@tests) + 1'));
$n2 = new DOMDocument; $n2->loadXML('<r><a>1</a></r>');
$imp = $n2->importNode($d->documentElement, true);
$n2->documentElement->insertBefore($imp, $n2->documentElement->firstChild);
$n2->documentElement->removeChild($n2->documentElement->lastChild);
echo strlen($n2->saveXML()), "\n";
// load with whitespace handling
$w = new DOMDocument; $w->preserveWhiteSpace = false; $w->loadXML("<a>\n  <b>x</b>\n  <c/>\n</a>"); $w->formatOutput = true; echo $w->saveXML();
$w = new DOMDocument; $w->loadXML("<a>\n  <b>x</b>\n  <c/>\n</a>"); $w->formatOutput = true; echo $w->saveXML();
$w = new DOMDocument; var_dump(@$w->loadXML(''));
try { $d->createElement('1bad'); } catch (DOMException $ex) { echo get_class($ex), ' ', $ex->getCode(), ' ', $ex->getMessage(), "\n"; }
var_dump($d->save(sys_get_temp_dir() . '/rhp-dom-test.xml'));
echo file_get_contents(sys_get_temp_dir() . '/rhp-dom-test.xml') === $d->saveXML() ? "eq\n" : "ne\n";
unlink(sys_get_temp_dir() . '/rhp-dom-test.xml');
