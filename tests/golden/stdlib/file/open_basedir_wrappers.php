<?php
// open_basedir also binds the wrappers and extensions that open files
// themselves: compress.zlib://, gz*, bzopen, zip, libxml, phar, and an
// include of a missing file outside the base dir.
$base = sys_get_temp_dir() . '/rhp_open_basedir_w_' . getmypid();
@mkdir($base);
$base = realpath($base);
@mkdir("$base/in");
@mkdir("$base/out");
file_put_contents("$base/out/t.gz", gzencode("gz data\n"));
file_put_contents("$base/out/t.bz2", bzcompress("bz data\n"));
file_put_contents("$base/out/t.xml", "<a>secret</a>");
$z = new ZipArchive();
$z->open("$base/out/t.zip", ZipArchive::CREATE);
$z->addFromString('a.txt', 'zip data');
$z->close();
chdir("$base/in");
set_include_path(".");

set_error_handler(function ($no, $msg) use ($base) {
    echo str_replace($base, 'BASE', $msg), "\n";
    return true;
});
function show($label, $v) {
    global $base;
    $v = is_object($v) || is_resource($v) ? get_debug_type($v) : $v;
    echo $label, ': ', str_replace($base, 'BASE', var_export($v, true)), "\n";
}

ini_set('open_basedir', "$base/in");
show('compress.zlib', file_get_contents("compress.zlib://$base/out/t.gz"));
show('gzfile', gzfile("$base/out/t.gz"));
show('gzopen', gzopen("$base/out/t.gz", 'r'));
show('readgzfile', readgzfile("$base/out/t.gz"));
show('gz write', file_put_contents("compress.zlib://$base/out/w.gz", 'x'));
show('compress.bzip2', file_get_contents("compress.bzip2://$base/out/t.bz2"));
show('bzopen', bzopen("$base/out/t.bz2", 'r'));
show('ZipArchive::open', (new ZipArchive())->open("$base/out/t.zip"));
show('zip://', file_get_contents("zip://$base/out/t.zip#a.txt"));
show('simplexml', simplexml_load_file("$base/out/t.xml"));
show('dom', (new DOMDocument())->load("$base/out/t.xml"));
show('include missing', include '../out/missing.php');
show('include missing inside', include 'missing.php');
show('getRealPath', (new SplFileInfo('..'))->getRealPath() === $base);
show('w.gz written', file_exists("$base/out/w.gz"));

restore_error_handler();
exec('rm -rf ' . escapeshellarg($base));
echo "done\n";
