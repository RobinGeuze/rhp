<?php
// Reflection subclasses with forged properties, every method called with
// garbage arguments: each call returns or throws, and the script finishes.
error_reporting(0);
trait Forge { function forge($slot, $v) { $this->$slot = $v; return $this; } }
class F extends ReflectionFunction { use Forge; }
class M extends ReflectionMethod { use Forge; }
class C extends ReflectionClass { use Forge; }
class O extends ReflectionObject { use Forge; }
$vals = [null, 0, -1, PHP_INT_MAX, '', "\0", 'stdClass', [], [1], ['stdClass'],
         [1, 2, 3], new stdClass, function () {}, [new stdClass, 'x']];
$subjects = [
    [F::class, ['strlen'], ['__i', '__l']],
    [M::class, ['ArrayObject', 'count'], ['__i', '__l']],
    [C::class, ['ArrayObject'], ['__c', '__o']],
    [O::class, [new ArrayObject([1])], ['__c', '__o']],
];
foreach ($subjects as [$cls, $ctor, $slots]) {
    foreach ($slots as $slot) {
        foreach ($vals as $v) {
            $o = (new $cls(...$ctor))->forge($slot, $v);
            foreach (get_class_methods($o) as $m) {
                if (in_array($m, ['forge', '__construct'], true)) { continue; }
                foreach ([[], [$v], [$v, $v]] as $args) {
                    try { $o->$m(...$args); } catch (Throwable $e) {}
                }
            }
        }
    }
}
echo "ok\n";
