<?php
class Base {
    public static $instances = 0;
    const KIND = "base";
    public function __construct() { static::$instances++; }
    public static function make() { return new static(); }
    public function kind() { return static::KIND; }
}
class Derived extends Base {
    const KIND = "derived";
}
$a = new Base();
$b = Derived::make();
$c = Derived::make();
echo Base::$instances, "\n";
var_dump($b instanceof Derived);
echo $a->kind(), " ", $b->kind(), "\n";
echo Base::KIND, " ", Derived::KIND, "\n";
