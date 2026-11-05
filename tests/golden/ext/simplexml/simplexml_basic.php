<?php
// SimpleXML over the shared libxml tree (navigation, casts, xpath, import).
$x = simplexml_load_string('<?xml version="1.0"?><root a="1" b="two"><item id="x">one</item><item id="y">two<sub>s</sub>tail</item><other/><p:q xmlns:p="urn:p">pq</p:q></root>');
var_dump((string)$x['a'], (int)$x['a'], (string)$x['b'], isset($x['a']), isset($x['zz']), (string)$x['zz']);
var_dump(isset($x->item), isset($x->nope), count($x->item), count($x), $x->getName());
foreach ($x->item as $k => $it) { echo $k, ' ', $it['id'], ' [', $it, "]\n"; }
foreach ($x as $k => $c) { echo "child $k\n"; }
echo $x->item[1]->sub, "\n";
echo $x->item[1], "\n";
var_dump((bool)$x->nope, (bool)$x->other, (bool)$x->item);
if ($x->nope) echo "yes\n"; else echo "no\n";
foreach ($x->attributes() as $k => $v) echo "$k=$v\n";
echo $x->asXML(), "\n";
echo $x->item[0]->asXML(), "\n";
$c = $x->addChild('new', 'v&amp;w');
$c->addAttribute('k', 'v');
echo $x->asXML(), "\n";
var_dump(count($x->children('urn:p')), (string)$x->children('p', true)->q);
$r = $x->xpath('//item[@id="y"]');
var_dump(count($r), (string)$r[0]['id']);
var_dump($x->item == 'one', $x->item[0] == 'one');
var_dump(json_encode((string)$x->item));
$d = new DOMDocument; $d->loadXML('<cfg xmlns="urn:c" level="3"><projectFiles><directory name="src"/><directory name="lib"/></projectFiles></cfg>');
$s = simplexml_import_dom($d);
var_dump((int)$s['level'], isset($s->projectFiles), isset($s->projectFiles->directory));
foreach ($s->projectFiles->directory as $dir) echo $dir['name'], "\n";
var_dump($s->projectFiles->directory[1]['name'] . '');
$e = dom_import_simplexml($s->projectFiles);
var_dump(get_class($e), $e->nodeName);
set_error_handler(function ($no, $str) { echo "W[$no]: $str\n"; return true; });
try { new SimpleXMLElement('<bad'); } catch (Exception $ex) { echo get_class($ex), ': ', $ex->getMessage(), "\n"; }
var_dump(simplexml_load_string('<x'));
$o = new SimpleXMLElement('<a><b>1</b><b>2</b></a>');
var_dump((float)$o->b[1] + 1);
echo $o->b, $o->b[1], "\n";
var_dump(get_class($o->b), $o->getNamespaces(), $x->getDocNamespaces(true));
