<?php
// the gzip header's OS byte (offset 9) is platform-specific (zlib OS_CODE): mask it
function noos($g) { return substr_replace($g, '?', 9, 1); }
$inputs = ['', 'abc', str_repeat('hello ', 100), implode(',', range(1, 3000))];
// a larger, less repetitive input (exercises memLevel / block splitting)
$big = '';
$x = 12345;
for ($i = 0; $i < 60000; $i++) {
    $x = ($x * 1103515245 + 12345) & 0x7fffffff;
    $big .= chr(97 + ($x >> 16) % 7);
}
$inputs[] = $big;
foreach ($inputs as $s) {
    foreach ([-1, 0, 1, 6, 9] as $lvl) {
        echo strlen($s), " L$lvl: ",
            md5(gzdeflate($s, $lvl)), ' ',
            md5(gzcompress($s, $lvl)), ' ',
            md5(noos(gzencode($s, $lvl))), "\n";
    }
    var_dump(gzinflate(gzdeflate($s)) === $s, gzuncompress(gzcompress($s)) === $s, gzdecode(gzencode($s)) === $s);
    var_dump(zlib_decode(gzdeflate($s)) === $s, zlib_decode(gzcompress($s)) === $s, zlib_decode(gzencode($s)) === $s);
}
echo bin2hex(noos(gzencode('abc'))), "\n", bin2hex(noos(gzencode('abc', 9))), "\n", bin2hex(noos(gzencode('abc', 1))), "\n";
echo bin2hex(gzcompress('abc', 0)), ' ', bin2hex(gzcompress('abc', 1)), ' ', bin2hex(gzcompress('abc', 9)), "\n";
echo bin2hex(noos(gzdeflate('abc', 6, ZLIB_ENCODING_GZIP))), "\n";
echo bin2hex(gzencode('abc', -1, ZLIB_ENCODING_RAW)), "\n";
echo bin2hex(noos(gzcompress('abc', -1, FORCE_GZIP))), "\n";
echo bin2hex(zlib_encode('abc', ZLIB_ENCODING_DEFLATE)), ' ', bin2hex(zlib_encode('abc', ZLIB_ENCODING_RAW, 9)), "\n";
// errors: bad data → false
var_dump(@gzinflate('not deflate data'), @gzuncompress('xyz'), @gzdecode('abc'), @zlib_decode(''), @gzinflate(''));
var_dump(@gzuncompress(substr(gzcompress('hello world'), 0, -3)));
var_dump(@gzdecode(substr(gzencode('hello world'), 0, 5)));
// max_length
$c = gzcompress('hello world');
var_dump(@gzuncompress($c, 5), gzuncompress($c, 11), gzuncompress($c, 100), @gzuncompress($c, 10));
var_dump(@gzinflate(gzdeflate(str_repeat('a', 1000)), 999), strlen(gzinflate(gzdeflate(str_repeat('a', 1000)), 1000)));
// trailing garbage after the stream is ignored
var_dump(gzuncompress($c . 'garbage'), gzinflate(gzdeflate('abc') . 'zzz'));
foreach ([
    fn() => gzcompress('a', 10),
    fn() => gzdeflate('a', -2),
    fn() => gzencode('a', 1, 99),
    fn() => zlib_encode('a', 1),
    fn() => zlib_encode('a', ZLIB_ENCODING_GZIP, 11),
    fn() => gzinflate('a', -1),
    fn() => zlib_decode('a', -5),
] as $f) {
    try {
        $f();
    } catch (ValueError $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
var_dump(ZLIB_ENCODING_RAW, ZLIB_ENCODING_GZIP, ZLIB_ENCODING_DEFLATE, FORCE_GZIP, FORCE_DEFLATE);
var_dump(ZLIB_NO_FLUSH, ZLIB_SYNC_FLUSH, ZLIB_FINISH, ZLIB_RLE, ZLIB_FIXED, ZLIB_BUF_ERROR, ZLIB_DATA_ERROR);
var_dump(zlib_get_coding_type());

// compress.zlib:// + gzopen family
$dir = sys_get_temp_dir() . '/rhp_golden_zlib';
@mkdir($dir);
$f = "$dir/a.gz";
var_dump(file_put_contents("compress.zlib://$f", "line1\nline2\nline3"));
echo bin2hex(noos(file_get_contents($f))), "\n";
var_dump(file_get_contents("compress.zlib://$f"));
$g = gzopen($f, 'rb');
var_dump(is_resource($g), gzgets($g), gzread($g, 3), gzgetc($g), gzeof($g), gzread($g, 100), gzeof($g));
var_dump(gzrewind($g), gzread($g, 5), gzclose($g));
var_dump(gzfile($f));
$g = gzopen($f, 'wb');
var_dump(gzwrite($g, 'written '), gzputs($g, 'via gz'), gzclose($g));
echo bin2hex(noos(file_get_contents($f))), "\n";
var_dump(gzdecode(file_get_contents($f)));
ob_start();
$n = readgzfile($f);
$out = ob_get_clean();
var_dump($n, $out);
// plain (non-gzip) files read transparently
file_put_contents("$dir/plain.txt", "not compressed\n");
var_dump(file_get_contents("compress.zlib://$dir/plain.txt"), gzfile("$dir/plain.txt"));
// concatenated members
file_put_contents("$dir/multi.gz", gzencode('one-') . gzencode('two'));
var_dump(file_get_contents("compress.zlib://$dir/multi.gz"));
var_dump(@gzopen("$dir/a.gz", 'r+'));
var_dump(@gzopen("$dir/missing.gz", 'r'));
foreach (['a.gz', 'plain.txt', 'multi.gz'] as $p) {
    unlink("$dir/$p");
}
rmdir($dir);
