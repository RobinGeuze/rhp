<?php
// libcurl logs from curl_setopt (an added cookie) through the debug callback
// or into CURLOPT_STDERR. The text differs between libcurl builds.
$f = tempnam(sys_get_temp_dir(), 'rhpcurl');
file_put_contents($f, "hello");
function verbose($f) {
    $ch = curl_init("file://$f");
    curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
    curl_setopt($ch, CURLOPT_VERBOSE, true);
    curl_setopt($ch, CURLOPT_COOKIEFILE, '');
    return $ch;
}
$ch = verbose($f);
curl_setopt($ch, CURLOPT_DEBUGFUNCTION, function ($h, $t, $d) { echo "DBG[$t] ", trim($d), "\n"; });
curl_exec($ch);
var_dump(curl_setopt($ch, CURLOPT_COOKIELIST, 'Set-Cookie: a=b; domain=example.com'));

// From inside another handle's callback.
$c2 = curl_init("file://$f");
curl_setopt($c2, CURLOPT_WRITEFUNCTION, function ($x, $d) use ($ch) {
    echo "in write\n";
    var_dump(curl_setopt($ch, CURLOPT_COOKIELIST, 'Set-Cookie: c=d; domain=example.com'));
    var_dump(curl_copy_handle($ch) instanceof CurlHandle);
    return strlen($d);
});
var_dump(curl_exec($c2));

// A debug callback that throws: curl_setopt throws it; curl_setopt_array
// sets the remaining options first.
$c3 = verbose($f);
curl_exec($c3);
curl_setopt($c3, CURLOPT_DEBUGFUNCTION, function ($h, $t, $d) { throw new Exception("dbg: " . trim($d)); });
try {
    var_dump(curl_setopt($c3, CURLOPT_COOKIELIST, 'Set-Cookie: e=f; domain=example.com'));
} catch (Exception $e) {
    echo "caught ", $e->getMessage(), "\n";
}
try {
    var_dump(curl_setopt_array($c3, [
        CURLOPT_COOKIELIST => 'Set-Cookie: g=h; domain=example.com',
        CURLOPT_PRIVATE => 'p',
    ]));
} catch (Exception $e) {
    echo "caught ", $e->getMessage(), "\n";
}
var_dump(curl_getinfo($c3, CURLINFO_PRIVATE));

// Without a debug callback the text goes to CURLOPT_STDERR.
$c4 = verbose($f);
$err = fopen('php://memory', 'w+');
curl_setopt($c4, CURLOPT_STDERR, $err);
curl_exec($c4);
ftruncate($err, 0);
rewind($err);
curl_setopt($c4, CURLOPT_COOKIELIST, 'Set-Cookie: i=j; domain=example.com');
rewind($err);
var_dump(stream_get_contents($err));
unlink($f);
