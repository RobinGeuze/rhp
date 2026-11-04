<?php
// DOMXPath: XPath 1.0 location paths, predicates, functions and namespaces.
set_error_handler(function ($no, $str) { echo "W[$no]: $str\n"; return true; });
$d = new DOMDocument;
$d->preserveWhiteSpace = false;
$d->loadXML('<?xml version="1.0"?>
<phpunit bootstrap="vendor/autoload.php">
  <!-- a comment -->
  <testsuites>
    <testsuite name="unit"><directory suffix="Test.php">tests/Unit</directory><file>a.php</file></testsuite>
    <testsuite name="iso"><directory>tests/Iso</directory></testsuite>
  </testsuites>
  <php><ini name="a" value="1"/><ini name="b" value="2"/><const name="X" value="y"/></php>
  <n:extra xmlns:n="urn:n"><n:item k="1">one</n:item><n:item k="2">two</n:item></n:extra>
</phpunit>');
$x = new DOMXPath($d);
foreach (['testsuites/testsuite', '//directory', 'php/ini', '//comment()', '/phpunit/*', '//testsuite[@name="iso"]/directory',
    '//ini[2]', '//ini[last()]', '//ini[position() < 2]/@name', '//@name', 'testsuites/testsuite[1]/*', '//text()',
    '(//ini | //const)', '//testsuite[directory/@suffix]', '//*[not(*)]', '//ini[@value > 1]', 'child::php/child::ini[@name = "b"]',
    '//testsuite[contains(@name, "n")]', '..', '.', '//directory/..', '//ini/following-sibling::*', '//const/preceding-sibling::ini'] as $q) {
    $r = $x->query($q);
    echo str_pad($q, 40), ' => ', $r->length, ':';
    foreach ($r as $n) echo ' ', $n->nodeName, '=', json_encode($n->nodeValue);
    echo "\n";
}
foreach (['count(//ini)', 'string(//ini/@name)', 'sum(//ini/@value)', 'name(/*)', 'local-name(//*[last()])', '1 = 1', '//ini/@value = 2',
    'concat("a", "b", 3)', 'substring("12345", 2, 3)', 'string-length("héllo")', 'normalize-space("  a  b ")', 'translate("abc", "ab", "AB")',
    '10 div 4', '7 mod 3', '-3 + 1.5', 'floor(2.7)', 'ceiling(2.1)', 'round(2.5)', 'boolean(//nope)', 'not(true())', 'number("12")',
    'starts-with("phpunit", "php")', 'substring-before("a/b", "/")', 'substring-after("a/b", "/")', '1 div 0', '0 div 0', 'string(1 div 3)', 'string(0.1 + 0.2)', 'string(1e20 * 1)'] as $q) {
    echo str_pad($q, 40), ' => ', var_export($x->evaluate($q), true), "\n";
}
$ctx = $x->query('//testsuite')->item(0);
var_dump($x->query('directory', $ctx)->item(0)->textContent, $x->evaluate('string(@name)', $ctx));
$x->registerNamespace('m', 'urn:n');
var_dump($x->query('//m:item[@k="2"]')->item(0)->textContent, $x->evaluate('count(//m:*)'));
var_dump($x->query('//['), $x->query('count(//ini)')->length);
var_dump($x->document === $d, DOMXPath::quote("it's"), DOMXPath::quote('a"b\'c'));
// removing comments the way phpunit's loader does
foreach ($x->query('//comment()') as $c) { $c->parentNode->removeChild($c); }
var_dump($x->query('//comment()')->length);
