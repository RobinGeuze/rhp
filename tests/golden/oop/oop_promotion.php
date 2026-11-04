<?php
class Point {
    public function __construct(public $x, public $y, protected $label = "pt") {}
    public function str() { return "$this->label($this->x,$this->y)"; }
}
$p = new Point(3, 4);
echo $p->x, ",", $p->y, "\n";
echo $p->str(), "\n";
