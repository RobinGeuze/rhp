<?php
// A stream is freed when its last reference goes away: the descriptor is
// closed (no EMFILE after many unclosed fopen()s), flock locks are released,
// buffered writes land, and other references keep it open.
$base = sys_get_temp_dir() . "/rhp_srel_" . bin2hex(random_bytes(6));
mkdir($base);
$f = "$base/z.txt";
file_put_contents($f, 'x');

function rd($f) { $h = fopen($f, 'r'); return fread($h, 1); }
$ok = 0;
for ($i = 0; $i < 3000; $i++) if (rd($f) === 'x') $ok++;
var_dump($ok);

function lk($f) { $h = fopen($f, 'r'); return flock($h, LOCK_EX | LOCK_NB); }
var_dump(lk($f), lk($f));

function dd($d) { $h = opendir($d); return readdir($h) !== false; }
$ok = 0;
for ($i = 0; $i < 3000; $i++) if (dd($base)) $ok++;
var_dump($ok);

// still referenced elsewhere: stays open
$h = fopen($f, 'r'); $g = $h; unset($h);
var_dump(fread($g, 1));
$a = [fopen($f, 'r')]; $b = $a; unset($a);
var_dump(fread($b[0], 1));
$o = new stdClass; $o->h = fopen($f, 'r'); $p = $o; unset($o);
var_dump(fread($p->h, 1));

// a held lock blocks; overwriting the variable releases it
$h = fopen($f, 'r');
var_dump(flock($h, LOCK_EX), lk($f));
$h = fopen($f, 'r');
var_dump(lk($f));
$h = null;
var_dump(lk($f));

function w($f) { $h = fopen($f, 'w'); fwrite($h, 'hello'); }
w("$base/w.txt");
var_dump(file_get_contents("$base/w.txt"));

unlink($f);
unlink("$base/w.txt");
rmdir($base);
