<?php
class Stack {
    public array $items = [10, 20, 30];
    public function pop() { return array_pop($this->items); }
    public function push($v) { array_push($this->items, $v); }
    public function sortIt() { sort($this->items); }
}
$s = new Stack();
echo $s->pop(), "|", implode(",", $s->items), "\n";
$s->push(99); $s->push(5);
echo implode(",", $s->items), "\n";
$s->sortIt();
echo implode(",", $s->items), "\n";

$data = ['nums' => [3, 1, 2]];
sort($data['nums']);
array_push($data['nums'], 99);
echo implode(",", $data['nums']), "\n";

$grid = [[3, 1], [2, 4]];
sort($grid[0]);
echo implode(",", $grid[0]), " ", implode(",", $grid[1]), "\n";

$q = ['a', 'b', 'c'];
$first = array_shift($q);
array_unshift($q, 'z');
echo "$first ", implode(",", $q), "\n";
