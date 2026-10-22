<?php
class Point { public int $x = 3; public int $y = 7; public string $label = "p"; }
// Keep every object alive so object ids are stable (no GC id reuse).
$a = new Point();
$b = new Point();
$b->x = 100;
var_dump($a);
var_dump($b);
$pair = [$a, $b];
var_dump($pair);

class Box { public $items = []; public $name = "box"; }
$box = new Box();
$box->items = [1, 2, 3];
var_dump($box);
