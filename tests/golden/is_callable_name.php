<?php
// is_callable()'s third parameter: zend_get_callable_name_ex.
class A {
    public function m() {}
    public static function s() {}
    public function __invoke() {}
    public function closures() {
        foreach ([function () {}, static fn() => 1, $this->m(...), strlen(...)] as $c) {
            var_dump(is_callable($c, false, $name), $name);
        }
    }
}
$cases = ['strlen', 'nope', ['A', 's'], [new A, 'm'], ['A', 'x'], [1, 2], [1], new A,
    42, null, 1.5, true, 'A::s', [new A, 3], new stdClass];
foreach ($cases as $c) {
    var_dump(is_callable($c, false, $name), $name);
    is_callable($c, true, $name);
    var_dump($name);
}
(new A)->closures();
class B {
    public $n;
    public function t() { is_callable('strlen', false, $this->n); return $this->n; }
}
var_dump((new B)->t());
