<?php
// A CURLOPT_FILE / CURLOPT_WRITEHEADER stream closed before (or during) the
// transfer: php's _php_curl_verify_handlers drops it (with a warning, silenced
// here) and falls back to stdout / ignoring headers.
$dir = sys_get_temp_dir() . '/rhp_curl_closed_' . getmypid();
@mkdir($dir);
file_put_contents("$dir/a.txt", "body text\n");
$url = "file://$dir/a.txt";

echo "== FILE closed before exec\n";
$f = fopen('php://memory', 'w+');
$h = curl_init($url);
curl_setopt($h, CURLOPT_FILE, $f);
fclose($f);
var_dump(@curl_exec($h));
var_dump(@curl_exec($h));

echo "== WRITEHEADER closed before exec\n";
$f = fopen('php://memory', 'w+');
$h = curl_init($url);
curl_setopt($h, CURLOPT_WRITEHEADER, $f);
curl_setopt($h, CURLOPT_RETURNTRANSFER, 1);
fclose($f);
var_dump(@curl_exec($h));

echo "== FILE closed in a header callback\n";
$f = fopen('php://memory', 'w+');
$h = curl_init($url);
curl_setopt($h, CURLOPT_FILE, $f);
curl_setopt($h, CURLOPT_HEADERFUNCTION, function ($c, $l) use ($f) {
    if (is_resource($f)) {
        fclose($f);
    }
    return strlen($l);
});
var_dump(@curl_exec($h));

echo "== open streams still work\n";
$f = fopen('php://memory', 'w+');
$hf = fopen('php://memory', 'w+');
$h = curl_init($url);
curl_setopt($h, CURLOPT_FILE, $f);
curl_setopt($h, CURLOPT_WRITEHEADER, $hf);
var_dump(curl_exec($h));
rewind($f);
var_dump(stream_get_contents($f));

echo "== multi\n";
$f = fopen('php://memory', 'w+');
$h = curl_init($url);
curl_setopt($h, CURLOPT_FILE, $f);
$m = curl_multi_init();
fclose($f);
var_dump(@curl_multi_add_handle($m, $h));
do {
    @curl_multi_exec($m, $running);
    if ($running) {
        curl_multi_select($m, 0.2);
    }
} while ($running);
echo "\n";
$i = curl_multi_info_read($m);
var_dump($i['result']);

echo "== setopt with a closed stream\n";
foreach ([CURLOPT_FILE, CURLOPT_WRITEHEADER, CURLOPT_INFILE, CURLOPT_STDERR] as $opt) {
    try {
        curl_setopt($h, $opt, $f);
    } catch (TypeError $e) {
        echo $e->getMessage(), "\n";
    }
}
unlink("$dir/a.txt");
rmdir($dir);
