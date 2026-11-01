<?php
// Filesystem functions over phar:// paths (opened by filesystem path, no
// Phar::loadPhar): stat family, dir streams, file(), copy(), fopen/fstat.
const BLOB = "PD9waHAgX19IQUxUX0NPTVBJTEVSKCk7ID8+DQqsAAAABAAAABEAAAABAAgAAABmc3gucGhhcgAAAAAFAAAAYS50eHQLAAAAO1zmagsAAABuUDBupAEAAAAAAAAJAAAAZGlyL2IucGhwEgAAADtc5moSAAAAJbAoYaQBAAAAAAAADQAAAGRpci9zdWIvYy50eHQFAAAAO1zmagUAAABx0EPEpAEAAAAAAAAHAAAAenouanNvbgIAAAA7XOZqAgAAAEO/pqOkAQAAAAAAAGFscGhhCmJldGEKPD9waHAgcmV0dXJuICdiJzsKZ2FtbWF7fbKgt5Yk0sF0LS2HwrfaNgQcNLmhEHf085eInMW6hRqwAwAAAEdCTUI=";

function main() {
    $dir = sys_get_temp_dir() . "/rhp_pharfs_" . bin2hex(random_bytes(6));
    mkdir($dir);
    $file = "$dir/fsx.phar";
    file_put_contents($file, base64_decode(BLOB));
    $p = "phar://$file";

    var_dump(file_exists("$p/a.txt"), is_file("$p/a.txt"), is_dir("$p/a.txt"));
    var_dump(is_dir("$p/dir"), is_dir("$p/dir/sub"), is_dir("$p/"), is_dir($p), file_exists($p), file_exists("$p/"));
    var_dump(file_exists("$p/nope"), is_file("$p/dir"), is_readable("$p/a.txt"), is_writable("$p/a.txt"), is_executable("$p/a.txt"));
    var_dump(filesize("$p/a.txt"), filemtime("$p/a.txt"), decoct(fileperms("$p/a.txt")), decoct(fileperms("$p/dir")));
    var_dump(filetype("$p/a.txt"), filetype("$p/dir"), is_link("$p/a.txt"));
    $s = stat("$p/dir/sub/c.txt");
    var_dump(count($s), $s['size'], $s['dev'], $s['nlink'], $s['rdev'], $s['blksize'], $s['blocks'], $s['uid'], $s['mtime'] === $s['ctime']);
    var_dump($s['ino'] === fileinode("$p/dir/sub/c.txt"), $s['ino'] > 0, fileinode("$p/a.txt") !== $s['ino']);
    $d = stat("$p/dir");
    var_dump($d['mode'], $d['size'], $d['ino'], $d['mtime'] === filemtime("$p/a.txt"));
    var_dump(lstat("$p/a.txt")['mode'], @stat("$p/missing"), @stat($p));

    // directory streams
    var_dump(scandir("$p/"), scandir("$p/dir"), scandir("$p/dir", SCANDIR_SORT_DESCENDING));
    $h = opendir("$p/dir");
    $seen = [];
    while (($e = readdir($h)) !== false) $seen[] = $e;
    rewinddir($h);
    var_dump($seen, readdir($h));
    closedir($h);
    var_dump(@opendir("$p/nope"), @opendir("$p/a.txt"), is_resource(@opendir($p)), @scandir("$p/zz.json"));

    // contents
    var_dump(file("$p/a.txt"), file("$p/a.txt", FILE_IGNORE_NEW_LINES), file_get_contents("$p/dir/sub/c.txt"));
    var_dump(file_get_contents("$p/dir/b.php"));
    var_dump(md5_file("$p/zz.json") === md5("{}"));
    var_dump(copy("$p/a.txt", "$dir/copied.txt"), file_get_contents("$dir/copied.txt"));
    var_dump(decoct(fileperms("$dir/copied.txt") & 0777) === decoct(0666 & ~umask()));

    // streams on phar entries
    $h = fopen("$p/a.txt", "r");
    var_dump(fgets($h), ftell($h), fread($h, 100), feof($h));
    var_dump(fseek($h, 2), fread($h, 3), rewind($h), fgets($h, 3));
    $st = fstat($h);
    var_dump($st['size'], decoct($st['mode']), $st['ino'] === fileinode("$p/a.txt"));
    $m = stream_get_meta_data($h);
    var_dump($m['wrapper_type'], $m['stream_type'], $m['mode'], $m['uri'] === "$p/a.txt", $m['seekable']);
    var_dump(stream_is_local($h), stream_is_local("$p/a.txt"), flock($h, LOCK_SH));
    fclose($h);
    var_dump(@fopen("$p/missing", "r"));

    unlink("$dir/copied.txt");
    unlink($file);
    rmdir($dir);
}

try { main(); } catch (Throwable $e) { echo "Uncaught: ", get_class($e), ": ", $e->getMessage(), "\n"; }
