<?php
class A {
    function f(?A $a, (A&Countable)|null $b, A&Countable $c, A&Countable $d = null) {}
}
class B extends A {
    function f() {}
}
