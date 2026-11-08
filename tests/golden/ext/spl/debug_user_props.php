<?php
// The debug view lists a subclass's own properties before the storage.
class MyHeap extends SplMinHeap { public $tag = 'h'; }
class MyQueue extends SplPriorityQueue { protected $tag = 'q'; }
class MyMulti extends MultipleIterator { private $tag = 'm'; }
$h = new MyHeap; $h->insert(2);
$q = new MyQueue; $q->insert('a', 1);
$m = new MyMulti; $m->attachIterator(new ArrayIterator([1]));
var_dump($h, $q, $m);
print_r($h); echo "\n";
