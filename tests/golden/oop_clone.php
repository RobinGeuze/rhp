<?php
class Box {
    public $items = [];
    public function add($x) { $this->items[] = $x; return $this; }
}
$a = new Box();
$a->add(1)->add(2);
$b = clone $a;
$b->add(3);
echo count($a->items), " ", count($b->items), "\n";
var_dump($a === $b);
var_dump($a === $a);

class Secret {
    private $pw = "hunter2";
    public function reveal() { return $this->pw; }
}
$s = new Secret();
echo $s->reveal(), "\n";
