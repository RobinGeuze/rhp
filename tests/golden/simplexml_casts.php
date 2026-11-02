<?php
// SimpleXMLElement's object handlers: cast_object (int/float/bool/string,
// arithmetic operands), compare, get_properties ((array), get_object_vars,
// var_export), has_property, clone, unset_dimension, plus addChild's entity
// parsing, serialization refusal and asXML() to an unwritable path.
set_error_handler(function ($no, $msg) { echo "E$no: $msg\n"; return true; });
$x = new SimpleXMLElement('<a x="1"><b>abc</b><c>3x</c><d>2.5</d><e/></a>');
var_dump($x->b + 1, $x->c + 1, $x->d * 2, $x->d . "!", $x->b % 2, $x->d ** 2, -$x->d, $x->d <=> 3);
var_dump($x->nope + 1, $x->d | 1, $x->d > "2", (int)$x, intval($x->d), intval($x->d, 16), floatval($x->d), boolval($x->nope), boolval($x->e));
var_dump(strval($x->b), array_map('strval', [$x->c, $x->d]));
var_dump($x->b == "abc", $x->d == 2.5, 2.5 == $x->d, $x->b == $x->b, $x->b == $x->c, $x->b < $x->c, $x->d == true);
var_dump($x->b != null, $x->nope == null, $x == $x, $x->b[0] == $x->b, $x->d == $x->d[0], [] == $x->b);
$y = $x->d; $y += 1; var_dump($y);
$y = $x->d; $y++; var_dump($y);
$z = new SimpleXMLElement('<a x="1"><b>1</b><b>2</b><c><d>q</d></c></a>');
print_r((array)$z);
print_r((array)$z->c);
print_r(get_object_vars($z));
var_export($z); echo "\n";
var_dump(property_exists($z, 'b'), property_exists($z, 'nope'), property_exists($z, 'c'));
$arr = iterator_to_array($z->b, false);
var_dump(count($arr), (string)$arr[1]);
$c = clone $z;
$c->b[0] = 'cl';
echo $z->asXML(), $c->asXML();
$cb = clone $z->c;
$cb->d = 'changed';
echo $z->c->asXML(), "\n", $cb->asXML(), "\n";
try { serialize($z); } catch (Exception $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { unserialize('O:16:"SimpleXMLElement":0:{}'); } catch (Exception $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
unset($z->b[0]);
echo $z->asXML();
unset($z->c);
echo $z->asXML();
unset($z['x']);
echo $z->asXML();
$n = new SimpleXMLElement('<a/>');
var_dump($n->addChild('b', 'x&y')->getName());
$n->addChild('c', '&amp;&lt;&#65;&bogus;<');
$n->addChild('d', '&#x41;&#66;');
echo $n->asXML();
libxml_use_internal_errors(true);
$n->addChild('e', 'q&r');
foreach (libxml_get_errors() as $er) var_dump($er->level, $er->code, $er->line, $er->column, $er->message);
libxml_clear_errors();
libxml_use_internal_errors(false);
var_dump($n->asXML('/nonexistent-dir/x.xml'));
// a plain object: strval goes through __toString or fails
class S { function __toString(): string { return "s"; } }
var_dump(strval(new S));
try { strval(new stdClass); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
// unset() of a dimension of a magic (__get) property reaches offsetUnset
class AA implements ArrayAccess {
    function offsetGet($k): mixed { return 1; }
    function offsetSet($k, $v): void { echo "set $k\n"; }
    function offsetExists($k): bool { return true; }
    function offsetUnset($k): void { echo "unset $k\n"; }
}
class M { function __get($n) { echo "get $n\n"; return new AA; } }
$m = new M;
unset($m->x[0]);
unset($m->y['k']);
