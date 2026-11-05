<?php
echo "start\n";
var_dump(class_exists('D', false), class_exists('B', false), class_exists('C', false));
if (1) { class A { const K = 'k'; public $p = [1]; static function who() { return static::class; } } }
class D extends A {}
class B {}
class C extends B {}
var_dump(class_exists('D', false), D::who(), D::K, (new D)->p);
class_alias('A', 'AA');
class E extends AA { function f() { return parent::who(); } }
var_dump(get_parent_class('E'), (new E)->f(), new E instanceof AA, is_subclass_of('E', 'AA'));
function decl() { class InFn { function hi() { return 'hi'; } } }
var_dump(class_exists('InFn', false));
decl();
var_dump((new InFn)->hi());
interface I0 { function m(); }
class_alias('I0', 'II');
if (true) { class Impl implements II { function m() { return 'm'; } } }
var_dump((new Impl)->m(), new Impl instanceof I0, class_implements('Impl'));
if (false) { class NotEver {} }
var_dump(class_exists('NotEver', false));
if (!class_exists('Maybe', false)) { class Maybe { public $v = 1; } }
var_dump((new Maybe)->v);
spl_autoload_register(function ($c) { echo "autoload($c)\n"; if ($c === 'Base') { class Base { function b() { return 'base'; } } } });
if (true) { class Child extends Base {} }
var_dump((new Child)->b());
trait T { function t() { return 't'; } }
if (true) { class UsesT { use T; } }
var_dump((new UsesT)->t());
if (true) { enum Suit: string { case H = 'h'; } }
var_dump(Suit::H->value, Suit::from('h') === Suit::H);
try { if (true) { class Bad extends Missing {} } } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { if (true) { class Bad2 implements MissingI {} } } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
var_dump(class_exists('Bad', false));
class_alias('A', 'AA2');
var_dump(is_callable('AA2::who'), call_user_func('AA2::who'), call_user_func('\AA2::who'), call_user_func(['AA2', 'who']), method_exists('aa2', 'WHO'), method_exists('AA2', 'nope'));
var_dump(get_parent_class('D'), get_parent_class('A'));
try { get_parent_class('NopeClass'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { get_parent_class(1); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$f = 'D::who'; var_dump($f());
var_dump(array_map('E::who', [1]));
