<?php
// SimpleXML's property views: var_dump / print_r (get_debug_info) and
// json_encode (get_properties), incl. the json round-trip idiom.
$x = simplexml_load_string('<?xml version="1.0"?><root a="1" b="two"><item id="x">one</item><item id="y">two<sub>s</sub>tail</item><other/><c><d>1</d><d>2</d></c><!-- cm --><e>  </e><f><![CDATA[cd]]></f></root>');
var_dump($x);
print_r($x);
echo "\n";
print_r($x->item);
print_r($x->c->d);
print_r($x['a']);
print_r($x->attributes());
print_r($x->children());
echo json_encode($x), "\n";
echo json_encode($x->c), "\n";
echo json_encode(simplexml_load_string('<a><b>1</b><b>2</b><c x="y">t</c></a>')), "\n";
var_dump(json_decode(json_encode(simplexml_load_string('<cfg><name>x</name><list><i>1</i><i>2</i></list></cfg>')), true));
print_r(simplexml_load_string('<a>hi</a>'));
print_r(simplexml_load_string('<a/>'));
$l = (new DOMDocument);
$l->loadXML('<r a="1" b="2"><i/><i/></r>');
$nodes = $l->getElementsByTagName('i');
var_dump(isset($nodes[1]), isset($nodes[2]), $nodes[0]->nodeName, $nodes['x'], $l->documentElement->attributes['b']->value, $l->documentElement->attributes[0]->name);
