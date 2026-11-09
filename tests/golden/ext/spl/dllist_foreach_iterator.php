<?php
// SplDoublyLinkedList's get_iterator: each foreach has its own position;
// a subclass overriding current() iterates through its methods instead.
$l = new SplDoublyLinkedList; $l->push(1); $l->push(2); $l->push(3);
foreach ($l as $k => $a) foreach ($l as $j => $b) echo "$k$a$j$b "; echo "\n";
$l->rewind(); $l->next();
foreach ($l as $a) { echo $a, $l->current(), $l->key(), " "; } echo "\n";
$l->setIteratorMode(SplDoublyLinkedList::IT_MODE_LIFO | SplDoublyLinkedList::IT_MODE_DELETE);
foreach ($l as $k => $a) { echo "$k=$a(", count($l), ") "; } echo count($l), "\n";
try { foreach ($l as &$r) {} } catch (Error $e) { echo $e->getMessage(), "\n"; }
$s = new SplStack; $s[] = 1; $s[] = 2;
foreach ($s as $k => $a) foreach ($s as $b) echo "$k$a$b "; echo "\n";
var_dump(iterator_to_array($s));
$q = new SplQueue; $q[] = 'a'; $q[] = 'b';
foreach ($q as $k => $v) { echo "$k$v "; $q[] = 'c'; if (count($q) > 4) break; } echo "\n";
class D extends SplDoublyLinkedList { function current(): mixed { return 'x' . parent::current(); } }
$d = new D; $d->push(1); $d->push(2);
foreach ($d as $a) foreach ($d as $b) echo "$a$b "; echo "\n";
