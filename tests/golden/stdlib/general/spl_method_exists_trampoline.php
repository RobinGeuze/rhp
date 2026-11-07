<?php
// method_exists() through SPL outer iterators (their get_method handler
// forwards to the inner iterator), and reflection of a first-class
// callable only __call / __callStatic answers.
$it = new IteratorIterator(new ArrayIterator([1, 2, 3]));
var_dump(method_exists($it, 'offsetGet'), method_exists($it, 'nope'), method_exists('IteratorIterator', 'offsetGet'));
foreach ($it as $v) {
    var_dump($it->offsetGet(0) + $v);
}
$f = new CallbackFilterIterator(new ArrayIterator([1, 2]), fn() => true);
var_dump(method_exists($f, 'getArrayCopy'));
$r = new RecursiveIteratorIterator(new RecursiveArrayIterator([1, [2]]));
$r->rewind();
var_dump(method_exists($r, 'getArrayCopy'));

class Magic {
    public function __call($name, $args) { return [$name, $args]; }
    public static function __callStatic($name, $args) { return [$name, $args]; }
}
foreach ([Magic::stat(...), (new Magic)->inst(...)] as $c) {
    $rf = new ReflectionFunction($c);
    var_dump($rf->getName(), $rf->isVariadic(), $rf->getNumberOfParameters(), $rf->isInternal());
    var_dump($rf->isStatic(), $rf->getClosureThis() instanceof Magic);
    $p = $rf->getParameters()[0];
    var_dump($p->getName(), $p->isVariadic(), $p->isOptional(), (string) $p->getType());
    echo $rf;
    var_dump($c(1, b: 2));
}

// Unpacking in a constant expression takes arrays only.
const OK = [...[1, 2], ...['a' => 3]];
var_dump(OK);
function d($x = [...new ArrayIterator([1])]) { return $x; }
try {
    d();
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), ' @', $e->getLine(), "\n";
}
var_dump(d([5]));
