<?php
// The engine's `__` helpers behind its built-in classes are not functions
// user code can see or call.
foreach (['__reflect', '__reflect_invoke', '__reflect_meta', '__xml_fn'] as $f) {
    echo $f, ": ", var_export(function_exists($f), true), "\n";
    try {
        $f();
    } catch (Error $e) {
        echo "  ", get_class($e), ": ", $e->getMessage(), "\n";
    }
}
try {
    __reflect('class', 'stdClass');
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
try {
    __reflect_meta('stdClass');
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
try {
    __xmlwriter_fn(...[1, 2]);
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
echo "__dom_throw: ", var_export(function_exists('__dom_throw'), true), "\n";
try {
    __dom_throw(12, 'x');
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
try {
    call_user_func('__reflect', 'class', 'stdClass');
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
// A user function may still use the prefix.
function __mine() { return "mine"; }
echo __mine(), " ", var_export(function_exists('__mine'), true), "\n";
// The classes built on the helpers still work.
echo (new ReflectionClass('ArrayObject'))->getName(), "\n";
