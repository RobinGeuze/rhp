<?php
// clone of a CurlHandle copies the handle (curl_clone_obj, as curl_copy_handle);
// CurlMultiHandle / CurlShareHandle cannot be cloned.
$dir = sys_get_temp_dir() . '/rhp_curl_clone_' . getmypid();
@mkdir($dir);
file_put_contents("$dir/a.txt", "file a\n");
file_put_contents("$dir/b.txt", "file b\n");

$h = curl_init("file://$dir/a.txt");
curl_setopt($h, CURLOPT_RETURNTRANSFER, true);
curl_setopt($h, CURLOPT_PRIVATE, ['tag' => 1]);
$c = clone $h;
var_dump($c instanceof CurlHandle, $c === $h, $c == $h, spl_object_id($c) !== spl_object_id($h));
var_dump(curl_exec($c), curl_getinfo($c, CURLINFO_PRIVATE), curl_errno($c));
// the copy is independent
curl_setopt($c, CURLOPT_URL, "file://$dir/b.txt");
var_dump(curl_exec($c), curl_exec($h));
var_dump(basename(curl_getinfo($h, CURLINFO_EFFECTIVE_URL)), basename(curl_getinfo($c, CURLINFO_EFFECTIVE_URL)));
// a clone taken after a transfer starts with fresh transfer state
$d = clone $h;
var_dump(curl_getinfo($d, CURLINFO_HTTP_CODE), curl_getinfo($d, CURLINFO_SIZE_DOWNLOAD), basename(curl_getinfo($d, CURLINFO_EFFECTIVE_URL)));
// callbacks carry over
$seen = [];
curl_setopt($h, CURLOPT_WRITEFUNCTION, function ($ch, $data) use (&$seen) { $seen[] = trim($data); return strlen($data); });
$e = clone $h;
var_dump(curl_exec($e), $seen);
// cloning a clone, and curl functions on it
$f = clone $e;
curl_setopt($f, CURLOPT_RETURNTRANSFER, true);
curl_setopt($f, CURLOPT_URL, "file://$dir/b.txt");
var_dump(curl_exec($f), count($seen));
curl_reset($f);
var_dump(curl_getinfo($f, CURLINFO_PRIVATE));
$m = curl_multi_init();
var_dump(curl_multi_add_handle($m, clone $f));

foreach ([curl_multi_init(), curl_share_init()] as $x) {
    try {
        $y = clone $x;
        echo "cloned\n";
    } catch (Error $err) {
        echo get_class($err), ": ", $err->getMessage(), "\n";
    }
}
unlink("$dir/a.txt");
unlink("$dir/b.txt");
rmdir($dir);
