<?php
// var_dump runs __debugInfo() when it reaches the object, so what it
// prints and the diagnostics it raises land mid-dump, as in php.
set_error_handler(function ($no, $msg) {
    echo "[error: $msg]\n";
    return true;
});
class Loud {
    public function __debugInfo() {
        echo "[debugInfo]\n";
        return ['x' => 1];
    }
}
class Nul {
    public function __debugInfo() { return null; }
}
var_dump([1, new Loud, 'after']);
var_dump(['before', new Nul]);
print_r([new Loud]);
echo "\n";

// A __debugInfo() that drops the container being dumped.
$a = [new class {
    public function __debugInfo() {
        global $b;
        $b->a = null;
        return [];
    }
}];
$b = new stdClass;
$b->a = &$a;
var_dump($b);

// Generator::__debugInfo(): the generator function's name
function gen() { yield 1; }
class G {
    public function m() { yield 1; }
    public static function s() { yield 2; }
    public function c() { return function () { yield 3; }; }
}
$g = gen();
var_dump($g);
foreach ($g as $_) {}
var_dump($g, $g->__debugInfo());
var_dump((new G)->m(), G::s(), ((new G)->c())());
print_r(gen());
echo "\n";
var_dump((array) gen(), get_object_vars(gen()));
var_export(gen());
echo "\n";
