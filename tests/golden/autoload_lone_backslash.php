<?php
// GH-23232 (php 8.5.11): an empty class name, or a lone "\", never reaches
// the autoloader.
spl_autoload_register(function (string $class): void {
    echo "autoload: '$class'\n";
});

foreach (['::a', '\::a', '\Foo::a', 'Foo::a', '\\\\::a'] as $callable) {
    echo "is_callable(\"$callable\")\n";
    var_dump(is_callable($callable));
}

var_dump(class_exists('\\'));
var_dump(class_exists(''));
var_dump(interface_exists('\\'));
var_dump(enum_exists('\\', true));
var_dump(class_exists('\\Bar'));
try { new ReflectionClass('\\'); } catch (ReflectionException $e) { echo $e->getMessage(), "\n"; }
var_dump(is_subclass_of('\\', 'Foo'));
var_dump(is_a('\\', 'Foo', true));
