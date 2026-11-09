<?php
// A method argument fetched by SEND_FUNC_ARG (callee unknown at compile
// time) leaves no copy behind: its object is freed when the call returns,
// even when an echo statement (no temp clearing) runs the call.
class U {
    function m($a) { return 1; }
    function r(&$a) { $a = 5; return 2; }
}
class G {
    function __get($n) { return new stdClass; }
}
function ids() {
    $x = new stdClass;
    $y = new stdClass;
    echo " ", spl_object_id($x), ",", spl_object_id($y), "\n";
}
$u = new U;
$g = new G;
$a = [new stdClass];
$x = new stdClass; $y = new stdClass; unset($x, $y);
echo $u->m($g->p); ids();
echo $u->m($a[0]); ids();
$o = new stdClass;
$o->p = 1;
$b = [1, [2]];
echo $u->r($o->p), $u->r($b[1][0]), $u->r($b[2]); ids();
$m = 'r';
echo $u->$m($o->q), $u->$m($b[]); ids();
var_dump($o, $b);
