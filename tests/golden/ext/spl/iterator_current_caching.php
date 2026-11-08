<?php
// spl_dual_it caches the inner iterator's current element once per step;
// RecursiveIteratorIterator reads a user iterator's current() once per
// element (zend_user_iterator); AppendIterator::current() fetches again.
class Tree implements RecursiveIterator {
    private $k = 0;
    public function __construct(private array $a) {}
    public function current(): mixed { echo "current\n"; return (object) ['v' => $this->a[$this->k]]; }
    public function key(): mixed { return $this->k; }
    public function next(): void { $this->k++; }
    public function rewind(): void { $this->k = 0; }
    public function valid(): bool { return $this->k < count($this->a); }
    public function hasChildren(): bool { return is_array($this->a[$this->k]); }
    public function getChildren(): Tree { return new Tree($this->a[$this->k]); }
}
$it = new RecursiveIteratorIterator(new Tree([1, [2, 3], 4]), RecursiveIteratorIterator::SELF_FIRST);
$seen = new WeakMap;
$f = new CallbackFilterIterator($it, function ($v) use ($seen) { if (is_object($v)) { $seen[$v] = 1; } return true; });
foreach ($f as $v) { var_dump(is_object($v) ? isset($seen[$v]) : 'array'); }
foreach ($it as $x) { $a = $it->current(); $b = $it->current(); var_dump($a === $b, $a === $x); break; }
class II implements Iterator {
    private $i = 0;
    function current(): mixed { echo "cur\n"; return new stdClass; }
    function key(): mixed { return $this->i; }
    function next(): void { $this->i++; }
    function rewind(): void { $this->i = 0; }
    function valid(): bool { return $this->i < 2; }
}
$ii = new IteratorIterator(new II);
var_dump($ii->valid());
foreach ($ii as $v) { var_dump($ii->current() === $v); }
$ai = new AppendIterator; $ai->append(new II);
foreach ($ai as $v) { var_dump($ai->current() === $v); }
$ci = new CallbackFilterIterator(new II, fn($x) => true);
foreach ($ci as $v) { var_dump($ci->current() === $v); }
$inf = new LimitIterator(new InfiniteIterator(new ArrayIterator([1, 2])), 0, 5);
foreach ($inf as $k => $v) { echo "$k=>$v "; }
echo "\n";
// RecursiveArrayIterator::hasChildren()/getChildren() read the element
// itself, not through an overriding current().
class ATree extends RecursiveArrayIterator {
    public function current(): mixed { echo "current\n"; return parent::current(); }
}
$it = new RecursiveIteratorIterator(new ATree([1, [2, 3], 4]), RecursiveIteratorIterator::SELF_FIRST);
foreach ($it as $v) { echo is_array($v) ? 'A' : $v, "\n"; }
// RecursiveIteratorIterator over an array iterator reads the element anew.
$rai = new RecursiveArrayIterator(['a' => 1, 'b' => 2]);
$rii = new RecursiveIteratorIterator($rai);
$rii->rewind(); var_dump($rii->current()); $rai['a'] = 'changed'; var_dump($rii->current());
// InfiniteIterator::next() checks valid() once.
class LogIt extends ArrayIterator {
    public function valid(): bool { echo "valid "; return parent::valid(); }
}
$n = 0;
foreach (new InfiniteIterator(new LogIt([1, 2])) as $v) { if (++$n > 3) { break; } echo "$v\n"; }
