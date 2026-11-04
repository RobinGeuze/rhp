<?php
// A file required inside a function reads the caller's variables and
// defines new ones in its scope.
function load($p) {
    $base = '/root';
    $ret = require $p;
    return "$ret|$vendorDir|$extra";
}
echo load(__DIR__ . '/include_caller_scope_vars.inc'), "\n";
