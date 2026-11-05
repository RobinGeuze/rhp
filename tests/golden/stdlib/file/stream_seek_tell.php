<?php
// fseek/ftell/feof/fread/fwrite on plain files in every fopen mode, memory and
// temp streams (php's position / EOF bookkeeping), closed-stream TypeErrors.
$d = sys_get_temp_dir() . "/rhp_fseek_" . getmypid();
@mkdir($d);
$fn = "$d/a.txt";
function show($label, ...$v) { echo $label, ": "; foreach ($v as $x) { echo var_export($x, true), " "; } echo "\n"; }
// w+
$w = fopen($fn, 'w+');
show('w+', ftell($w), feof($w), fwrite($w, 'hello'), ftell($w), fseek($w, 1), ftell($w), fwrite($w, 'E'), ftell($w));
show('w+r', rewind($w), fread($w, 10), feof($w), ftell($w), fread($w, 1), feof($w));
show('seek', fseek($w, -2, SEEK_END), fread($w, 10), fseek($w, -1, SEEK_CUR), fgetc($w), fseek($w, -100, SEEK_CUR), ftell($w), fseek($w, -1), ftell($w));
show('past', fseek($w, 8), ftell($w), feof($w), fwrite($w, 'Z'), ftell($w));
fclose($w);
show('file', bin2hex(file_get_contents($fn)));
// r
file_put_contents($fn, "line1\nline2\nlast");
$r = fopen($fn, 'r');
show('r', fgets($r), ftell($r), feof($r), fgets($r), fgets($r), feof($r), fgets($r), ftell($r));
show('r2', rewind($r), fread($r, 3), fseek($r, 100), ftell($r), feof($r), fread($r, 5), feof($r));
show('r3', fseek($r, 2), fgets($r, 3), ftell($r), stream_get_contents($r), feof($r), ftell($r));
show('r4', stream_get_contents($r, -1, 6), stream_get_contents($r, 3, 0), ftell($r));
fclose($r);
// r+ / c+ / a / a+ / x
file_put_contents($fn, '0123456789');
$f = fopen($fn, 'r+'); show('r+', fread($f, 2), fwrite($f, 'AB'), ftell($f), fread($f, 2)); fclose($f); show('r+f', file_get_contents($fn));
$f = fopen($fn, 'c+'); show('c+', fwrite($f, 'cc'), ftell($f), fread($f, 3)); fclose($f); show('c+f', file_get_contents($fn));
$f = fopen($fn, 'c'); show('c', fwrite($f, 'C'), ftell($f)); fclose($f); show('cf', file_get_contents($fn));
$f = fopen($fn, 'a'); show('a', ftell($f), fwrite($f, 'aa'), ftell($f), fseek($f, 0), fwrite($f, 'b'), ftell($f)); fclose($f); show('af', file_get_contents($fn));
$f = fopen($fn, 'a+'); show('a+', ftell($f), fread($f, 4), fwrite($f, 'Q'), ftell($f), fread($f, 100)); fclose($f); show('a+f', file_get_contents($fn));
show('x', @fopen($fn, 'x'), @fopen($fn, 'q'));
$f = fopen("$d/b.txt", 'x+'); show('x+', fwrite($f, 'new'), rewind($f), fread($f, 9)); fclose($f);
// live write visible before fclose
$f = fopen("$d/c.txt", 'w'); fwrite($f, 'live'); show('live', file_get_contents("$d/c.txt"), ftell($f)); fclose($f);
// fwrite length
$f = fopen("$d/c.txt", 'w'); show('len', fwrite($f, 'abcdef', 3), fwrite($f, 'xyz', 0), fwrite($f, 'xyz', -1), fwrite($f, ''), fwrite($f, 'q', null)); fclose($f); show('lenf', file_get_contents("$d/c.txt"));
// big read
file_put_contents($fn, str_repeat('z', 30000));
$f = fopen($fn, 'r'); show('big', strlen(fread($f, 20000)), strlen(fread($f, 100000)), feof($f)); fclose($f);
// memory / temp
foreach (['php://memory', 'php://temp', 'php://temp/maxmemory:10'] as $u) {
  $m = fopen($u, 'w+');
  show($u, feof($m), fwrite($m, 'abc'), ftell($m), feof($m), fseek($m, 5), ftell($m), fwrite($m, 'Z'), ftell($m), rewind($m), bin2hex(fread($m, 100)), feof($m));
  show('m2', fseek($m, -1), ftell($m), fseek($m, 1), fread($m, 2), feof($m), bin2hex(fread($m, 10)), feof($m), fseek($m, -2, SEEK_END), bin2hex(fgets($m)), feof($m));
  show('m3', fseek($m, -100, SEEK_END), ftell($m), fseek($m, 0, 7), ftell($m));
}
$m = fopen('php://memory', 'r'); show('mr', fwrite($m, 'a'), ftell($m), fread($m, 5), feof($m));
$m = fopen('php://memory', 'a+'); show('ma', fwrite($m, 'abc'), fseek($m, 0), fwrite($m, 'd'), ftell($m), rewind($m), fread($m, 10));
$m = fopen('php://memory', 'w+'); fwrite($m, "l1\nl2\nl3"); rewind($m);
show('mg', fgets($m), feof($m), fgets($m, 2), fgets($m), fgets($m), feof($m), fgets($m));
$m = fopen('php://memory', 'w+'); fwrite($m, "l1\n"); rewind($m); show('mg2', fgets($m), feof($m), fgets($m), feof($m));
// fread errors
$f = fopen($fn, 'r');
try { fread($f, 0); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { fread($f, -1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
fclose($f);
foreach (['fread' => [$f, 1], 'fwrite' => [$f, 'x'], 'feof' => [$f], 'ftell' => [$f], 'fseek' => [$f, 0], 'rewind' => [$f], 'fclose' => [$f], 'fgets' => [$f], 'fgetc' => [$f], 'fflush' => [$f], 'stream_get_contents' => [$f]] as $fnn => $a) {
  try { $fnn(...$a); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
}
try { fwrite(false, 'x'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { fread("str", 1); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { feof(null); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
foreach (['a.txt', 'b.txt', 'c.txt'] as $x) @unlink("$d/$x");
rmdir($d);
