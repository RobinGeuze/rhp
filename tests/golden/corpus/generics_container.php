<?php
class TypedCollection {
    private array $items = [];
    public function __construct(private string $type) {}
    public function add(mixed $item): void {
        if (!($item instanceof $this->type)) {
            throw new TypeError("expected {$this->type}");
        }
        $this->items[] = $item;
    }
    public function map(callable $fn): array {
        return array_map($fn, $this->items);
    }
    public function filter(callable $fn): array {
        return array_values(array_filter($this->items, $fn));
    }
    public function each(callable $fn): void {
        array_walk($this->items, $fn);
    }
}
class Animal {
    public function __construct(public string $name, public int $legs) {}
}
$c = new TypedCollection(Animal::class);
$c->add(new Animal("dog", 4));
$c->add(new Animal("bird", 2));
$c->add(new Animal("spider", 8));
print_r($c->map(fn($a) => $a->name));
$manyLegs = $c->filter(fn($a) => $a->legs > 2);
echo count($manyLegs), "\n";
$c->each(function ($a) { echo "{$a->name}: {$a->legs}\n"; });
try {
    $c->add("not an animal");
} catch (TypeError $e) {
    echo "rejected: " . $e->getMessage() . "\n";
}
