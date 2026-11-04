<?php
// A function the compiler cannot see at the call site (declared
// conditionally, or in another file): whether an argument goes by
// reference is asked at run time (php's SEND_VAR_EX).
if (true) {
    function set_ref(&$x, $v = 1) { $x = $v; return 'set'; }
    function by_value($x) { $x = 99; return $x; }
    function nested(&$out = null) { return set_ref($out, [1, 2]); }
}
var_dump(set_ref($a), $a);
$b = 5;
var_dump(by_value($b), $b);
var_dump(nested($c), $c);
function scope() {
    set_ref($local, 'in function');
    var_dump($local);
    [$x] = [nested($d)];
    var_dump($x, $d);
}
scope();
$arr = [];
set_ref($arr['k'], 'key');
var_dump($arr);
