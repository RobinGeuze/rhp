<?php
class Foo {
    private function method() { return __METHOD__; }
    private static function sm() { return static::class; }
    public function bar() { return $this->method(...); }
    public static function baz() { return static::sm(...); }
    public function selfm() { return self::sm(...); }
}
class Sub extends Foo {}
$foo = new Foo;
$bar = $foo->bar(...);
echo ($bar())(), "\n";
$u = Sub::baz(); echo $u(), "\n";
$t = (new Sub)->selfm(); echo $t(), "\n";
$s = strlen(...); echo $s("abc"), "\n";
echo implode(",", array_map(($foo->bar()), [])), "|\n";
