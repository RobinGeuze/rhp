<?php
$st = function ($what) {
    $s = opcache_get_status(false)['opcache_statistics'];
    echo "$what: scripts {$s['num_cached_scripts']} keys {$s['num_cached_keys']} hits {$s['hits']} misses {$s['misses']}\n";
};
$d = __DIR__;
var_dump(include "$d/lib/../lib/a.php");
var_dump(include "$d/lib/../lib/a.php");
var_dump(include "$d/lib/a.php");
var_dump(include 'lib/a.php');
var_dump(include 'lib/a.php');
var_dump(include_once "$d/./lib/a.php");
$st('same file');
// a symlinked directory swapped to another target
$t = sys_get_temp_dir() . '/rhp_oc_keys_' . getmypid();
@mkdir("$t/a", 0777, true);
@mkdir("$t/b");
$t = realpath($t);
file_put_contents("$t/a/x.php", '<?php return "a";');
file_put_contents("$t/b/x.php", '<?php return "b";');
touch("$t/a/x.php", time() - 100);
touch("$t/b/x.php", time() - 100);
symlink("$t/a", "$t/cur");
var_dump(include "$t/cur/x.php");
exec('ln -sfn ' . escapeshellarg("$t/b") . ' ' . escapeshellarg("$t/cur"));
var_dump(include "$t/cur/x.php");
clearstatcache(true);
var_dump(include "$t/cur/x.php");
// a changed file
file_put_contents("$t/b/x.php", '<?php return "b2";');
touch("$t/b/x.php", time() - 50);
var_dump(include "$t/cur/x.php");
$st('swapped');
unlink("$t/cur");
unlink("$t/a/x.php");
unlink("$t/b/x.php");
rmdir("$t/a");
rmdir("$t/b");
rmdir($t);
