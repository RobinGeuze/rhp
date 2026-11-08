<?php
// spl_dllist.c: iterator mode, prev(), serialize(), the frozen modes of
// SplStack/SplQueue, offsets and a traverse pointer past a removed element.
$d = new SplDoublyLinkedList;
foreach ([1, 2, 3, 4] as $v) { $d->push($v); }
var_dump($d->getIteratorMode(), $d->setIteratorMode(SplDoublyLinkedList::IT_MODE_LIFO));
for ($d->rewind(); $d->valid(); $d->next()) { echo $d->key(), '=', $d->current(), "\n"; }
$d->rewind(); $d->next(); $d->prev(); var_dump($d->current());
$d->setIteratorMode(0);
var_dump($d->serialize(), $d[1]);
$d->add(1, 'x'); $d->unshift(0); var_dump(iterator_to_array($d));
try { $d->offsetGet(10); } catch (OutOfRangeException $e) { echo $e->getMessage(), "\n"; }
try { $d['a'] = 1; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$d->rewind(); $d->pop(); $d->rewind(); $d->offsetUnset(0); var_dump($d->valid(), $d->current());
$e = new SplDoublyLinkedList; $e->unserialize('i:1;:s:1:"a";:s:1:"b";'); var_dump($e->getIteratorMode(), $e->count());
try { $e->unserialize('i:1;:x'); } catch (UnexpectedValueException $x) { echo $x->getMessage(), "\n"; }
try { (new SplStack)->top(); } catch (RuntimeException $x) { echo $x->getMessage(), "\n"; }
$q = new SplQueue; var_dump($q->getIteratorMode());
try { $q->setIteratorMode(SplDoublyLinkedList::IT_MODE_LIFO); } catch (RuntimeException $x) { echo $x->getMessage(), "\n"; }
$s = new SplStack; $s->push('a'); $s->push('b'); var_dump($s->getIteratorMode(), iterator_to_array($s));
class MyStack extends SplStack { function __construct() {} }
$m = new MyStack; $m->push(1); $m->push(2); var_dump($m->getIteratorMode(), unserialize(serialize($m)));
var_dump(method_exists('SplDoublyLinkedList', 'toArray'), (new ReflectionClass('SplStack'))->getConstructor());
