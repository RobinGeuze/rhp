<?php
function f() {
    return new ArrayObject([], 0, 'ArrayIterator', 5);
}
try {
    iterator_to_array();
} catch (ArgumentCountError $e) {
    echo basename($e->getFile()), ':', $e->getLine(), "\n";
}
f();
