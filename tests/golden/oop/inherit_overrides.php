<?php
ini_set('display_errors', '0');
register_shutdown_function(function () {
    $e = error_get_last();
    echo $e['type'], ': ', $e['message'], ' (', basename($e['file']), ':', $e['line'], ")\n";
});
// An override may not narrow the parent's visibility; a static
// property's type must match the parent's.
class A {
    public static int $n = 1;
    protected function f() {}
}
class B extends A {
    public static int $n = 2;
    public function f() {} // widening is fine
}
echo B::$n, "\n";
interface I { function g(self $x): static; }
final class C implements I {
    function g(I $x): self { return $this; } // self for static in a final class
}
echo get_class((new C)->g(new C)), "\n";
abstract class P { abstract function __construct(int $a); }
class Q extends P { function __construct(int $a, $b = 0) {} }
echo "ok\n";
if (true) {
    // Checked against P's abstract constructor, not Q's.
    class R extends Q { function __construct(string $a) {} }
}
