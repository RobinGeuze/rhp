<?php
// #[\Override] that does match something: no error.
interface I { public function i(); }
trait T { abstract public function t(); }
abstract class AP { abstract public function __construct(); }
class P { public function p() {} protected function q() {} private function r() {} }
class C extends P implements I {
    use T;
    #[\Override] public function i() {}
    #[\Override] public function t() {}
    #[\Override] public function p() {}
    #[\Override] public function q() {}
    public function r() {}
}
class D extends AP { #[\Override] public function __construct() {} }
interface J extends I { #[\Override] public function i(); }
abstract class HP { public $h { get => 1; } public int $plain = 0; }
class HC extends HP {
    #[\Override] public $h { #[\Override] get => 2; }
    public int $plain { #[\Override] get => 3; #[\Override] set => $value; }
}
echo (new HC)->h, (new HC)->plain, "\n";
echo "Done\n";
