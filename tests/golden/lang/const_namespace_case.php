<?php
// php keys a namespaced constant by its namespace in lowercase and its short
// name as written (zend_register_constant, zend_get_constant_ex): the
// namespace matches in any case, the redefinition warning names that key,
// and get_defined_constants() keeps the name as defined.
namespace Foo\Sub;

\set_error_handler(function ($no, $msg) {
    echo "warning: $msg\n";
    return true;
});
const X = 1;
const X = 2;
echo X, " ", \FOO\SUB\X, " ", \foo\sub\X, "\n";
\var_dump(\defined('FOO\SUB\X'), \defined('\FOO\SUB\X'), \defined('foo\sub\x'));
\var_dump(\constant('FOO\sub\X'), \constant('\foo\SUB\X'));
\var_dump(\define('A\B\C', 1), \define('a\b\C', 2), \define('A\B\c', 3));
echo \A\B\C, \a\b\C, \A\B\c, \a\B\c, "\n";
\var_dump(\get_defined_constants(true)['user']);
foreach (['\Nope\X', 'nope\X', 'Foo\Sub\x'] as $name) {
    try {
        \constant($name);
    } catch (\Error $e) {
        echo \get_class($e), ": ", $e->getMessage(), "\n";
    }
}
try {
    echo \Nope\Y;
} catch (\Error $e) {
    echo \get_class($e), ": ", $e->getMessage(), "\n";
}
try {
    echo Z;
} catch (\Error $e) {
    echo \get_class($e), ": ", $e->getMessage(), "\n";
}
// The warning prints the name as a C string.
\var_dump(\define("N\0ul", 1), \define("N\0ul", 2));
