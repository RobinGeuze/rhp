<?php
$d = new DOMDocument; $d->loadXML('<r><e>baz</e><e>qux</e></r>');
$els = iterator_to_array($d->documentElement->childNodes);
var_dump(array_column($els, 'nodeValue'), array_column($els, 'localName'));
var_dump(isset($els[0]->nodeValue), property_exists($els[0], 'nodeValue'));
