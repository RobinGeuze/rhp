<?php
spl_autoload_register('A::l'); spl_autoload_register(['A','l']); var_dump(count(spl_autoload_functions()));
spl_autoload_unregister(['A','l']); var_dump(count(spl_autoload_functions()));
class A { static function l($c){ echo "A::l $c\n"; } private function p($c) { echo "p $c\n"; } function reg() { spl_autoload_register([$this, 'p']); } function __call($n,$a){ echo "call $n\n"; } }
$a = new A; $a->reg(); new Foo1;
var_dump(spl_autoload_functions());
var_dump(spl_autoload_unregister([$a, 'p']));
spl_autoload_register([$a, 'zz']); spl_autoload_register([$a, 'zz']); spl_autoload_register([$a, 'ZZ']);
var_dump(spl_autoload_functions()); new Foo2;
$f = function($c) { echo "f $c\n"; }; spl_autoload_register($f); spl_autoload_register($f);
spl_autoload_register('strlen'); spl_autoload_register('StrLen');
var_dump(count(spl_autoload_functions()), spl_autoload_unregister(function(){}), spl_autoload_unregister($f));
var_dump(spl_autoload_register('l', false));
function l($c) { echo "l $c\n"; spl_autoload_unregister('l'); }
spl_autoload_register('L', true, true);
class_exists('Foo3'); class_exists('Foo4');
try { spl_autoload_register('spl_autoload_call'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
spl_autoload_unregister('spl_autoload_call'); var_dump(spl_autoload_functions());
