<?php
trait Greet {
    public function hello(): string { return "Hello from " . $this->who(); }
    abstract public function who(): string;
}
trait Counter {
    private int $count = 0;
    public function inc(): static { $this->count++; return $this; }
    public function count(): int { return $this->count; }
}
class Widget {
    use Greet, Counter;
    public function __construct(private string $name) {}
    public function who(): string { return $this->name; }
}
$w = new Widget("gadget");
echo $w->hello(), "\n";
$w->inc()->inc()->inc();
echo $w->count(), "\n";
echo get_class($w), "\n";
var_dump(method_exists($w, 'hello'));
var_dump(in_array('Greet', class_uses($w)));
