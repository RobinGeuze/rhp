<?php
// Stream-function edge cases: argument validation, closed handles, fread/
// fwrite lengths, stream_get_contents offsets, dir handles passed to stream
// functions, php://memory sparse writes, php's buffer-discarding failed seek.
function err($f) {
    try { var_dump($f()); }
    catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}

function main() {
    $dir = sys_get_temp_dir() . "/rhp_sedge_" . bin2hex(random_bytes(6));
    mkdir($dir);
    $f = "$dir/data.txt";
    file_put_contents($f, "<?php\n" . str_repeat("0123456789\n", 20));
    $h = fopen($f, "r");
    var_dump(fgets($h, 1), ftell($h), fgets($h, 2), ftell($h));
    err(fn() => fread($h, 0));
    err(fn() => fread($h, -1));
    // a failed seek drops php's read buffer without re-syncing the descriptor
    var_dump(fseek($h, 0, 99), ftell($h));
    var_dump(stream_get_contents($h, 5), feof($h));
    var_dump(stream_get_contents($h, 3, 0), ftell($h));
    var_dump(stream_get_contents($h, -1, 10) === substr(file_get_contents($f), 10));
    var_dump(stream_get_contents($h, 0, 0), ftell($h));
    err(fn() => stream_get_contents($h, -2));
    var_dump(fgetc($h), fgetc($h));
    fseek($h, 0, SEEK_END);
    var_dump(fgetc($h), feof($h));

    $d = opendir($dir);
    var_dump(@fgets($d), ftell($d), feof($d));
    readdir($d);
    var_dump(ftell($d) > 0);
    var_dump(@fclose($d), is_resource($d));
    closedir($d);

    $m = fopen("php://memory", "r+");
    var_dump(fwrite($m, "abc\ndef"), fwrite($m, "zz", 0), fwrite($m, "zz", -5), fwrite($m, ""), fwrite($m, "123", 2));
    rewind($m);
    var_dump(fgets($m, 1), fgets($m), fgets($m), fgets($m), feof($m));
    var_dump(fseek($m, 20), ftell($m), fwrite($m, "X"), ftell($m));
    rewind($m);
    var_dump(bin2hex(stream_get_contents($m)));
    var_dump(fflush($m), fclose($m));

    var_dump(fflush($h), fclose($h));
    err(fn() => fclose($h));
    err(fn() => fwrite($h, "x"));
    err(fn() => fread($h, 1));
    err(fn() => feof($h));
    err(fn() => fgetc($h));
    err(fn() => fflush($h));
    err(fn() => fgets($h));
    err(fn() => stream_get_contents($h));
    err(fn() => fwrite("str", "x"));
    err(fn() => feof(null));
    err(fn() => fclose([]));

    unlink($f);
    rmdir($dir);
}

try { main(); } catch (Throwable $e) { echo "Uncaught: ", get_class($e), ": ", $e->getMessage(), "\n"; }
