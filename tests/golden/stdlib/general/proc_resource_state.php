<?php
// Closed resources, stream_select on plain files, pipe read chunking.
function t($f) {
    try {
        var_dump($f());
    } catch (\Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
function ids($s) { return preg_replace('/(resource\(|Resource id #)\d+/', '$1N', $s); }

// proc_close / fclose leave the handles closed
$p = proc_open('true', [1 => ['pipe', 'w']], $pipes);
var_dump(is_resource($p), gettype($p), get_resource_type($p), get_debug_type($p), get_debug_type($pipes[1]));
var_dump(proc_close($p));
var_dump(is_resource($p), gettype($p), get_resource_type($p), get_debug_type($p));
var_dump(is_resource($pipes[1]), get_resource_type($pipes[1]));
ob_start(); var_dump($p, $pipes); echo ids(ob_get_clean());
t(fn() => proc_close($p));
t(fn() => proc_get_status($p));
t(fn() => proc_terminate($p));
$f = fopen('php://memory', 'r');
$g = $f;
fclose($f);
var_dump(is_resource($g), gettype($g), get_resource_type($g), (bool) $g, ids((string) $g), ids(print_r($g, true)));
$c = stream_context_create();
var_dump(get_resource_type($c), is_resource($c));

// stream_select: plain files are always ready; memory streams cannot be selected
$w = null;
$e = null;
$file = fopen('/etc/hosts', 'r');
$r = [$file];
var_dump(stream_select($r, $w, $e, 0), count($r));
$m = fopen('php://memory', 'r');
$r = [$m];
try {
    @stream_select($r, $w, $e, 0);
} catch (ValueError $ex) {
    echo $ex->getMessage(), "\n";
}
$r = ['m' => $m, 'f' => $file];
var_dump(@stream_select($r, $w, $e, 0), array_keys($r));
$r = [$file];
$w = [$file];
var_dump(stream_select($r, $w, $e, null), count($r), count($w));
$p = proc_open('sleep 1', [1 => ['pipe', 'w']], $pipes);
$r = ['pipe' => $pipes[1], 'file' => $file];
$w = null;
var_dump(stream_select($r, $w, $e, 5), array_keys($r));
proc_terminate($p);
proc_close($p);
var_dump(stream_isatty($file), posix_isatty($file), posix_get_last_error() === 25);

// a pipe fread returns at most one 8192-byte chunk per call
$p = proc_open('head -c 30000 /dev/zero', [1 => ['pipe', 'w']], $pipes);
$got = '';
$r = [$pipes[1]];
$w = null;
stream_select($r, $w, $e, 5);
usleep(200000);
var_dump(strlen(fread($pipes[1], 20000)) <= 8192);
var_dump(strlen(fread($pipes[1], 5)));
$rest = stream_get_contents($pipes[1]);
var_dump(strlen($rest) > 0);
proc_close($p);

// loose comparison of resources and closures
$h1 = fopen('php://memory', 'r');
$h2 = fopen('php://memory', 'r');
var_dump($h1 == $h2, $h1 <=> $h2, $h2 <=> $h1, $h1 == $h1, in_array($h1, [$h2]), in_array($h1, [$h2, $h1], false));
var_dump($h1 == (int) $h1, $h1 == "x", $h1 < 100000, $h1 == true, $h1 == null, null == $h1, $h1 <=> null, $h1 == [], $h1 <=> [1]);
$c1 = fn() => 1;
$c2 = $c1;
$c3 = fn() => 1;
var_dump($c1 == $c2, $c1 == $c3, $c1 <=> $c3, $c3 <=> $c1, $c1 == true, $c1 == null, null <=> $c1);
$s1 = strlen(...);
$s2 = strlen(...);
var_dump($s1 == $s2, $s1 === $s2, $s1 <=> $s2, Closure::fromCallable('strlen') == $s1, strtoupper(...) == $s1);
switch ($c1) {
    case $c3: echo "c3\n"; break;
    case $c2: echo "c2\n"; break;
}
var_dump(array_search($c2, [$c3, $c1], false), in_array($c1, [$c3]));
