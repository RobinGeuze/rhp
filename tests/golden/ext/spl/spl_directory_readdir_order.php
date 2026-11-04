<?php
// Directory iterators walk entries in readdir order (not sorted), like
// opendir/readdir; DirectoryIterator yields itself and includes the dots.
$probe = new EmptyIterator();
$d = sys_get_temp_dir() . '/rhp_rdorder_' . getmypid() . '_' . mt_rand();
mkdir($d);
foreach (['zeta', 'alpha', 'mid', 'Beta', '0num', 'x.txt'] as $n) { touch("$d/$n"); }
mkdir("$d/sub");
touch("$d/sub/inner");
$h = opendir($d);
$raw = [];
while (($e = readdir($h)) !== false) { $raw[] = $e; }
closedir($h);
$noDots = array_values(array_filter($raw, fn($e) => $e !== '.' && $e !== '..'));

$fs = [];
foreach (new FilesystemIterator($d) as $k => $f) { $fs[] = $f->getFilename(); }
var_dump($fs === $noDots, $fs === scandir($d, SCANDIR_SORT_NONE) || $fs === $noDots);

$di = [];
foreach (new DirectoryIterator($d) as $k => $f) { $di[$k] = $f->getFilename(); }
var_dump($di === $raw);

$rdi = [];
foreach (new RecursiveDirectoryIterator($d) as $f) { $rdi[] = $f->getFilename(); }
var_dump($rdi === $raw);

$rii = [];
foreach (new RecursiveIteratorIterator(new RecursiveDirectoryIterator($d, FilesystemIterator::SKIP_DOTS), RecursiveIteratorIterator::SELF_FIRST) as $k => $f) {
    $rii[] = substr($k, strlen($d));
}
sort($rii);
var_dump($rii);

$it = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($d, FilesystemIterator::SKIP_DOTS));
$sub = [];
foreach ($it as $f) { $sub[] = $it->getSubPathname() . '@' . $it->getDepth(); }
sort($sub);
var_dump($sub);

$di = new DirectoryIterator($d);
foreach ($di as $e) {
    if ($e->isDot()) { continue; }
    var_dump($e === $di);
    break;
}
$di->seek(2);
var_dump($di->key(), in_array($di->getFilename(), $raw, true));
try { $di->seek(99); } catch (OutOfBoundsException $e) { echo $e->getMessage(), "\n"; }

unlink("$d/sub/inner");
rmdir("$d/sub");
foreach (['zeta', 'alpha', 'mid', 'Beta', '0num', 'x.txt'] as $n) { unlink("$d/$n"); }
rmdir($d);
