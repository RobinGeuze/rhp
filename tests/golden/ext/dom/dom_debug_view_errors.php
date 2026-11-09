<?php
// dom_get_debug_info: a read handler's exception stays pending while the
// dump goes on; each failing property chains one.
$node = (new ReflectionClass('DOMElement'))->newInstanceWithoutConstructor();
try { print_r($node); } catch (DOMException $e) {
    $n = 0; for ($x = $e; $x; $x = $x->getPrevious()) $n++;
    echo "\n", get_class($e), ' ', $e->getMessage(), ' ', $e->getCode(), ' ', $n, "\n";
}
echo spl_object_id(new stdClass), "\n";
class Demo extends DOMXPath { public function __construct() {} }
$demo = new Demo;
try { var_dump($demo); } catch (DOMException $e) { echo $e->getMessage(), "\n"; }
try { var_dump($demo->document); } catch (DOMException $e) { echo $e->getMessage(), "\n"; }

// Dom\XMLDocument::xinclude refuses a flag value past int.
$dom = Dom\XMLDocument::createEmpty();
try { $dom->xinclude(PHP_INT_MAX); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(Dom\XMLDocument::createFromString('<r/>')->xinclude());
var_dump(@(new DOMDocument)->xinclude(PHP_INT_MAX));

// Validators see a modern document's namespace declarations (GH-22219).
$xml = '<root xmlns="urn:test" xmlns:ref="urn:other"><item target="ref:something"/></root>';
$xsd = '<schema xmlns="http://www.w3.org/2001/XMLSchema" targetNamespace="urn:test" elementFormDefault="qualified">
  <element name="root"><complexType><sequence><element name="item"><complexType>
  <attribute name="target" type="QName"/></complexType></element></sequence></complexType></element></schema>';
libxml_use_internal_errors(true);
var_dump(Dom\XMLDocument::createFromString($xml, LIBXML_NSCLEAN)->schemaValidateSource($xsd));
$m = Dom\XMLDocument::createFromString($xml);
var_dump($m->schemaValidateSource($xsd), $m->saveXml());
var_dump(count(libxml_get_errors()));
