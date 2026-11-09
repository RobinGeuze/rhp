<?php
// `$a[]->p = v` is FETCH_DIM_W with no dimension: an ArrayAccess object's
// offsetGet(null) gives the object written to; a variable holding an
// array, null or false gets a null element first; strings and other
// scalars are an Error.
ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
class AA implements ArrayAccess {
    public $o;
    function offsetGet($k): mixed { echo "get "; var_dump($k); return $this->o ??= new stdClass; }
    function offsetSet($k, $v): void { echo "set\n"; }
    function offsetExists($k): bool { return true; }
    function offsetUnset($k): void {}
}
$a = new AA; $a[]->q = 2; var_dump($a->o);
$a = new AA; $a[]->q[] = 4; var_dump($a->o);
foreach ([[], null, [1], "", "ab", 5, false, true, new stdClass] as $a) {
    try { $a[]->p = 1; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    var_dump($a);
}
try { $nope[]->p = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump($nope);
$r = [1]; $ref = &$r;
try { $r[]->p = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump($ref);
$x = simplexml_load_string("<root><bla/></root>");
$t = $x->bla; $t[]->name = "A";
$x->bla[]->name = "B";
echo $x->asXML();
