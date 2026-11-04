<?php
class R { const A = self::B . 'a'; const B = 'b'; const C = [self::A, PHP_INT_SIZE > 0]; }
$r = new ReflectionClass('R');
var_dump($r->getConstants());
var_dump($r->getConstant('A'));
var_dump(constant('R::C'));
var_dump(defined('R::A'), defined('R::Z'));
enum E: string { case X = 'x' . 'y'; case Y = R::B; }
var_dump(constant('E::Y')->value, E::from('b'), E::tryFrom('zz'));
var_dump(array_map(fn($c) => $c->value, E::cases()));
