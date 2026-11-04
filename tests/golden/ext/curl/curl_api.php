<?php
// ext/curl API surface, offline (file:// transfers + validation).
$dir = sys_get_temp_dir() . '/rhp_curl_' . getmypid();
@mkdir($dir);
$f = $dir . '/data.txt';
file_put_contents($f, "line one\nline two\n");
touch($f, 1700000000);
$url = 'file://' . $f;

echo "== loaded\n";
var_dump(extension_loaded('curl'), function_exists('curl_multi_exec'), function_exists('curl_share_init'));
var_dump(CURLOPT_URL, CURLOPT_RETURNTRANSFER, CURLE_OK, CURLM_OK, CURLMSG_DONE, CURLPROTO_HTTPS, CURL_HTTP_VERSION_2_0, CURLINFO_HTTP_CODE, CURL_LOCK_DATA_DNS, CURLSHOPT_SHARE);
$v = curl_version();
var_dump(is_array($v), isset($v['version'], $v['features'], $v['ssl_version'], $v['protocols']), is_int($v['features']));

echo "== objects\n";
$h = curl_init();
var_dump($h, $h instanceof CurlHandle, get_class(curl_multi_init()), get_class(curl_share_init()));
var_dump(is_int((int) $h), (int) $h === spl_object_id($h), intval($h) === spl_object_id($h));
$mh = curl_multi_init();
var_dump((int) $mh === spl_object_id($mh));
try { new CurlHandle(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { new CurlMultiHandle(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }

echo "== fresh info\n";
var_dump(curl_errno($h), curl_error($h));
var_dump(curl_exec($h), curl_errno($h), curl_error($h));
$i = curl_getinfo($h);
var_dump(count($i), $i['url'], $i['content_type'], $i['http_code'], $i['effective_method'], $i['redirect_url'], $i['primary_port']);
$h2 = curl_init('http://example.invalid/x');
var_dump(curl_getinfo($h2, CURLINFO_EFFECTIVE_URL), curl_getinfo($h2, CURLINFO_EFFECTIVE_METHOD));
curl_setopt($h2, CURLOPT_NOBODY, true);
var_dump(curl_getinfo($h2, CURLINFO_EFFECTIVE_METHOD));
curl_setopt($h2, CURLOPT_POSTFIELDS, 'a=b');
var_dump(curl_getinfo($h2, CURLINFO_EFFECTIVE_METHOD));
curl_setopt($h2, CURLOPT_CUSTOMREQUEST, 'PATCH');
var_dump(curl_getinfo($h2, CURLINFO_EFFECTIVE_METHOD));
var_dump(curl_getinfo($h2, CURLINFO_CONTENT_TYPE), curl_getinfo($h2, CURLINFO_REDIRECT_URL), curl_getinfo($h2, 12345));
var_dump(curl_getinfo($h2, CURLINFO_PRIVATE));
curl_setopt($h2, CURLOPT_PRIVATE, ['job' => 7]);
var_dump(curl_getinfo($h2, CURLINFO_PRIVATE));

echo "== file transfer\n";
$h = curl_init($url);
curl_setopt($h, CURLOPT_RETURNTRANSFER, true);
var_dump(curl_exec($h));
var_dump(curl_getinfo($h, CURLINFO_SIZE_DOWNLOAD), curl_getinfo($h, CURLINFO_SIZE_DOWNLOAD_T), curl_getinfo($h, CURLINFO_CONTENT_LENGTH_DOWNLOAD));
var_dump(curl_getinfo($h, CURLINFO_FILETIME), curl_getinfo($h, CURLINFO_PROTOCOL), curl_getinfo($h, CURLINFO_SCHEME), curl_getinfo($h, CURLINFO_HTTP_CODE));
curl_setopt($h, CURLOPT_HEADER, true);
var_dump(curl_exec($h));
curl_setopt($h, CURLOPT_NOBODY, true);
var_dump(curl_exec($h), curl_getinfo($h, CURLINFO_EFFECTIVE_METHOD));

echo "== callbacks\n";
$h = curl_init($url);
curl_setopt($h, CURLOPT_HEADERFUNCTION, function ($ch, $line) use (&$h) {
    echo "H[", rtrim($line), "] same=", var_export($ch === $h, true), "\n";
    return strlen($line);
});
curl_setopt($h, CURLOPT_WRITEFUNCTION, function ($ch, $data) {
    echo "W[", strlen($data), "]\n";
    return strlen($data);
});
var_dump(curl_exec($h));
curl_setopt($h, CURLOPT_WRITEFUNCTION, fn ($ch, $d) => 3);
var_dump(curl_exec($h), curl_errno($h), curl_error($h));
curl_setopt($h, CURLOPT_WRITEFUNCTION, null);
curl_setopt($h, CURLOPT_HEADERFUNCTION, fn ($ch, $l) => 0);
var_dump(curl_exec($h), curl_errno($h), curl_error($h));

echo "== stdout default\n";
$h = curl_init($url);
var_dump(curl_exec($h));

echo "== streams\n";
$hh = fopen('php://temp', 'w+b');
$bh = fopen('php://temp', 'w+b');
$h = curl_init($url);
curl_setopt($h, CURLOPT_WRITEHEADER, $hh);
curl_setopt($h, CURLOPT_FILE, $bh);
var_dump(curl_exec($h));
rewind($hh); rewind($bh);
var_dump(stream_get_contents($hh), stream_get_contents($bh));
curl_setopt($h, CURLOPT_RETURNTRANSFER, true); // later option wins
var_dump(curl_exec($h));

echo "== errors\n";
$h = curl_init('file://' . $dir . '/missing.txt');
curl_setopt($h, CURLOPT_RETURNTRANSFER, true);
var_dump(curl_exec($h), curl_errno($h), str_replace($dir, 'DIR', curl_error($h)));
$h = curl_init('foo://bar');
var_dump(curl_exec($h), curl_errno($h), curl_error($h));
// (The disabled-protocol texts differ between libcurl releases: tests/curl.rs.)
$h = curl_init('ftp://x/');
curl_setopt($h, CURLOPT_PROTOCOLS, CURLPROTO_HTTP | CURLPROTO_HTTPS);
var_dump(curl_exec($h), curl_errno($h));
$h = curl_init($url);
curl_setopt($h, CURLOPT_PROTOCOLS, CURLPROTO_HTTP);
var_dump(curl_exec($h), curl_errno($h));

echo "== validation\n";
$h = curl_init();
foreach ([1, 'x', true, null, [], new stdClass(), curl_multi_init()] as $bad) {
    try { curl_setopt($bad, CURLOPT_URL, 'x'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
}
try { curl_exec('nope'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { curl_multi_exec($h, $r); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { curl_share_setopt($h, CURLSHOPT_SHARE, 1); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { curl_setopt($h, 99999, 1); } catch (ValueError $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { curl_setopt_array($h, [99999 => 1]); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { curl_setopt_array($h, ['a' => 1]); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { curl_setopt($h, CURLOPT_HTTPHEADER, 'x'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { curl_setopt($h, CURLOPT_WRITEFUNCTION, 'no_such_fn'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { curl_setopt_array($h, [CURLOPT_HEADERFUNCTION => 'no_such_fn']); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { curl_setopt($h, CURLOPT_FILE, 'x'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { curl_setopt($h, CURLOPT_URL, "a\0b"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { curl_setopt($h, CURLOPT_SAFE_UPLOAD, false); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { curl_setopt($h, 'abc', 1); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { curl_multi_setopt(curl_multi_init(), 12345, 1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { curl_share_setopt(curl_share_init(), 12345, 1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { curl_multi_select(curl_multi_init(), -1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(curl_setopt($h, CURLOPT_TIMEOUT, '30'), curl_setopt($h, CURLOPT_ENCODING, ''), curl_setopt($h, CURLOPT_SHARE, curl_share_init()));
var_dump(curl_setopt_array($h, [CURLOPT_URL => $url, CURLOPT_RETURNTRANSFER => true, CURLOPT_FOLLOWLOCATION => true]));
var_dump(strlen(curl_exec($h)));
$s = curl_share_init();
var_dump(curl_share_setopt($s, CURLSHOPT_SHARE, CURL_LOCK_DATA_COOKIE), curl_share_errno($s));
var_dump(curl_multi_setopt(curl_multi_init(), CURLMOPT_PIPELINING, 2), curl_multi_setopt(curl_multi_init(), CURLMOPT_MAX_HOST_CONNECTIONS, 8));

echo "== copy/reset/escape\n";
$h = curl_init($url);
curl_setopt($h, CURLOPT_RETURNTRANSFER, true);
$c = curl_copy_handle($h);
var_dump($c === $h, strlen(curl_exec($c)));
curl_reset($c);
var_dump(curl_getinfo($c, CURLINFO_EFFECTIVE_URL));
var_dump(curl_escape($h, "a b&c=d/é~"), curl_unescape($h, "a%20b%26c%3Dd%2F%C3%A9~%zz+"));
var_dump(curl_escape($h, ""), curl_unescape($h, ""), curl_escape($h, "a\0b"), bin2hex(curl_unescape($h, "a%00b")));
var_dump(@curl_close($h), curl_errno($h));

echo "== multi\n";
$m = curl_multi_init();
$jobs = [];
foreach (['data.txt', 'missing.txt', 'data.txt'] as $n) {
    $c = curl_init('file://' . $dir . '/' . $n);
    curl_setopt($c, CURLOPT_RETURNTRANSFER, true);
    var_dump(curl_multi_add_handle($m, $c));
    $jobs[(int) $c] = $n;
}
var_dump(curl_multi_add_handle($m, $c), curl_multi_errno($m));
var_dump(count(curl_multi_get_handles($m)));
$results = [];
$running = null;
do {
    $rc = curl_multi_exec($m, $running);
    if ($rc !== CURLM_OK) { echo "rc $rc\n"; }
    if ($running) { curl_multi_select($m, 1.0); }
    while ($msg = curl_multi_info_read($m, $queued)) {
        $c = $msg['handle'];
        $results[$jobs[(int) $c] . '#' . (int) $c] = [$msg['msg'], $msg['result'], curl_errno($c), strlen((string) curl_multi_getcontent($c))];
        var_dump(is_int($queued));
        curl_multi_remove_handle($m, $c);
    }
} while ($running > 0);
ksort($results);
foreach ($results as $k => $r) { echo preg_replace('/#\d+/', '', $k), ' ', json_encode($r), "\n"; }
var_dump($running, curl_multi_info_read($m), count(curl_multi_get_handles($m)));
var_dump(curl_multi_remove_handle($m, curl_init()));
curl_multi_close($m);

echo "== curlfile\n";
$cf = new CURLFile('/tmp/x.bin', 'text/plain', 'up.txt');
var_dump($cf->getFilename(), $cf->getMimeType(), $cf->getPostFilename());
$cf2 = curl_file_create('/tmp/y.bin');
var_dump(get_class($cf2), $cf2->name, $cf2->mime, $cf2->postname);

unlink($f);
rmdir($dir);
