<?php
// A compound assignment frees its operand after the store (php frees
// OP_DATA last), even when the store runs user code.
class D {
    public function __construct(public int $n) {}
    public function __destruct() { echo "~{$this->n}\n"; }
    public function __toString(): string { return "D{$this->n}"; }
}
class M {
    public function __get($n) { echo "__get $n\n"; return "g"; }
    public function __set($n, $v) { echo "__set $n $v\n"; }
}
class A implements ArrayAccess {
    public function offsetGet($o): mixed { echo "get $o\n"; return "v"; }
    public function offsetSet($o, $v): void { echo "set $o $v\n"; }
    public function offsetExists($o): bool { return true; }
    public function offsetUnset($o): void {}
}

$m = new M;
$m->p .= new D(1);
echo "--\n";
$a = new A;
$a['k'] .= new D(2);
echo "--\n";
$s = new D(3);
$s .= new D(4);
echo "--\n";
$arr = ['q' => 's'];
$arr['q'] .= new D(5);
echo "--\n";
// A plain assignment's operand goes before the store.
$m->p = new D(6) . "";
$a['k'] = new D(7) . "";
echo "end\n";
// `$x = $x . ...` is a plain assignment: the operand goes first.
$x = new D(8);
$x = $x . new D(9);
echo "done\n";
