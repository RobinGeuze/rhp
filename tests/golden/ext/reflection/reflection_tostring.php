<?php
// ReflectionClass / ReflectionMethod / ReflectionFunction::__toString
// (php_reflection.c _class_string, _function_string).
function clean($s) { return str_replace(__FILE__, 'F', $s); }

interface I {
    const C = 1;
    function m(int $a = 5): string;
}
enum E: string implements I {
    case A = 'a';
    case B = 'b';
    const X = self::A;
    public function m(int $a = 5): string { return ''; }
    public static function s() {}
}
echo clean(new ReflectionMethod('E', 'cases')), "\n";
echo clean(new ReflectionClassConstant('E', 'X')), "\n";
echo clean(new ReflectionClass('I')), "\n";

/** Doc for AB. */
abstract class AB {
    public static $sp = [1, 2];
    protected ?int $x = null;
    final public const F = 1.5;
    private const P = 'p';
    abstract protected function am();
    private static function ps(&$r, ...$rest) {}
}
#[AllowDynamicProperties]
final class CC extends AB {
    public readonly string $ro;
    protected function am() {}
    public function __construct(public int $promoted = 3) {}
}
echo clean(new ReflectionClass('AB')), "\n";
echo clean(new ReflectionClass('CC')), "\n";

trait T { public function tm() {} }
echo clean(new ReflectionClass('T')), "\n";

$o = new CC;
$o->dyn = 1;
echo clean(new ReflectionObject($o)), "\n";

$v = 1;
echo clean(new ReflectionFunction(function &(int ...$a) use ($v): ?array { return $a; })), "\n";
echo clean(new ReflectionFunction(strlen(...))), "\n";
echo clean(new ReflectionMethod('CC', 'am')), "\n";
echo clean(new ReflectionMethod('E', 'm')), "\n";
