<?php
// A `callable` parameter is checked in the callee's scope: its own private
// methods count, the caller's do not.
class E {
    public function set(?callable $h) { return is_callable($h) ? 'callable here' : 'accepted'; }
    private function priv() {}
    public function mk() { return [$this, 'priv']; }
}
class Other {
    private function mine() {}
    public function pass(E $e) {
        try {
            return $e->set([$this, 'mine']);
        } catch (TypeError $t) {
            return get_class($t);
        }
    }
}
function f(callable $c) { return 'ok'; }

$e = new E;
var_dump($e->set($e->mk()));
var_dump((new Other)->pass($e));
try {
    f($e->mk());
} catch (TypeError $t) {
    echo get_class($t), "\n";
}
