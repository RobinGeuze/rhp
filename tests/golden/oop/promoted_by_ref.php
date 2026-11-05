<?php
// A by-reference promoted parameter binds the property to the argument.
class Box {
    public function __construct(public array &$items, public int $n = 0) {}
}
$items = [1];
$b = new Box($items);
$items[] = 2;
$b->items[] = 3;
var_dump($b->items, $items, $b->n);
