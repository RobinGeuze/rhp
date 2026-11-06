<?php
namespace Foo;
use Foo\Bar;
const C = 1;
class A {
    const X = 1;
    function f($a = C, $b = \Foo\C, $c = Bar::X, $d = \Foo\Bar::X, $e = namespace\C, $f = self::X, $g = PHP_EOL, $h = Baz::X) {}
}
foreach ((new \ReflectionMethod(A::class, 'f'))->getParameters() as $p) {
    var_dump($p->getDefaultValueConstantName());
}
// class_alias onto a taken name: the warning names the aliased class.
set_error_handler(function ($no, $msg) {
    echo str_replace(__FILE__, 'F', $msg), "\n";
    return true;
});
var_dump(class_alias('stdClass', 'Foo\A'));
var_dump(class_alias('Foo\A', 'stdclass'));
var_dump(class_alias('NoSuchClass', 'X'));
// A protected destructor runs when released in a scope that may call it.
class P { protected function __destruct() { echo __METHOD__, "\n"; } }
class Q extends P { static function make() { $o = new Q; unset($o); echo "after\n"; } }
Q::make();
// An exception from a destructor reports the releasing line.
class E { function __destruct() { throw new \Exception("dtor"); } }
try {
    $e = new E;
    unset($e);
} catch (\Exception $x) {
    echo $x->getMessage(), " ", $x->getTrace()[0]['line'], "\n";
}
// Override satisfied in an enum by an interface; engine cases() wins
// over a trait's.
interface HasLabel { public function label(): string; }
trait Cases { public static function cases(): array { return []; } }
enum S implements HasLabel {
    use Cases;
    case A;
    #[\Override] public function label(): string { return 'a'; }
}
var_dump(count(S::cases()), S::A->label());
