<?php
// `$ao[k] =& $v` on ArrayObject / ArrayIterator: the storage slot becomes
// the reference; php's key conversion and errors.
set_error_handler(function ($no, $msg) { echo "E$no: $msg\n"; return true; });
function t($f) {
    try { $f(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}
$v = 1;
t(function () use (&$v) { $ao = new ArrayObject([]); $ao['k'] =& $v; $v = 2; var_dump($ao['k']); });
t(function () use (&$v) { $ao = new ArrayObject([]); $ao[] =& $v; var_dump(count($ao)); });
t(function () use (&$v) { $ao = new ArrayObject([]); $ao[null] =& $v; var_dump($ao->getArrayCopy()); });
t(function () use (&$v) { $ao = new ArrayObject([]); $ao[1.5] =& $v; var_dump($ao->getArrayCopy()); });
t(function () use (&$v) { $ao = new ArrayObject([]); $ao[true] =& $v; var_dump($ao->getArrayCopy()); });
t(function () use (&$v) { $ao = new ArrayObject([]); $ao[[]] =& $v; });
t(function () use (&$v) { $it = new ArrayIterator([1]); $it[0] =& $v; $v = 4; var_dump($it[0]); });
class S extends ArrayObject {
    function offsetGet($k): mixed { echo "get $k\n"; return parent::offsetGet($k); }
}
t(function () use (&$v) { $s = new S(['k' => 0]); $s['k'] =& $v; });
$ao = new ArrayObject([]);
$ao[2.5]['x'] = 1;
var_dump($ao->getArrayCopy());
// an error handler may read the ArrayObject while the key converts
set_error_handler(function ($no, $msg) use (&$ao2) { echo "handler sees ", count($ao2), "\n"; return true; });
$ao2 = new ArrayObject([]);
$ao2[1.5] =& $v;
var_dump(count($ao2));
