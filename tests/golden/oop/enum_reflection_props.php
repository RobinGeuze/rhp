<?php
// An internal backed enum creates only the case used; Reflection lists an
// enum's name/value as declared properties.
var_dump(spl_object_id(PropertyHookType::Set));
var_dump(spl_object_id(new stdClass));
enum E { case A; }
enum F: int { case A = 1; }
foreach ([E::class, F::class] as $c) {
    foreach ((new ReflectionClass($c))->getProperties() as $p) {
        echo $p;
    }
    var_dump((new ReflectionClass($c))->hasProperty('value'));
}
