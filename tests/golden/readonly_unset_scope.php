<?php
// unset() of readonly properties: an initialized one never; an
// uninitialized one under the (implicit protected(set)) set visibility.
function attempt(callable $f) {
    try {
        $f();
        echo "ok\n";
    } catch (Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
class P { public readonly int $x; }
class Kid extends P { function u() { unset($this->x); } }
class Other { function u($o) { unset($o->x); } }
class Sib extends P { function u($o) { unset($o->x); } }
class Exp { public protected(set) readonly int $x; }
class Priv { public private(set) readonly int $x; function __construct() { $this->x = 1; } }

attempt(fn() => (new Kid)->u());
attempt(function () { $p = new P; unset($p->x); });
attempt(fn() => (new Other)->u(new P));
attempt(fn() => (new Sib)->u(new Kid));
attempt(function () { $e = new Exp; unset($e->x); });
attempt(function () { $e = new Exp; $e->x = 1; });
attempt(function () { $p = new Priv; unset($p->x); });
