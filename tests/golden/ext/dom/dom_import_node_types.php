<?php
$d = Dom\XMLDocument::createEmpty();
foreach ([null, 1, new stdClass] as $a) { try { $d->importNode($a); } catch (TypeError $e) { echo $e->getMessage(), "\n"; } }
echo new ReflectionMethod('Dom\Document', 'importNode'), "\n";
$d2 = new DOMDocument;
foreach ([null, 1] as $a) { try { $d2->importNode($a); } catch (TypeError $e) { echo $e->getMessage(), "\n"; } }
echo (new ReflectionMethod('DOMDocument', 'importNode'))->getParameters()[0]->getType(), "\n";
echo $d->saveXml($d->importNode($d->createElement("x"))), "\n";
