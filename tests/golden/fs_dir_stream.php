<?php
// opendir/readdir/rewinddir/closedir, dir() + Directory, scandir.
// (Wrapped in a function + try so the php-free oracle skips this fixture.)
function rm_tree($p) {
    if (is_link($p) || is_file($p)) { unlink($p); return; }
    if (!is_dir($p)) return;
    foreach (scandir($p) as $e) {
        if ($e !== '.' && $e !== '..') rm_tree("$p/$e");
    }
    rmdir($p);
}

function err($f) {
    try { $f(); echo "no error\n"; }
    catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}

function main() {
    $base = sys_get_temp_dir() . "/rhp_fsdir_" . bin2hex(random_bytes(6));
    mkdir($base);
    foreach (["b.txt", "a.txt", "0", ".dot", "c"] as $f) touch("$base/$f");
    mkdir("$base/sub");

    $h = opendir($base);
    var_dump(is_resource($h), get_resource_type($h));
    $names = [];
    while (($n = readdir($h)) !== false) $names[] = $n;
    sort($names);
    echo implode(",", $names), "\n";
    var_dump(readdir($h));          // exhausted
    rewinddir($h);
    $again = [];
    while (($n = readdir($h)) !== false) $again[] = $n;
    sort($again);
    var_dump($again === $names);
    var_dump(closedir($h));
    err(fn() => readdir($h));
    err(fn() => closedir($h));

    // An entry named "0" must not end a `!== false` loop early.
    $h = opendir($base);
    $count = 0;
    while (($n = readdir($h)) !== false) $count++;
    var_dump($count);
    closedir($h);

    // errors
    err(fn() => readdir("nope"));
    err(fn() => rewinddir(42));
    $fp = fopen("$base/a.txt", "r");
    err(fn() => readdir($fp));
    err(fn() => closedir($fp));
    fclose($fp);
    var_dump(@opendir("$base/missing"));
    var_dump(@opendir("$base/a.txt"));
    err(fn() => opendir("a\0b"));

    // dir() / Directory
    $d = dir($base);
    var_dump($d instanceof Directory, get_class($d), $d->path === $base);
    var_dump(is_resource($d->handle));
    $seen = [];
    while (false !== ($e = $d->read())) $seen[] = $e;
    sort($seen);
    var_dump($seen === $names);
    $d->rewind();
    var_dump(is_string($d->read()));
    $d->close();
    err(fn() => $d->read());
    err(fn() => $d->rewind());
    err(fn() => new Directory());
    var_dump(@dir("$base/missing"));
    $rc = new ReflectionClass("Directory");
    var_dump($rc->isFinal());

    // scandir sorting
    echo implode(",", scandir($base)), "\n";
    echo implode(",", scandir($base, SCANDIR_SORT_DESCENDING)), "\n";
    echo implode(",", scandir($base, 7)), "\n";
    $none = scandir($base, SCANDIR_SORT_NONE);
    sort($none);
    echo implode(",", $none), "\n";
    var_dump(@scandir("$base/missing"));
    err(fn() => scandir(""));
    var_dump(SCANDIR_SORT_ASCENDING, SCANDIR_SORT_DESCENDING, SCANDIR_SORT_NONE);

    rm_tree($base);
    var_dump(file_exists($base));
}

try { main(); } catch (Throwable $e) { echo "Uncaught: ", $e->getMessage(), "\n"; }
