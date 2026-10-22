<?php
// phrust demo — a tour of what the engine runs today.

echo "=== scalars, operators, type juggling ===\n";
echo 2 ** 10, " ", intdiv(17, 5), " ", 17 % 5, " ", 7 / 2, "\n";
echo PHP_INT_MAX, " + 1 = ", PHP_INT_MAX + 1, "\n";
var_dump(0 == "a", "10" == "1e1", "abc" <=> "abd");
echo "0.1 + 0.2 = ", 0.1 + 0.2, "  (precise: ", var_export(0.1 + 0.2, true), ")\n\n";

echo "=== arrays & foreach ===\n";
$nums = range(1, 10);
$squares = [];
foreach ($nums as $n) {
    if ($n % 2 === 0) $squares[$n] = $n * $n;
}
print_r($squares);
echo "sum of squares: ", array_sum($squares), "\n";
echo "keys: ", implode(", ", array_keys($squares)), "\n\n";

echo "=== functions, references, recursion ===\n";
function fib(int $n): int { return $n < 2 ? $n : fib($n - 1) + fib($n - 2); }
echo "fib(15) = ", fib(15), "\n";
function addTax(float &$price, float $rate = 0.21): void { $price += $price * $rate; }
$total = 100.0;
addTax($total);
echo "with 21% tax: ", number_format($total, 2), "\n\n";

echo "=== classes, inheritance, late static binding ===\n";
abstract class Shape {
    public function __construct(protected string $name) {}
    abstract public function area(): float;
    public function describe(): string {
        return sprintf("%s has area %.2f", $this->name, $this->area());
    }
}
class Circle extends Shape {
    public function __construct(private float $r) { parent::__construct("Circle"); }
    public function area(): float { return M_PI * $this->r ** 2; }
}
class Rectangle extends Shape {
    public function __construct(private float $w, private float $h) { parent::__construct("Rectangle"); }
    public function area(): float { return $this->w * $this->h; }
}
$shapes = [new Circle(2.0), new Rectangle(3.0, 4.0)];
foreach ($shapes as $s) {
    echo "  ", $s->describe(), " (", get_class($s), ")\n";
}

echo "\n=== traits + magic methods ===\n";
trait Tagged {
    private array $tags = [];
    public function tag(string $t): static { $this->tags[] = $t; return $this; }
    public function __toString(): string { return "[" . implode(", ", $this->tags) . "]"; }
}
class Post { use Tagged; }
$p = (new Post())->tag("php")->tag("rust")->tag("vm");
echo "post tags: $p\n\n";

echo "=== exceptions ===\n";
function parseRatio(string $s): float {
    $parts = explode("/", $s);
    if (count($parts) !== 2) throw new InvalidArgumentException("bad ratio: $s");
    $a = (float)$parts[0];
    $b = (float)$parts[1];
    return $a / $b;  // may throw DivisionByZeroError
}
foreach (["3/4", "10/0", "nope"] as $r) {
    try {
        printf("  %-6s => %.4f\n", $r, parseRatio($r));
    } catch (DivisionByZeroError $e) {
        echo "  $r => division by zero\n";
    } catch (InvalidArgumentException $e) {
        echo "  $r => ", $e->getMessage(), "\n";
    } finally {
        // runs for every iteration
    }
}

echo "\n=== strings, hashing, json ===\n";
$msg = "Hello, phrust!";
echo "upper:  ", strtoupper($msg), "\n";
echo "md5:    ", md5($msg), "\n";
echo "base64: ", base64_encode($msg), "\n";
echo "json:   ", json_encode(["engine" => "phrust", "shapes" => count($shapes), "ok" => true]), "\n";
echo "date:   ", date("Y-m-d", 1760000000), "\n";

echo "\n=== closures & first-class callables ===\n";
$scale = fn(float $factor) => fn(float $x) => $x * $factor;
$double = $scale(2.0);
echo "double(21) = ", $double(21), "\n";
$areas = array_map(fn($s) => round($s->area(), 2), $shapes);
echo "areas: ", implode(", ", $areas), "\n";
$total = array_reduce($areas, fn($c, $a) => $c + $a, 0.0);
echo "total area: ", round($total, 2), "\n";
echo "lengths: ", implode(",", array_map(strlen(...), ["a", "bb", "ccc"])), "\n";

echo "\n=== enums ===\n";
enum Suit: string {
    case Hearts = '♥';
    case Spades = '♠';
    public function color(): string {
        return match ($this) {
            Suit::Hearts => "red",
            Suit::Spades => "black",
        };
    }
}
foreach (Suit::cases() as $s) {
    echo "  {$s->name} ({$s->value}) is {$s->color()}\n";
}
echo "from('♠') = ", Suit::from('♠')->name, "\n";

echo "\n=== named args, sorting, regex ===\n";
function label(string $text, int $width = 10, string $fill = ".") {
    return str_pad($text, $width, $fill);
}
echo "[", label(text: "hi", width: 8), "]\n";
$nums = [5, 3, 8, 1, 9, 2];
usort($nums, fn($a, $b) => $a <=> $b);
echo "sorted: ", implode(",", $nums), "\n";
preg_match('/(\w+)@(\w+)\.(\w+)/', "user@example.com", $m);
echo "email user=", $m[1], " domain=", $m[2], "\n";

echo "\nDone.\n";
