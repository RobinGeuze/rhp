<?php
// `$x =& $this` must not let `$x = null` null out $this (php protects it);
// the object handle is still shared, so method calls through $x work.
class Node {
    public $tag;
    public function __construct($tag) { $this->tag = $tag; }
    public function label() { return "<{$this->tag}>"; }

    // Mirrors react/promise Promise::call(): alias $this, run a callback that
    // nulls the alias, then still use $this.
    public function run(callable $cb) {
        $target =& $this;
        try {
            $cb(function () use (&$target) {
                if ($target !== null) { $target = null; }
            });
            throw new RuntimeException("boom");
        } catch (\Throwable $e) {
            $target = null;              // writes through the alias
            return $this->label() . ":" . $e->getMessage();
        }
    }
}

$n = new Node("x");
echo $n->run(function ($settle) { $settle(); }), "\n";

// Plain case: aliasing and nulling the alias leaves $this intact.
class Box {
    public $v = 7;
    public function f() {
        $t =& $this;
        $t = null;
        return $this === null ? "NULL" : ("ok:" . $this->v);
    }
}
echo (new Box())->f(), "\n";
