<?php
abstract class Product {
    public function __construct(
        public readonly string $sku,
        public readonly string $name,
        protected float $price,
    ) {}
    abstract public function finalPrice(): float;
    public function label(): string {
        return sprintf("[%s] %s: $%.2f", $this->sku, $this->name, $this->finalPrice());
    }
}
class Standard extends Product {
    public function finalPrice(): float { return $this->price; }
}
class Discounted extends Product {
    public function __construct(string $sku, string $name, float $price, private float $pct) {
        parent::__construct($sku, $name, $price);
    }
    public function finalPrice(): float { return $this->price * (1 - $this->pct / 100); }
}
$products = [
    new Standard("A1", "Widget", 9.99),
    new Discounted("B2", "Gadget", 19.99, 25),
    new Standard("C3", "Gizmo", 4.50),
    new Discounted("D4", "Doohickey", 100, 10),
];
$total = 0;
foreach ($products as $p) {
    echo $p->label(), "\n";
    $total += $p->finalPrice();
}
printf("Total: $%.2f\n", $total);
$cheap = array_filter($products, fn($p) => $p->finalPrice() < 10);
echo "cheap: " . implode(",", array_map(fn($p) => $p->name, $cheap)) . "\n";
