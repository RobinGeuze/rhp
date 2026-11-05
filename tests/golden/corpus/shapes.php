<?php
interface Shape {
    public function area(): float;
    public function name(): string;
}
abstract class Base implements Shape {
    public function describe(): string {
        return sprintf("%s has area %.2f", $this->name(), $this->area());
    }
}
class Circle extends Base {
    public function __construct(private float $r) {}
    public function area(): float { return M_PI * $this->r ** 2; }
    public function name(): string { return "Circle"; }
}
class Rect extends Base {
    public function __construct(private float $w, private float $h) {}
    public function area(): float { return $this->w * $this->h; }
    public function name(): string { return "Rect"; }
}
$shapes = [new Circle(2), new Rect(3, 4), new Circle(1.5)];
usort($shapes, fn(Shape $a, Shape $b) => $a->area() <=> $b->area());
foreach ($shapes as $s) echo $s->describe(), "\n";
$total = array_sum(array_map(fn(Shape $s) => $s->area(), $shapes));
printf("total: %.4f\n", $total);
echo ($shapes[0] instanceof Shape) ? "yes\n" : "no\n";
