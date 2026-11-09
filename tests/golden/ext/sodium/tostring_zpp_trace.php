<?php
// zpp runs in the builtin's frame: a __toString() it calls shows the
// builtin on its trace, and sodium strips the arguments off what it threw.
class S { function __toString(): string { throw new Exception('ts'); } }
function f($k) { return sodium_bin2hex($k); }
function g($k) { return str_repeat($k, 2); }
foreach (['f', 'g'] as $fn) {
    try { $fn(new S); } catch (Exception $e) { echo preg_replace('~^#\d+ \S*?(\(\d+\))?:~m', '#:', $e->getTraceAsString()), "\n"; }
}
