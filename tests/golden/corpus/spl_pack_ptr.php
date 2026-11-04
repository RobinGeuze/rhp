<?php
// SPL data structures.
$q = new SplQueue();
$q->enqueue('a'); $q->enqueue('b'); $q->push('c');
echo $q->count(), ' ', $q->dequeue(), ' ', $q->top(), ' | ';
foreach ($q as $x) { echo $x; }
echo "\n";

$s = new SplStack();
$s->push(1); $s->push(2); $s->push(3);
foreach ($s as $x) { echo $x; }
echo "\n";

$pq = new SplPriorityQueue();
$pq->insert('low', 1); $pq->insert('high', 10); $pq->insert('mid', 5);
while (!$pq->isEmpty()) { echo $pq->extract(), ' '; }
echo "\n";

$fa = SplFixedArray::fromArray([10, 20, 30]);
$fa[1] = 99;
echo $fa->getSize(), ' ', $fa[0], ' ', $fa[1], ' ', $fa[2], "\n";

// pack / unpack.
echo bin2hex(pack('Nn', 258, 1)), ' ', bin2hex(pack('C*', 1, 2, 255)), "\n";
$u = unpack('Nlen/a*rest', "\x00\x00\x00\x02abXY");
echo $u['len'], ' ', $u['rest'], "\n";
print_r(array_values(unpack('C*', "\x01\x02\xff")));

// array internal pointer.
$a = [5, 10, 15];
echo end($a), ' ', key($a), ' ', prev($a), ' ', reset($a), ' ', next($a), "\n";

// (array) cast key mangling for private/protected properties.
class Box {
    private $secret = 's';
    protected $shared = 'h';
    public $open = 'o';
}
$arr = (array) new Box();
$keys = array_map(fn($k) => str_replace("\0", '#', $k), array_keys($arr));
echo implode(',', $keys), "\n";
