<?php
// realpath() goes through php's realpath cache: a removed symlink still
// resolves until clearstatcache(true[, $path]) (or an unlink) drops it.
$base = sys_get_temp_dir() . '/rhp_realpath_cache_' . getmypid();
@mkdir($base);
$base = realpath($base);
@mkdir("$base/dir");
@symlink("$base/dir", "$base/l1");
@symlink("$base/l1", "$base/l2");
$show = fn($p) => var_dump(is_string($p) ? str_replace($base, 'BASE', $p) : $p);

$show(realpath("$base/l2"));
$e = realpath_cache_get()["$base/l2"];
var_dump($e['is_dir'], str_replace($base, 'BASE', $e['realpath']), is_int($e['key']) || is_float($e['key']));
var_dump(realpath_cache_size() > 0);
exec('rm ' . escapeshellarg("$base/l1"));
$show(realpath("$base/l2"));
clearstatcache();
$show(realpath("$base/l2"));
clearstatcache(true, "$base/l1");
$show(realpath("$base/l2"));
clearstatcache(true, "$base/l2");
$show(realpath("$base/l2"));
// lexical '..' after a missing directory, through fopen and file()
@mkdir("$base/dir/d");
file_put_contents("$base/dir/f", "x\n");
chdir("$base/dir");
var_dump(file('d/missing/../../f'), is_resource(fopen('./d/no/../../f', 'r')));
var_dump(@file_get_contents('f/'));
// unlink clears the whole cache
realpath("$base/dir");
unlink("$base/dir/f");
var_dump(isset(realpath_cache_get()["$base/dir"]));
rmdir("$base/dir/d");
chdir($base);
rmdir("$base/dir");
unlink("$base/l2");
rmdir($base);
