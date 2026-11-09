<?php
$d = __DIR__;
require "$d/a/helper.php";
require "$d/b/helper.php";
var_dump(inc_from_a("x.php"), inc_from_b("x.php"));
chdir("$d/b");
var_dump(include "./x.php");
chdir($d);
var_dump(include "a/../b/x.php", include "$d/./a/x.php");
var_dump(include_once "$d/a/../a/x.php", require_once "a/x.php");
$s = opcache_get_status(false)['opcache_statistics'];
echo "scripts {$s['num_cached_scripts']} keys {$s['num_cached_keys']} hits {$s['hits']} misses {$s['misses']}\n";
