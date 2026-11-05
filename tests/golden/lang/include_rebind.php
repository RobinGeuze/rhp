<?php
// An included file shares the including scope's variables: re-binding one
// there (`=&`, `global`, catch) or unsetting it is visible to the
// includer afterwards, at top level and inside a function.
$dir = sys_get_temp_dir() . '/rhp_inc_rebind_' . getmypid();
@mkdir($dir);
file_put_contents("$dir/b.php", '<?php
global $gb;
$gb = "GB";
unset($toUnset);
$refd = &$target;
$target = "T";
try { throw new Exception("caught in include"); } catch (Exception $caught) {}
unset($resetMe);
$resetMe = "fresh";
');
$toUnset = 5;
$resetMe = 'old';
$keep = &$resetMe;
include "$dir/b.php";
var_dump($gb, isset($toUnset), $refd, $target, $caught->getMessage(), $resetMe, $keep);
$target = 'T2';
var_dump($refd);

function inner($dir) {
    $toUnset = 1;
    include "$dir/b.php";
    var_dump($gb, isset($toUnset), $refd, $caught->getMessage(), $resetMe);
    $gb = 'changed in function';
}
inner($dir);
var_dump($gb);
unlink("$dir/b.php");
rmdir($dir);
