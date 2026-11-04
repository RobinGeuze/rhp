<?php
// gzopen / compress.zlib:// use two resource ids (inner plain file + zlib
// stream); a compress.zlib:// write creates the file at open; gz* argument
// errors name the gz function; get_resource_type() rejects non-resources.
$d = sys_get_temp_dir() . '/rhp_zlib_' . getmypid();
@mkdir($d);
$a = fopen('php://memory', 'r+'); var_dump((int) $a);
$e = gzopen("$d/z.gz", 'w'); var_dump((int) $e); gzwrite($e, "hello"); gzclose($e);
$g = fopen("compress.zlib://$d/z.gz", 'r'); var_dump((int) $g, fread($g, 10));
$f = fopen("compress.zlib://$d/y.gz", 'w'); fwrite($f, 'd'); var_dump((int) $f, @file_get_contents("compress.zlib://$d/y.gz"));
fclose($f);
var_dump(file_get_contents("compress.zlib://$d/y.gz"));
$x = fopen('php://memory', 'r'); var_dump((int) $x);
try { gzread($e, 1); } catch (TypeError $t) { echo $t->getMessage(), "\n"; }
try { get_resource_type(false); } catch (TypeError $t) { echo $t->getMessage(), "\n"; }
unlink("$d/z.gz"); unlink("$d/y.gz"); rmdir($d);
