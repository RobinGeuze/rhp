<?php
class Range implements Iterator {
    private int $cur;
    public function __construct(private int $start, private int $end, private int $step = 1) {
        $this->cur = $start;
    }
    public function current(): mixed { return $this->cur; }
    public function key(): mixed { return ($this->cur - $this->start) / $this->step; }
    public function next(): void { $this->cur += $this->step; }
    public function rewind(): void { $this->cur = $this->start; }
    public function valid(): bool { return $this->cur <= $this->end; }
}
foreach (new Range(0, 10, 2) as $k => $v) {
    echo "$k=>$v ";
}
echo "\n";
$sum = 0;
foreach (new Range(1, 100) as $v) $sum += $v;
echo $sum, "\n";

class Bag implements Countable, ArrayAccess {
    private array $data = [];
    public function offsetExists(mixed $o): bool { return isset($this->data[$o]); }
    public function offsetGet(mixed $o): mixed { return $this->data[$o] ?? null; }
    public function offsetSet(mixed $o, mixed $v): void {
        if ($o === null) $this->data[] = $v; else $this->data[$o] = $v;
    }
    public function offsetUnset(mixed $o): void { unset($this->data[$o]); }
    public function count(): int { return count($this->data); }
}
$b = new Bag();
$b[] = "x"; $b['k'] = "y";
echo $b['k'], " ", count($b), " ", isset($b['k']) ? 'y' : 'n', "\n";
unset($b['k']);
echo count($b), "\n";
