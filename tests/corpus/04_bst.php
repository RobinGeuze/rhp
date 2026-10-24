<?php
class Node {
    public ?Node $left = null;
    public ?Node $right = null;
    public function __construct(public int $val) {}
}
class BST {
    private ?Node $root = null;
    public function insert(int $v): void {
        $this->root = $this->ins($this->root, $v);
    }
    private function ins(?Node $n, int $v): Node {
        if ($n === null) return new Node($v);
        if ($v < $n->val) $n->left = $this->ins($n->left, $v);
        else $n->right = $this->ins($n->right, $v);
        return $n;
    }
    public function inorder(): array {
        $out = [];
        $this->walk($this->root, $out);
        return $out;
    }
    private function walk(?Node $n, array &$out): void {
        if ($n === null) return;
        $this->walk($n->left, $out);
        $out[] = $n->val;
        $this->walk($n->right, $out);
    }
}
$t = new BST();
foreach ([5, 3, 8, 1, 4, 7, 9, 2, 6] as $v) $t->insert($v);
echo implode(" ", $t->inorder()), "\n";
