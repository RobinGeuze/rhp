<?php
// deflate_init/deflate_add and inflate_init/inflate_add round trips, the
// option checks and the status getters.
$data = str_repeat("hello world ", 200) . implode(',', range(1, 300));
foreach ([ZLIB_ENCODING_RAW, ZLIB_ENCODING_DEFLATE, ZLIB_ENCODING_GZIP] as $enc) {
    $d = deflate_init($enc, ['level' => 6]);
    $c = '';
    foreach (str_split($data, 333) as $chunk) {
        $c .= deflate_add($d, $chunk, ZLIB_NO_FLUSH);
    }
    $c .= deflate_add($d, '', ZLIB_FINISH);
    $i = inflate_init($enc);
    $o = '';
    foreach (str_split($c, 50) as $chunk) {
        $o .= inflate_add($i, $chunk, ZLIB_SYNC_FLUSH);
    }
    var_dump($o === $data, inflate_get_status($i), inflate_get_read_len($i) === strlen($c));
    // the context resets after the stream end
    $again = deflate_add($d, 'x', ZLIB_FINISH);
    var_dump(zlib_decode($again) === 'x' || $enc === ZLIB_ENCODING_RAW);
}
// a dictionary
$dict = ['hello', 'world'];
$d = deflate_init(ZLIB_ENCODING_DEFLATE, ['dictionary' => $dict]);
$c = deflate_add($d, 'hello world hello', ZLIB_FINISH);
$i = inflate_init(ZLIB_ENCODING_DEFLATE, ['dictionary' => $dict]);
var_dump(inflate_add($i, $c));
$i = inflate_init(ZLIB_ENCODING_DEFLATE);
var_dump(@inflate_add($i, $c));
foreach ([
    fn() => deflate_init(99),
    fn() => deflate_init(ZLIB_ENCODING_RAW, ['level' => 10]),
    fn() => deflate_init(ZLIB_ENCODING_RAW, ['memory' => 0]),
    fn() => deflate_init(ZLIB_ENCODING_RAW, ['window' => 7]),
    fn() => deflate_init(ZLIB_ENCODING_RAW, ['strategy' => 42]),
    fn() => deflate_init(ZLIB_ENCODING_RAW, ['dictionary' => ['a', '']]),
    fn() => deflate_init(ZLIB_ENCODING_RAW, ['dictionary' => 1.5]),
    fn() => inflate_init(5),
    fn() => inflate_init(ZLIB_ENCODING_RAW, ['window' => 16]),
    fn() => deflate_add(deflate_init(ZLIB_ENCODING_RAW), 'x', 99),
    fn() => inflate_add(deflate_init(ZLIB_ENCODING_RAW), 'x'),
    fn() => new DeflateContext(),
    fn() => new InflateContext(),
] as $f) {
    try {
        $f();
    } catch (Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
var_dump(deflate_init(ZLIB_ENCODING_GZIP), (new ReflectionClass('InflateContext'))->isFinal());
var_dump(zlib_get_coding_type(), ob_gzhandler('abc', PHP_OUTPUT_HANDLER_START));
