<?php
// A write through a property of a non-object (zend_throw_non_object_error):
// "assign" when the property itself is the target, "modify" when the write
// goes further down; true/false are named as such.
function t($f) {
    try {
        $f();
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
t(function () { $x = null; $x->y[] = 1; });
t(function () { $x = 5; $x->y[] = 1; });
t(function () { $f = false; $f->p['a'] = 1; });
t(function () { $f = true; $f->p[] = 1; });
t(function () { $s = 'str'; $s->p['k'] = 1; });
t(function () { $a = [1]; $a->p[] = 1; });
$o = new stdClass;
$o->i = 5;
t(function () use ($o) { $o->i->deep = 2; });
t(function () use ($o) { $o->i->deep['x'] = 2; });
$a = ['x' => null];
t(function () use (&$a) { $a['x']->p[] = 1; });
t(function () use (&$a) { $a['x']->p = 1; });
