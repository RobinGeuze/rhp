<?php
// php's recursion protection: an array or object already being encoded
// is skipped.
$a = ['x' => 1];
$a['self'] = &$a;
echo http_build_query($a), "\n";
$o = new stdClass;
$o->a = 1;
$o->me = $o;
$o->list = [2, 3];
echo http_build_query($o), "\n";
$deep = 'v';
for ($i = 0; $i < 300; $i++) {
    $deep = [$deep];
}
echo strlen(http_build_query(['d' => $deep])), "\n";
