<?php
class Animal {
    protected $name;
    public function __construct($name) { $this->name = $name; }
    public function speak() { return "..."; }
    public function describe() { return $this->name . " says " . $this->speak(); }
    public function who() { return static::class; }
}
class Dog extends Animal {
    public function speak() { return "Woof"; }
    public function describe() { return "Dog: " . parent::describe(); }
}
$d = new Dog("Rex");
echo $d->describe(), "\n";
echo $d->who(), "\n";
var_dump($d instanceof Animal);
var_dump($d instanceof Dog);
var_dump((new Animal("x")) instanceof Dog);

class Config {
    public static $count = 0;
    const VERSION = "1.0";
    public static function bump() { self::$count++; return static::class; }
}
Config::bump();
Config::bump();
echo Config::$count, "\n";
echo Config::VERSION, "\n";
echo Config::class, "\n";

class Point {
    public function __construct(public $x, public $y) {}
}
