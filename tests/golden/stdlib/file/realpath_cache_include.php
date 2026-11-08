<?php
// include resolves its path through php's realpath cache: every prefix it
// walked is listed by realpath_cache_get(), under the name as written
// ('./', '..'), and a symlink swapped under it keeps resolving to the old
// target until clearstatcache(true).
$base = sys_get_temp_dir() . '/rhp_rpc_include_' . getmypid();
@mkdir($base);
$base = realpath($base);
@mkdir("$base/a");
@mkdir("$base/b");
file_put_contents("$base/a/inc.php", "<?php return 'a';");
file_put_contents("$base/b/inc.php", "<?php return 'b';");
symlink("$base/a", "$base/cur");
chdir($base);
clearstatcache(true);
$show = function () use ($base) {
    $c = realpath_cache_get();
    $k = array_filter(array_keys($c), fn($p) => str_starts_with($p, "$base/"));
    sort($k);
    foreach ($k as $p) {
        echo str_replace($base, 'BASE', $p), ' => ', str_replace($base, 'BASE', $c[$p]['realpath']),
            $c[$p]['is_dir'] ? ' (dir)' : '', "\n";
    }
};
var_dump(include "$base/a/../a/inc.php");
var_dump(include 'a/inc.php');
var_dump(include './b/inc.php');
var_dump(include "$base/cur/inc.php");
$show();
// swapped behind php's back (php's own unlink would clear the cache)
exec('ln -sfn ' . escapeshellarg("$base/b") . ' ' . escapeshellarg("$base/cur"));
var_dump(include "$base/cur/inc.php");
clearstatcache(true);
var_dump(include "$base/cur/inc.php");
var_dump(array_map(fn($f) => str_replace($base, 'BASE', $f), array_slice(get_included_files(), 1)));
unlink("$base/cur");
unlink("$base/a/inc.php");
unlink("$base/b/inc.php");
rmdir("$base/a");
rmdir("$base/b");
chdir('/');
rmdir($base);
