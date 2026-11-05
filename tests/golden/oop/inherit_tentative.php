<?php
// Overriding internal methods with tentative return types: the deprecation,
// its #[\ReturnTypeWillChange] suppression, and when the error handler sees it.
set_error_handler(function ($no, $msg, $file, $line) {
    echo "handler($no, line $line): $msg\n";
    return true;
});

echo "start\n";

if (true) {
    class It implements Iterator {
        private $i = 0;
        function current() { return $this->i; }
        function key(): mixed { return $this->i; }
        function next(): void { $this->i++; }
        #[\ReturnTypeWillChange]
        function rewind() { $this->i = 0; }
        function valid(): bool { return $this->i < 2; }
    }
}
foreach (new It as $k => $v) {
    echo "$k => $v\n";
}

if (true) {
    class Arr implements ArrayAccess, Countable, JsonSerializable {
        function offsetExists($o): bool { return true; }
        function offsetGet($o): mixed { return $o; }
        function offsetSet($o, $v) {}
        function offsetUnset($o): void {}
        function count() { return 3; }
        function jsonSerialize(): array { return [1]; }
    }
}
$a = new Arr;
echo count($a), " ", $a['x'], " ", json_encode($a), "\n";

// A covariant override of a tentative type is fine.
if (true) {
    class Agg implements IteratorAggregate {
        function getIterator(): ArrayIterator { return new ArrayIterator([7]); }
    }
}
foreach (new Agg as $v) {
    echo $v, "\n";
}

// Anonymous classes with a parent link when `new class` runs.
echo "before anon\n";
$o = new class extends ArrayIterator {
    function count() { return 0; }
};
echo get_parent_class($o), "\n";
echo "end\n";
