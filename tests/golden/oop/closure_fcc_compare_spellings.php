<?php
// zend_closure_compare: first-class callables of one function compare
// equal however the callable was spelled.
class I { function __invoke() {} function m() {} static function s() {} }
class J extends I {}
$o = new I;
var_dump('I::s'(...) == I::s(...));
var_dump(['I', 's'](...) == I::s(...));
var_dump(Closure::fromCallable('i::S') == I::s(...));
var_dump('\I::s'(...) == I::s(...));
var_dump('J::s'(...) == I::s(...));
var_dump([$o, 'm'](...) == $o->m(...));
var_dump($o(...) == [$o, '__invoke'](...));
var_dump('strlen'(...) == strlen(...));
var_dump('strlen'(...) == 'trim'(...));
