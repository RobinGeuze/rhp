<?php
// An exception thrown by a curl callback is held until the curl function that
// drove the transfer returns: the transfer completes, every finished transfer
// of the same curl_multi_exec is still delivered and reported, later callbacks
// are skipped.
$dir = sys_get_temp_dir() . '/rhp_curl_cbex_' . getmypid();
@mkdir($dir);
file_put_contents("$dir/a.txt", "hello world");

echo "== multi\n";
$calls = 0;
$a = curl_init("file://$dir/a.txt");
curl_setopt($a, CURLOPT_WRITEFUNCTION, function ($h, $d) use (&$calls) { $calls++; throw new Exception('boom'); });
$b = curl_init("file://$dir/a.txt");
curl_setopt($b, CURLOPT_RETURNTRANSFER, 1);
$c = curl_init("file://$dir/a.txt");
curl_setopt($c, CURLOPT_HEADERFUNCTION, function ($h, $l) { return strlen($l); });
curl_setopt($c, CURLOPT_RETURNTRANSFER, 1);
$m = curl_multi_init();
curl_multi_add_handle($m, $a);
curl_multi_add_handle($m, $b);
curl_multi_add_handle($m, $c);
$msgs = [];
$caught = 0;
do {
    try {
        curl_multi_exec($m, $running);
    } catch (Exception $e) {
        $caught++;
        echo "caught ", $e->getMessage(), " running=", var_export(is_int($running), true), "\n";
    }
    while ($info = curl_multi_info_read($m)) {
        $msgs[] = [$info['handle'] === $a ? 'a' : ($info['handle'] === $b ? 'b' : 'c'), $info['result']];
    }
    if ($running) {
        curl_multi_select($m, 0.2);
    }
} while ($running);
sort($msgs);
echo json_encode($msgs), "\n";
var_dump($caught, $calls, curl_multi_getcontent($b), curl_multi_getcontent($c), curl_errno($a));

echo "== curl_exec, header callback\n";
$n = 0;
$h = curl_init("file://$dir/a.txt");
curl_setopt($h, CURLOPT_HEADERFUNCTION, function ($c, $l) use (&$n) { $n++; throw new RuntimeException("hdr $n"); });
curl_setopt($h, CURLOPT_RETURNTRANSFER, 1);
try {
    var_dump(curl_exec($h));
} catch (Exception $e) {
    echo get_class($e), " ", $e->getMessage(), " n=$n\n";
}
var_dump(curl_errno($h), curl_getinfo($h, CURLINFO_SIZE_DOWNLOAD));
// the handle still works afterwards
curl_setopt($h, CURLOPT_HEADERFUNCTION, null);
var_dump(curl_exec($h));

echo "== progress callback\n";
$p = 0;
$h = curl_init("file://$dir/a.txt");
curl_setopt($h, CURLOPT_NOPROGRESS, false);
curl_setopt($h, CURLOPT_XFERINFOFUNCTION, function () use (&$p) { $p++; throw new LogicException("progress"); });
curl_setopt($h, CURLOPT_RETURNTRANSFER, 1);
try {
    var_dump(curl_exec($h));
} catch (LogicException $e) {
    echo "caught ", $e->getMessage(), " p=$p\n";
}
var_dump(curl_errno($h));
unlink("$dir/a.txt");
rmdir($dir);
