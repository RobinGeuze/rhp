<?php
// A concrete class must implement inherited abstract methods and hooks
// (zend_verify_abstract_class); the fatal error comes when it is declared.
ob_start(fn ($s) => str_replace(__FILE__, "F", $s));
echo "start\n";
interface Shape {
    public function area(): float;
    public string $name { get; }
}
abstract class Base implements Shape {
    abstract protected function sides(): int;
}
if (true) {
    class Square extends Base {
        public function area(): float { return 1.0; }
    }
}
echo "unreachable\n";
