<?php
// umask() is the request's: every way of creating a file applies it, also
// when it lets through bits the process's umask would take away.
$base = realpath(sys_get_temp_dir()) . "/rhp_umask_" . bin2hex(random_bytes(6));
mkdir($base);
chmod($base, 0777);
$old = umask(0);
var_dump(umask(null), umask());
function perms($f) { clearstatcache(); return decoct(fileperms($f) & 0777); }
foreach ([0, 022, 077, 044, 02] as $mask) {
    umask($mask);
    echo "-- ", sprintf("%03o", $mask), " --\n";
    $h = fopen("$base/fopen", "w"); fclose($h);
    file_put_contents("$base/fpc", "x");
    touch("$base/touch");
    copy("$base/fpc", "$base/copy");
    mkdir("$base/dir");
    mkdir("$base/deep/er", 0775, true);
    $t = tempnam($base, "t");
    $g = gzopen("$base/gz", "w"); gzclose($g);
    $z = new ZipArchive(); $z->open("$base/z.zip", ZipArchive::CREATE); $z->addFromString("a", "b"); $z->close();
    (new DOMDocument())->save("$base/dom.xml");
    error_log("line", 3, "$base/log");
    foreach (["fopen", "fpc", "touch", "copy", "dir", "deep", "deep/er", "gz", "z.zip", "dom.xml", "log"] as $f) {
        echo $f, " ", perms("$base/$f"), "\n";
    }
    echo "tempnam ", perms($t), "\n";
    // An existing file keeps its mode.
    chmod("$base/fpc", 0604);
    file_put_contents("$base/fpc", "y");
    echo "existing ", perms("$base/fpc"), "\n";
    chmod("$base/deep/er", 0777); chmod("$base/deep", 0777); chmod("$base/dir", 0777);
    rmdir("$base/deep/er"); rmdir("$base/deep"); rmdir("$base/dir");
    foreach (["fopen", "fpc", "touch", "copy", "gz", "z.zip", "dom.xml", "log"] as $f) unlink("$base/$f");
    unlink($t);
}
var_dump(umask($old), umask() === $old);
rmdir($base);
