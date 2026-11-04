<?php
// A nested lookup of a class whose autoload is already in progress does not
// re-enter the autoloaders (php's EG(in_autoload)).
spl_autoload_register(function ($c) {
    echo "load $c\n";
    var_dump(class_exists($c));
    if ($c === 'Dep') {
        try {
            var_dump(A::$t);
        } catch (Error $e) {
            echo "inner: ", $e->getMessage(), "\n";
        }
    }
});
var_dump(class_exists('X'));
var_dump(class_exists('X'));
class A { public static $t = 1; public static $s = Dep::V; }
for ($i = 0; $i < 2; $i++) {
    try {
        var_dump(A::$s);
    } catch (Error $e) {
        echo "outer: ", $e->getMessage(), "\n";
    }
}
var_dump(interface_exists('Y'));
var_dump(enum_exists('Z'));
try {
    new Missing();
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
