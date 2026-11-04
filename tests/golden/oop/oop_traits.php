<?php
trait Greetable {
    public $greeting = "Hello";
    public function greet() { return $this->greeting . ", " . $this->name() . "!"; }
}
trait Countable2 {
    private static $count = 0;
    public function bump() { self::$count++; return self::$count; }
}
class Person {
    use Greetable;
    use Countable2;
    public function __construct(private $name) {}
    public function name() { return $this->name; }
}
$p = new Person("Alice");
echo $p->greet(), "\n";
echo $p->bump(), " ", $p->bump(), "\n";
