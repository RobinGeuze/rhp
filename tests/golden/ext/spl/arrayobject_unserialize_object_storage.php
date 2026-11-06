<?php
// __unserialize() with an object as backing storage: one deprecation,
// named after the method's class and the object's kind.
set_error_handler(function ($no, $msg) {
    echo "[$no] $msg\n";
    return true;
});
$o = new stdClass;
$o->a = 1;
class MyIt extends ArrayIterator {}
class MyAo extends ArrayObject {}
foreach ([new ArrayIterator([]), new MyIt([]), new ArrayObject([]), new MyAo([])] as $x) {
    $x->__unserialize([0, $o, []]);
    echo get_class($x), " ", count($x), "\n";
}
unserialize('O:13:"ArrayIterator":4:{i:0;i:0;i:1;O:8:"stdClass":1:{s:1:"a";i:1;}i:2;a:0:{}i:3;N;}');
unserialize('O:11:"ArrayObject":4:{i:0;i:0;i:1;O:8:"stdClass":1:{s:1:"a";i:1;}i:2;a:0:{}i:3;N;}');
echo "done\n";
