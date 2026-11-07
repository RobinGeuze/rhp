<?php
// "Lazy object was released during initialization" for classes without a
// destructor, and no error when something else still holds the object.

class Plain {
    public $v;
}

$r = new ReflectionClass(Plain::class);

$o = $r->newLazyGhost(function ($obj) {
    global $o;
    $o = null;
});
try {
    $o->v = 1;
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}

$o = $r->newLazyGhost(function ($obj) {
    global $o;
    $o = null;
});
$keep = $o;
$o->v = 2;
var_dump($keep);

$o = $r->newLazyProxy(function ($obj) {
    global $o;
    $o = null;
    return new Plain;
});
try {
    var_dump($o->v);
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
var_dump($o);
