<?php
// property_exists() sees the properties internal classes declare
// but serve from handlers (DOM, XMLReader, DatePeriod).
var_dump(property_exists('DOMDocument', 'preserveWhiteSpace'));
var_dump(property_exists('DOMDocument', 'formatOutput'));
var_dump(property_exists(new DOMDocument, 'formatOutput'));
var_dump(property_exists('DOMDocument', 'nodeName'));
var_dump(property_exists('DOMElement', 'tagName'));
var_dump(property_exists('DOMDocument', 'tagName'));
var_dump(property_exists('DOMNodeList', 'length'));
var_dump(property_exists('DOMXPath', 'document'));
var_dump(property_exists('DOMXPath', 'registerNodeNamespaces'));
class MyDoc extends DOMDocument {}
var_dump(property_exists('MyDoc', 'formatOutput'));
var_dump(property_exists('DOMDocument', 'nope'));
var_dump(property_exists('XMLReader', 'name'));
var_dump(property_exists('XMLReader', 'nodeType'));
var_dump(property_exists(new XMLReader, 'name'));
var_dump(property_exists('DatePeriod', 'start'));
var_dump(property_exists('Exception', 'message'));
var_dump(property_exists('ArrayObject', 'x'));
var_dump(property_exists('SplFixedArray', 'x'));
var_dump(property_exists('DOMNameSpaceNode', 'nodeName'));
var_dump(property_exists('DOMEntityReference', 'nodeValue'));
