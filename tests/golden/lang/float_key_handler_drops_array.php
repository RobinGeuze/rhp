<?php
// The float-key warning's handler drops the last reference to the array:
// php gives up the lookup (isset false, empty true, array_key_exists false).
set_error_handler(function ($errno, $errstr) {
    global $ary, $mode, $x;
    echo $errstr, "\n";
    if ($mode == 1) $ary = null;
    if ($mode == 2) $x = $ary;
    if ($mode == 3) $ary[] = 9;
    if ($mode == 4) $ary = [7 => 1];
});
foreach ([0, 1, 2, 3, 4] as $mode) {
    echo "mode $mode\n";
    $ary = [5]; $keep = $ary;
    var_dump(isset($ary[1.0E+42]));
    $ary = [5];
    var_dump(isset($ary[1.0E+42]));
    $ary = [5];
    var_dump(empty($ary[1.0E+42]));
    $ary = [5];
    var_dump(array_key_exists(1.0E+42, $ary));
    $ary = [5]; $keep = $ary;
    var_dump(array_key_exists(1.0E+42, $ary));
}
