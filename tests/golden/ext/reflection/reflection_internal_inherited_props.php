<?php
// A built-in subclass does not inherit its parent's private properties.
foreach (['RuntimeException', 'LogicException', 'Exception', 'TypeError', 'Error'] as $c) {
    foreach ((new ReflectionClass($c))->getProperties() as $p) {
        echo $c, ' ', $p->class, '::', $p->getName(), "\n";
    }
    var_dump((new ReflectionClass($c))->hasProperty('string'));
}
class AppException extends RuntimeException { private $extra; }
foreach ((new ReflectionClass('AppException'))->getProperties() as $p) {
    echo $p->class, '::', $p->getName(), "\n";
}
