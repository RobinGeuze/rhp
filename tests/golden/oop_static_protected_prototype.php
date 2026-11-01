<?php
abstract class Base { protected static $s='base'; protected static $only='only'; public static $pub='p'; }
class C1 extends Base { protected static $s='c1'; public static $pub='p1'; }
class C2 extends Base { static function peek(){ return [C1::$s, C1::$only, isset(C1::$s), C1::$s ?? 'd', C1::$pub]; } static function w(){ C1::$s='w'; return C1::$s; } }
var_dump(C2::peek(), C2::w());
class Other { static function peek(){ try { return C1::$s; } catch (Error $e) { return $e->getMessage(); } } }
var_dump(Other::peek(), isset(C1::$s));
class A { private static $p=1; static function g(){ return static::$p; } } class B extends A { private static $p=2; } class B2 extends A {}
try { var_dump(B::g()); } catch (Error $e) { echo $e->getMessage(),"\n"; }
var_dump(B2::g());
trait T { protected static $t='t'; } class TB { protected static $t='tb'; } class TC extends TB { use T; } class TD extends TB { static function r(){ return TC::$t; } }
var_dump(TD::r());
