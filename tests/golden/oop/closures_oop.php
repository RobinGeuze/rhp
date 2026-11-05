<?php
class Counter {
    private int $n;
    public function __construct(int $start) { $this->n = $start; }
    public function adder(): callable { return fn($x) => $this->n + $x; }
    public function incrementer(): callable {
        return function() { $this->n++; return $this->n; };
    }
    public function value(): int { return $this->n; }
}
$c = new Counter(10);
$a = $c->adder();
echo $a(5), "\n";
$inc = $c->incrementer();
echo $inc(), " ", $inc(), " ", $inc(), "\n";
echo $c->value(), "\n";

class Registry {
    private array $items = [];
    public function add(string $k, callable $f): void { $this->items[$k] = $f; }
    public function run(string $k, ...$args) { return ($this->items[$k])(...$args); }
}
$r = new Registry();
$r->add("double", fn($x) => $x * 2);
$r->add("greet", fn($name) => "Hi $name");
echo $r->run("double", 21), "\n";
echo $r->run("greet", "Al"), "\n";
