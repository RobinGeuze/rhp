<?php
// Stream filters: what the phpt files leave open (call sequence and flags
// per stream kind, positions, parameters, php://filter writes).

class trace_filter extends php_user_filter {
    function onCreate(): bool { echo "create {$this->filtername}\n"; return true; }
    function onClose(): void { echo "close {$this->filtername}\n"; }
    function filter($in, $out, &$consumed, $closing): int {
        $n = 0;
        while ($b = stream_bucket_make_writeable($in)) {
            $n += $b->datalen;
            $consumed += $b->datalen;
            $b->data = strtoupper($b->data);
            stream_bucket_append($out, $b);
        }
        echo "  filter($n, ", $closing ? "closing" : "open", ", ", var_export($consumed, true), ")\n";
        return PSFS_PASS_ON;
    }
}
set_error_handler(function ($no, $str) {
    echo "E$no: $str\n";
    return true;
});
var_dump(stream_filter_register("trace.*", "trace_filter"));
var_dump(in_array("trace.*", stream_get_filters()));

$file = sys_get_temp_dir() . "/stream_filters.tmp";
file_put_contents($file, str_repeat("abc\n", 3000));

foreach (["php://memory", "php://temp", $file] as $i => $url) {
    echo "-- ", $i, "\n";
    $fp = fopen($url, $i < 2 ? "w+" : "r");
    if ($i < 2) {
        fwrite($fp, str_repeat("abc\n", 3000));
        rewind($fp);
    }
    $f = stream_filter_append($fp, "trace.x", STREAM_FILTER_READ);
    var_dump(fgets($fp), ftell($fp));
    var_dump(strlen(fread($fp, 10000)), ftell($fp), feof($fp));
    var_dump(strlen(stream_get_contents($fp)), feof($fp));
    var_dump(stream_filter_remove($f));
    var_dump(fseek($fp, 4), fgets($fp), ftell($fp));
    fclose($fp);
}

echo "-- write chain\n";
$fp = fopen($file, "w");
stream_filter_append($fp, "trace.w", STREAM_FILTER_WRITE);
stream_filter_prepend($fp, "string.rot13", STREAM_FILTER_WRITE);
var_dump(fwrite($fp, "hello\n"), ftell($fp));
var_dump(fflush($fp));
fclose($fp);
var_dump(file_get_contents($file));

echo "-- php://filter\n";
var_dump(file_put_contents("php://filter/write=string.toupper|string.rot13/resource=$file", "abc"));
var_dump(file_get_contents($file));
var_dump(file_get_contents("php://filter/read=string.rot13/resource=$file"));
var_dump(file("php://filter/string.tolower/resource=data:,A%0AB"));
var_dump(file_get_contents("php://filter/read=nope|string.toupper/resource=data:,x"));

echo "-- convert\n";
$fp = fopen("php://temp", "w+");
stream_filter_append($fp, "convert.base64-encode", STREAM_FILTER_WRITE, ["line-length" => 8, "line-break-chars" => "|"]);
fwrite($fp, "The quick brown fox");
fclose($fp);
foreach ([
    ["convert.quoted-printable-encode", ["line-length" => 10, "binary" => false], "über café\tend \nnext"],
    ["convert.quoted-printable-encode", ["force-encode-first" => true, "line-length" => 20], "From here"],
    ["convert.quoted-printable-encode", null, "a\tb \r\nc"],
    ["convert.base64-decode", null, "SGVs bG8=\n"],
    ["convert.base64-decode", null, "SGVsbG8"],
    ["convert.quoted-printable-decode", null, "a=3Db=\r\nc"],
    ["convert.quoted-printable-decode", ["line-break-chars" => "\n"], "x=\ny=4"],
    ["convert.unknown", null, "x"],
    ["convert.base64-encode", "notarray", "x"],
] as [$name, $opts, $data]) {
    $fp = fopen("php://memory", "w+");
    fwrite($fp, $data);
    rewind($fp);
    var_dump($opts === null ? stream_filter_append($fp, $name) !== false : stream_filter_append($fp, $name, STREAM_FILTER_READ, $opts) !== false);
    var_dump(stream_get_contents($fp));
}

echo "-- zlib / bzip2\n";
$data = str_repeat("zlib filter data ", 200);
foreach ([["zlib.deflate", "zlib.inflate", null], ["zlib.deflate", "zlib.inflate", ["level" => 9, "window" => 31]], ["bzip2.compress", "bzip2.decompress", ["blocks" => 1]]] as [$enc, $dec, $p]) {
    $fp = fopen("php://temp", "w+");
    stream_filter_append($fp, $enc, STREAM_FILTER_WRITE, $p);
    fwrite($fp, $data);
    fclose($fp);
    $fp = fopen("php://temp", "w+");
    $f = stream_filter_append($fp, $enc, STREAM_FILTER_WRITE, $p);
    fwrite($fp, $data);
    stream_filter_remove($f);
    rewind($fp);
    $packed = stream_get_contents($fp);
    var_dump(strlen($packed) < strlen($data));
    rewind($fp);
    stream_filter_append($fp, $dec, STREAM_FILTER_READ, $p);
    var_dump(stream_get_contents($fp) === $data);
}
$fp = fopen("php://memory", "w+");
var_dump(stream_filter_append($fp, "zlib.deflate", STREAM_FILTER_WRITE, ["level" => 12]) !== false);
var_dump(stream_filter_append($fp, "zlib.bogus"));

echo "-- iconv\n";
$fp = fopen("php://memory", "w+");
fwrite($fp, "caf\xc3\xa9");
rewind($fp);
stream_filter_append($fp, "convert.iconv.UTF-8/ISO-8859-1");
var_dump(bin2hex(stream_get_contents($fp)));

echo "-- errors\n";
$fp = fopen("php://memory", "w+");
var_dump(stream_filter_append($fp, "no.such.filter"));
try {
    stream_bucket_make_writeable($fp);
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
try {
    stream_filter_remove($fp);
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
stream_filter_register("missing", "NoSuchClass");
var_dump(stream_filter_append($fp, "missing"));
// the warning names the last wildcard tried: "a.b.*" fails, "a.*" is absent
stream_filter_register("wild.b.*", "NoSuchClass");
var_dump(stream_filter_append($fp, "wild.b.c"));
stream_filter_register("wild.*", "NoSuchClass");
var_dump(stream_filter_append($fp, "wild.b.c"));
class BadArgs extends php_user_filter {}
$f = new BadArgs;
try {
    $f->filter(1, 2, $c, false);
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
unlink($file);
echo "done\n";
