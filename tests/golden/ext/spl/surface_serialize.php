<?php
// The old Serializable payloads of ArrayObject, ArrayIterator and
// SplObjectStorage, their __debugInfo(), and the SplHeap /
// SplPriorityQueue / SplFixedArray serialization of php 8.5.
$a = new ArrayObject(['k' => 'v', 2 => [1]]);
$a->prop = 1;
var_dump($a->serialize(), $a->__debugInfo());
$i = new ArrayIterator([1, 2]); var_dump($i->serialize(), $i instanceof Serializable);
$o = new stdClass; $s = new SplObjectStorage; $s[$o] = 'info'; $s[new stdClass] = $o;
var_dump($s->serialize());
$h = new SplMinHeap; foreach ([5, 1, 3] as $v) { $h->insert($v); }
var_dump(serialize($h), unserialize(serialize($h))->top());
$p = new SplPriorityQueue; $p->insert('x', 1); $p->insert('y', 3); $p->setExtractFlags(SplPriorityQueue::EXTR_BOTH);
$u = unserialize(serialize($p)); var_dump($u->getExtractFlags(), $u->extract());
try { (new SplMaxHeap)->__unserialize([[], ['flags' => 1, 'heap_elements' => []]]); } catch (Exception $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { (new SplPriorityQueue)->__unserialize([[], ['flags' => 1, 'heap_elements' => [[1]]]]); } catch (Exception $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
$f = SplFixedArray::fromArray([1, 2]); $f->dyn = 'd';
var_dump($f, (array) $f, get_object_vars($f), method_exists($f, '__debugInfo'));
