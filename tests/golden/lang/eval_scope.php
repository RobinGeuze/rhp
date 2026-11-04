<?php
// eval() returning values and closures, eval of a close tag followed by a
// php file (Composer's FilesystemRepository), and eval running in the
// caller's scope.
echo eval('return 1 + 2;'), "\n";
$fn = eval('return function($v){ return $v * 2; };');
echo $fn(21), "\n";
$content = "<?php return array('a' => 1, 'b' => 2);";
$d = eval('?>' . $content);
echo json_encode($d), "\n";
function g() { $x = 5; eval('$x = $x * 10; $y = 99;'); return "$x,$y"; }
echo g(), "\n";
var_dump(eval('$z = 1;'));
