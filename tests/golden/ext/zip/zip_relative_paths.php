<?php
// ZipArchive, the zip:// wrapper, gzopen and bzopen take relative paths
// from the working directory chdir() set.
set_error_handler(function ($l, $m) { echo "[$l] $m\n"; return true; });
$base = realpath(sys_get_temp_dir()) . "/rhp_vcwdz_" . bin2hex(random_bytes(6));
mkdir("$base/sub", 0777, true);
$start = getcwd();
chdir($base);
file_put_contents("sub/a.txt", "alpha");

$z = new ZipArchive();
var_dump($z->open("sub/t.zip", ZipArchive::CREATE), $z->addFile("sub/a.txt", "a.txt"), $z->addFromString("d/b.txt", "beta"), $z->close());
var_dump(file_exists("$base/sub/t.zip"));
var_dump($z->open("sub/t.zip"), $z->numFiles, $z->extractTo("out"), $z->close());
var_dump(file_get_contents("$base/out/a.txt"), file_get_contents("$base/out/d/b.txt"));
var_dump(file_get_contents("zip://sub/t.zip#d/b.txt"));
$zr = zip_open("sub/t.zip");
var_dump(is_resource($zr));
zip_close($zr);

$g = gzopen("sub/g.gz", "w");
gzwrite($g, "gzipped");
gzclose($g);
var_dump(file_exists("$base/sub/g.gz"), implode("", gzfile("sub/g.gz")), file_get_contents("compress.zlib://sub/g.gz"));
var_dump(file_put_contents("compress.zlib://sub/h.gz", "zz"), readgzfile("sub/h.gz"));
echo "\n";

$b = bzopen("sub/b.bz2", "w");
bzwrite($b, "bzipped");
bzclose($b);
$b = bzopen("sub/b.bz2", "r");
var_dump(bzread($b), file_get_contents("compress.bzip2://sub/b.bz2"));
bzclose($b);

foreach (["sub/a.txt", "sub/t.zip", "out/a.txt", "out/d/b.txt", "sub/g.gz", "sub/h.gz", "sub/b.bz2"] as $f) unlink($f);
rmdir("out/d");
rmdir("out");
chdir($start);
var_dump(rmdir("$base/sub"), rmdir($base));
