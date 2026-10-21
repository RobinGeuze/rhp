<?php
class Money {
    public function __construct(private $amount) {}
    public function __toString() { return '$' . number_format($this->amount, 2); }
}
$m = new Money(1234.5);
echo $m, "\n";
echo "Price: $m!\n";
echo "total = " . $m . " ok\n";

class Magic {
    private $data = [];
    public function __get($name) { return $this->data[$name] ?? "<$name undefined>"; }
    public function __call($name, $args) { return "called $name(" . implode(",", $args) . ")"; }
    public static function __callStatic($name, $args) { return "static $name"; }
}
$x = new Magic();
echo $x->foo, "\n";
echo $x->doStuff(1, 2, 3), "\n";
echo Magic::make(), "\n";

$n = null;
echo ($n?->method() ?? "was null"), "\n";
$obj = new Money(5);
echo $obj?->__toString(), "\n";
