<?php
// stat/lstat (26-element arrays), fileinode/owner/group/[amc]time/perms,
// symlinks (is_link/readlink/lstat/filetype/realpath), touch with times,
// chmod/umask/mkdir modes, copy semantics.
function rm_tree($p) {
    if (is_link($p) || is_file($p)) { unlink($p); return; }
    if (!is_dir($p)) return;
    foreach (scandir($p) as $e) {
        if ($e !== '.' && $e !== '..') rm_tree("$p/$e");
    }
    rmdir($p);
}

function main() {
    $old = umask(022);
    $base = sys_get_temp_dir() . "/rhp_stat_" . bin2hex(random_bytes(6));
    mkdir($base);
    $f = "$base/file.txt";
    file_put_contents($f, "hello world");

    $s = stat($f);
    var_dump(count($s));
    echo implode(",", array_keys($s)), "\n";
    for ($i = 0; $i <= 12; $i++) {
        $names = ["dev", "ino", "mode", "nlink", "uid", "gid", "rdev", "size", "atime", "mtime", "ctime", "blksize", "blocks"];
        if ($s[$i] !== $s[$names[$i]]) echo "mismatch $i\n";
    }
    var_dump($s['size'], $s['nlink'], decoct($s['mode']));
    var_dump(fileinode($f) === $s['ino'], fileowner($f) === $s['uid'], filegroup($f) === $s['gid']);
    var_dump(filemtime($f) === $s['mtime'], fileatime($f) === $s['atime'], filectime($f) === $s['ctime']);
    var_dump(filesize($f), decoct(fileperms($f)), filetype($f), filetype($base));
    var_dump(decoct(fileperms($base)), is_dir($base), is_file($base), is_file($f));

    // touch with explicit times
    var_dump(touch($f, 1000000000, 1000000500));
    clearstatcache();
    var_dump(filemtime($f), fileatime($f));
    var_dump(touch($f, 1234567890));
    clearstatcache();
    var_dump(filemtime($f), fileatime($f));
    var_dump(touch("$base/new.txt"), file_exists("$base/new.txt"), filesize("$base/new.txt"));
    var_dump(decoct(fileperms("$base/new.txt")));
    var_dump(@touch("$base/nodir/x"));

    // chmod / fileperms / is_* access
    var_dump(chmod($f, 0600));
    clearstatcache();
    var_dump(decoct(fileperms($f)), is_readable($f), is_writable($f), is_executable($f));
    chmod($f, 0755);
    clearstatcache();
    var_dump(decoct(fileperms($f) & 0777), is_executable($f));
    chmod($f, 0644);
    var_dump(@chmod("$base/missing", 0644));

    // mkdir modes honour the permission argument and umask
    var_dump(mkdir("$base/m700", 0700), decoct(fileperms("$base/m700") & 0777));
    var_dump(mkdir("$base/m777"), decoct(fileperms("$base/m777") & 0777));
    var_dump(mkdir("$base/r/a/b", 0750, true), decoct(fileperms("$base/r/a") & 0777), is_dir("$base/r/a/b"));
    var_dump(@mkdir("$base/r/a/b", 0777, true));
    var_dump(@mkdir("$base/r/a/b"));
    var_dump(mkdir("$base/r//x//", 0777, true), is_dir("$base/r/x"));
    var_dump(@mkdir("$base/file.txt/sub", 0777, true));
    umask(0);
    var_dump(mkdir("$base/m0", 0777), decoct(fileperms("$base/m0") & 0777));
    umask(022);
    var_dump(decoct(umask()));

    // symlinks
    var_dump(symlink($f, "$base/link"), is_link("$base/link"), is_link($f));
    var_dump(readlink("$base/link") === $f, @readlink($f));
    var_dump(filetype("$base/link"), is_file("$base/link"));
    $l = lstat("$base/link");
    var_dump(count($l), $l['ino'] !== $s['ino'], decoct($l['mode'] & 0170000));
    var_dump(stat("$base/link")['ino'] === $s['ino']);
    var_dump(realpath("$base/link") === realpath($f));
    var_dump(symlink("$base/none", "$base/dangle"), is_link("$base/dangle"), file_exists("$base/dangle"));
    var_dump(@stat("$base/dangle"), is_array(lstat("$base/dangle")), realpath("$base/dangle"));
    var_dump(@symlink($f, "$base/link"));
    var_dump(realpath("$base/r/a/../a/./b") === realpath($base) . "/r/a/b");
    var_dump(realpath("$base/missing"));

    // failures return false
    var_dump(@stat("$base/missing"), @lstat("$base/missing"), @fileinode("$base/missing"),
        @fileowner("$base/missing"), @filegroup("$base/missing"), @fileatime("$base/missing"),
        @filectime("$base/missing"), @fileperms("$base/missing"), @filetype("$base/missing"));
    var_dump(stat(""), is_link(""), fileperms(""));

    // copy(): content, permissions of a new target, refuses dirs / self
    chmod($f, 0700);
    var_dump(copy($f, "$base/copy.txt"), file_get_contents("$base/copy.txt"));
    var_dump(decoct(fileperms("$base/copy.txt") & 0777));
    var_dump(@copy($base, "$base/dircopy"), @copy($f, "$base/m700"), @copy($f, $f));
    var_dump(@copy("$base/missing", "$base/x"), file_exists("$base/x"));
    file_put_contents("$base/copy.txt", "longer content here");
    var_dump(copy($f, "$base/copy.txt"), file_get_contents("$base/copy.txt"));

    rm_tree($base);
    umask($old);
}

try { main(); } catch (Throwable $e) { echo "Uncaught: ", $e->getMessage(), "\n"; }
