<?php
// A constant array as a property name is converted while compiling: the
// warning comes once, at compile time, however often the code runs.
$o = new stdClass;
for ($i = 0; $i < 3; $i++) {
    $o->{[]} = $i;
    $o->{[1, 2]} .= 'x';
}
var_dump($o, isset($o->{[]}), $o->{[]});
unset($o->{[]});
var_dump($o);
// Method names keep the runtime path.
class M { function __call($n, $a) { echo "$n\n"; } }
$m = new M;
$m->{'x' . 'y'}();
