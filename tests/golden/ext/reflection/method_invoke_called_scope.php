<?php
trait HasEv { public static function bootHasEv() { echo "boot ", static::class, "\n"; } public function inst() { echo "inst ", static::class, " ", get_class($this), "\n"; } }
class Model { use HasEv; public static function s() { echo "s ", static::class, "\n"; } }
class Child extends Model {}
foreach ((new ReflectionClass(Child::class))->getMethods() as $m) {
    if ($m->isStatic()) { $m->invoke(null); $m->invokeArgs(null, []); }
    else { $m->invoke(new Child); }
    echo $m->class, "\n";
}
(new ReflectionMethod(Child::class, 's'))->invoke(null);
(new ReflectionMethod(Model::class, 's'))->invoke(null);
(new ReflectionMethod(Child::class, 's'))->getClosure()();
(new ReflectionMethod(Child::class, 's'))->invoke(new Model);
