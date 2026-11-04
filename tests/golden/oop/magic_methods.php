<?php
class Container {
    private array $data = [];
    public function __get($name) { return $this->data[$name] ?? null; }
    public function __set($name, $value) { $this->data[$name] = $value; }
    public function __isset($name) { return isset($this->data[$name]); }
    public function __unset($name) { unset($this->data[$name]); }
    public function __call($name, $args) { return "$name(" . implode(",", $args) . ")"; }
    public static function __callStatic($name, $args) { return "static $name"; }
    public function __invoke(...$args) { return array_sum($args); }
}
$c = new Container();
$c->x = 10;
$c->y = 20;
echo $c->x + $c->y, "\n";
var_dump(isset($c->x));
var_dump(isset($c->missing));
unset($c->x);
var_dump(isset($c->x));
echo $c->x ?? "gone", "\n";
echo $c->compute(1, 2, 3), "\n";
echo Container::make(), "\n";
echo $c(4, 5, 6), "\n";

// declared properties still write directly and enforce visibility
class Account {
    public string $owner = "none";
    protected float $balance = 0.0;
    public function deposit(float $n): void { $this->balance += $n; }
    public function balance(): float { return $this->balance; }
}
$a = new Account();
$a->owner = "Robin";
$a->deposit(50.5);
$a->deposit(9.5);
echo $a->owner, ": ", $a->balance(), "\n";
var_dump(isset($a->owner));
try {
    echo $a->balance;
} catch (\Error $e) {
    echo "blocked: ", $e->getMessage(), "\n";
}
