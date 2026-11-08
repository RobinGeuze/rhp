<?php
// php_spl.c: a closure registered as [$closure, '__invoke'] is listed as
// that pair (its function has a scope), and differs from the closure.
$c = function ($x) {};
spl_autoload_register([$c, '__invoke']);
$f = spl_autoload_functions();
var_dump(is_array($f[0]), $f[0][0] === $c, $f[0][1]);
spl_autoload_register($c);
var_dump(count(spl_autoload_functions()), spl_autoload_functions()[1] === $c);
var_dump(spl_autoload_unregister([$c, '__invoke']), count(spl_autoload_functions()));
