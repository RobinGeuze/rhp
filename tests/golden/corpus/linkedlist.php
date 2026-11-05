<?php
class LinkedList {
    private ?object $head = null;
    private int $size = 0;
    public function push($val): void {
        $this->head = (object)['val' => $val, 'next' => $this->head];
        $this->size++;
    }
    public function reverse(): array {
        $out = [];
        $cur = $this->head;
        while ($cur !== null) {
            $out[] = $cur->val;
            $cur = $cur->next;
        }
        return $out;
    }
    public function size(): int { return $this->size; }
}
$l = new LinkedList();
foreach (range(1, 5) as $i) $l->push($i);
echo implode(" ", $l->reverse()), "\n";
echo $l->size(), "\n";

// anonymous object manipulation
$p = (object)['x' => 1, 'y' => 2];
$p->z = 3;
echo "{$p->x},{$p->y},{$p->z}\n";
foreach ($p as $k => $v) echo "$k=$v ";
echo "\n";
var_dump((array)$p);
