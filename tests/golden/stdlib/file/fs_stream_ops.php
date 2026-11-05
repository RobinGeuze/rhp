<?php
// Plain-file streams: fopen modes, fread/fgets/feof, fseek/ftell, ftruncate,
// fstat, flock (+ $would_block), stream_get_meta_data, php://memory,
// tempnam, stream_is_local, stream_resolve_include_path.
function err($f) {
    try { $r = $f(); echo "ok: ", var_export($r, true), "\n"; }
    catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}

function main() {
    $base = sys_get_temp_dir() . "/rhp_sops_" . bin2hex(random_bytes(6));
    mkdir($base);
    $f = "$base/data.txt";
    file_put_contents($f, "line1\nline2\nlast");

    // read side: eof is set only once a read hits the end
    $h = fopen($f, "r");
    var_dump(ftell($h), feof($h));
    var_dump(fgets($h), feof($h), ftell($h));
    var_dump(fread($h, 6), feof($h));
    var_dump(fread($h, 4), feof($h));       // exactly the rest: not yet eof
    var_dump(fread($h, 1), feof($h));       // now eof
    var_dump(fgets($h), fread($h, 10));
    var_dump(fseek($h, 0), feof($h), ftell($h));
    var_dump(fgets($h, 3), fgets($h, 10), fgets($h));
    var_dump(fseek($h, -4, SEEK_END), fread($h, 100), feof($h));
    var_dump(fseek($h, 2, SEEK_SET), fseek($h, 3, SEEK_CUR), ftell($h), fgetc($h));
    var_dump(fseek($h, -100, SEEK_CUR), ftell($h));
    var_dump(fseek($h, 100), ftell($h), fread($h, 5), feof($h));
    var_dump(rewind($h), ftell($h), stream_get_contents($h), feof($h));
    var_dump(@fwrite($h, "x"));              // read-only stream
    var_dump(ftruncate($h, 0));              // not writable
    $st = fstat($h);
    var_dump(count($st), $st['size'], $st['ino'] === fileinode($f));
    $m = stream_get_meta_data($h);
    var_dump($m['wrapper_type'], $m['stream_type'], $m['mode'], $m['seekable'], $m['uri'] === $f, $m['eof']);
    err(fn() => fgets($h, 0));
    fclose($h);
    err(fn() => ftell($h));
    err(fn() => fseek($h, 0));
    err(fn() => fstat($h));
    err(fn() => ftell("nope"));

    // write modes
    $h = fopen("$base/w.txt", "w");
    var_dump(fwrite($h, "abcdef"), ftell($h), @fread($h, 3));
    var_dump(ftruncate($h, 3), ftell($h), fstat($h)['size']);
    var_dump(fwrite($h, "Z"), fstat($h)['size']);
    fclose($h);
    var_dump(file_get_contents("$base/w.txt"));

    $h = fopen("$base/w.txt", "a+");
    var_dump(ftell($h), fwrite($h, "++"), ftell($h));
    var_dump(rewind($h), fread($h, 100));
    fclose($h);
    var_dump(file_get_contents("$base/w.txt"));

    $h = fopen("$base/w.txt", "r+");
    var_dump(fread($h, 2), fwrite($h, "__"), ftell($h));
    fclose($h);
    var_dump(file_get_contents("$base/w.txt"));

    $h = fopen("$base/w.txt", "c+");
    var_dump(fread($h, 100));
    var_dump(ftruncate($h, 0), fseek($h, 0), fwrite($h, "fresh"));
    fclose($h);
    var_dump(file_get_contents("$base/w.txt"));

    var_dump(@fopen("$base/w.txt", "x"), $h = fopen("$base/new.txt", "x+"));
    var_dump(fwrite($h, "n"), rewind($h), fread($h, 9));
    fclose($h);
    var_dump(@fopen("$base/missing.txt", "r"), @fopen("$base/nodir/f", "w"), @fopen($f, "z"));
    err(fn() => fopen("a\0b", "r"));
    err(fn() => ftruncate(fopen($f, "r+"), -1));

    // flock
    $a = fopen("$base/lock", "c");
    $b = fopen("$base/lock", "c");
    var_dump(flock($a, LOCK_EX), flock($b, LOCK_EX | LOCK_NB, $wb), $wb);
    var_dump(flock($b, LOCK_SH | LOCK_NB, $wb), $wb);
    var_dump(flock($a, LOCK_UN), flock($b, LOCK_SH | LOCK_NB, $wb), $wb);
    var_dump(flock($a, LOCK_SH | LOCK_NB), flock($b, LOCK_UN));
    $wb = "untouched";
    var_dump(flock($a, LOCK_UN, $wb), $wb);
    err(fn() => flock($a, 0));
    err(fn() => flock($a, LOCK_NB));
    err(fn() => flock("x", LOCK_SH));
    var_dump(LOCK_SH, LOCK_EX, LOCK_UN, LOCK_NB);
    fclose($a);
    fclose($b);

    // php://memory
    $mem = fopen("php://memory", "w+");
    var_dump(fwrite($mem, "hello"), ftell($mem), stream_get_contents($mem));
    var_dump(rewind($mem), fread($mem, 2), fseek($mem, 1), fwrite($mem, "EY"), ftell($mem));
    rewind($mem);
    var_dump(stream_get_contents($mem), ftruncate($mem, 2), fstat($mem)['size'], fstat($mem)['mode']);
    rewind($mem);
    var_dump(stream_get_contents($mem), fseek($mem, -1), flock($mem, LOCK_SH));
    $mm = stream_get_meta_data($mem);
    var_dump($mm['wrapper_type'], $mm['stream_type'], $mm['mode'], $mm['uri']);
    fclose($mem);

    // tempnam
    $t = tempnam($base, "pre");
    var_dump(dirname($t) === realpath($base), str_starts_with(basename($t), "pre"), strlen(basename($t)), filesize($t));
    var_dump(decoct(fileperms($t) & 0777));
    $t2 = tempnam($base, "dir/sub/xy");
    var_dump(str_starts_with(basename($t2), "xy"), dirname($t2) === realpath($base));
    $t3 = tempnam($base, str_repeat("p", 80));
    var_dump(strlen(basename($t3)));
    $t4 = tempnam("", "e");
    var_dump(is_file($t4), dirname($t4) === realpath(sys_get_temp_dir()));
    unlink($t4);
    err(fn() => tempnam("a\0", "x"));

    // stream_is_local / stream_resolve_include_path
    $h = fopen($f, "r");
    var_dump(stream_is_local($f), stream_is_local("file:///etc"), stream_is_local("php://memory"),
        stream_is_local("http://example.com"), stream_is_local("https://x"), stream_is_local("ftp://x"),
        stream_is_local("data://text/plain,x"), stream_is_local("phar://x.phar/y"), stream_is_local($h),
        stream_is_local("compress.zlib://x"));
    fclose($h);
    var_dump(stream_resolve_include_path($f) === realpath($f));
    var_dump(stream_resolve_include_path("$base/missing"));
    var_dump(stream_resolve_include_path("file://$f") === realpath($f));
    var_dump(stream_resolve_include_path("http://example.com/x"));
    var_dump(stream_resolve_include_path("") === getcwd());
    err(fn() => stream_resolve_include_path("a\0b"));

    // readfile / fpassthru
    var_dump(readfile($f));
    echo "\n";
    var_dump(@readfile("$base/missing"));
    $h = fopen($f, "r");
    fgets($h);
    var_dump(fpassthru($h));
    echo "\n";
    var_dump(fpassthru($h));
    fclose($h);

    foreach (scandir($base) as $e) if ($e[0] !== '.') unlink("$base/$e");
    rmdir($base);
}

try { main(); } catch (Throwable $e) { echo "Uncaught: ", $e->getMessage(), "\n"; }
