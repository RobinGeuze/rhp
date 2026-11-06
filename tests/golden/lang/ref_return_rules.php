<?php
// `=&` from calls and `return` in by-reference functions.

set_error_handler(function ($no, $msg, $file, $line) {
    echo "E$no: $msg (line $line)\n";
    return true;
});

function byval() { return 'v'; }

class C {
    public static $p = 'p';
    public $items = [];
    public function &items() { return $this->items; }
    public function &nothing() {}
    public function &expr() { return $this->items + [9]; }
}

// a by-value result is assigned through the existing reference
$a = 'a';
$b = &$a;
$b =& byval();
var_dump($a, $b);

$name = 'dyn';
$$name =& byval();
var_dump($dyn);

$arr = [];
$arr['k'] =& byval();
var_dump($arr);

C::$p =& byval();
var_dump(C::$p);

// a reference result binds
$o = new C;
$it =& $o->items();
$it[] = 1;
$o->items()[] = 2;
var_dump($o->items);

// implicit and expression returns notice and still return
$n =& $o->nothing();
var_dump($n);
$e =& $o->expr();
var_dump($e);

// a typed by-reference return coerces the variable itself
function &counter(): int {
    static $v = "5";
    return $v;
}
$c =& counter();
var_dump($c);
$c++;
var_dump(counter());

// a by-reference generator's return does not notice
function &gen() {
    $x = 1;
    yield $x;
    return 3;
}
$g = gen();
foreach ($g as &$y) {}
var_dump($g->getReturn());
