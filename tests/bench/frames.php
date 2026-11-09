<?php
// An analyser-shaped walk: a visitor recursing over a tree of node
// objects, each call a method with typed object parameters and return
// types, several locals and a short-lived array, as PHPStan's resolver
// does (frame setup and teardown, type checks by class name).
interface Node { public function kids(): array; }
abstract class Base implements Node {
    public function __construct(public array $kids = []) {}
    public function kids(): array { return $this->kids; }
}
final class Leaf extends Base {}
final class Pair extends Base {}
final class Scope {
    public function __construct(public int $depth = 0) {}
    public function enter(): Scope { return new Scope($this->depth + 1); }
}
final class Walker {
    public int $seen = 0;
    public function walk(Node $n, Scope $s): int {
        $kids = $n->kids();
        $sum = $s->depth;
        $inner = $s->enter();
        foreach ($kids as $k) {
            $sum += $this->visit($k, $inner);
        }
        return $sum;
    }
    private function visit(Node $n, Scope $s): int {
        $this->seen++;
        $info = ['depth' => $s->depth, 'leaf' => $n instanceof Leaf];
        return $info['leaf'] ? $this->leaf($n, $s) : $this->walk($n, $s);
    }
    private function leaf(Leaf $l, Scope $s): int { return $s->depth & 3; }
}
function tree(int $d): Node {
    return $d === 0 ? new Leaf() : new Pair([tree($d - 1), tree($d - 1), new Leaf()]);
}
$t = tree(8);
$w = new Walker();
$r = 0;
for ($i = 0; $i < 300; $i++) {
    $r = ($r + $w->walk($t, new Scope())) & 0xffffff;
}
echo $r, ' ', $w->seen, "\n";
