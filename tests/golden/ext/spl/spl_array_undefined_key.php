<?php
// ArrayObject / ArrayIterator reads of a missing key warn like arrays do,
// including the read half of a compound assignment through ArrayAccess.
set_error_handler(function ($no, $msg, $file, $line) {
    echo "[$no] $msg (line $line)\n";
    return true;
});
$ao = new ArrayObject(['a' => ['b' => 1]]);
var_dump($ao['zz'], $ao->offsetGet('yy'), $ao['a']['b'], $ao['q'] ?? 'dflt', isset($ao['q']));
$ao['x'] .= 'y';
$ao['n']++;
$ao['a']['c'] .= 'z';
print_r($ao->getArrayCopy());
$ai = new ArrayIterator([]);
var_dump($ai[5]);
