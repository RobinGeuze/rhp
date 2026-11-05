<?php
function sum(...$nums) { return array_sum($nums); }
$args = [1, 2, 3, 4];
echo sum(...$args), "\n";
echo sum(10, ...$args, ...[5, 6]), "\n";
echo max(...[3, 7, 2]), "\n";
class Vec {
    public function __construct(public $x, public $y, public $z) {}
    public function __toString() { return "($this->x,$this->y,$this->z)"; }
    public static function of(...$c) { return new self(...$c); }
}
$coords = [1, 2, 3];
$v = new Vec(...$coords);
echo $v, "\n";
echo Vec::of(...[4, 5, 6]), "\n";
echo implode(",", [...[1,2], ...[3,4]]), "\n";
