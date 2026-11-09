<?php
// The lazy object initializer is php's fcc: resolved where newLazyGhost()
// was called; getLazyInitializer() returns the callable as passed.
class K {
    public $a;
    static function init($o) { $o->a = 1; }
    private static function pinit($o) { $o->a = 2; }
    function g() {
        $r = new ReflectionClass('K');
        $o = $r->newLazyGhost(['self', 'init']);
        var_dump($r->getLazyInitializer($o));
        $p = $r->newLazyGhost('K::pinit');
        var_dump($r->getLazyInitializer($p));
        return [$o, $p];
    }
}
[$o, $p] = (new K)->g();
var_dump($o->a, $p->a);
foreach (['newLazyGhost', 'newLazyProxy'] as $m) {
    try { (new ReflectionClass('K'))->$m('nope'); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}
try { (new ReflectionClass('K'))->resetAsLazyGhost(new K, 'nope'); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
