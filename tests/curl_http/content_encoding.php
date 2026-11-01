<?php
// Content-Encoding is only undone when CURLOPT_ENCODING asks for it, and
// size_download always counts the bytes on the wire.
$base = '__BASE__';
$h = curl_init($base . '/gz');
curl_setopt($h, CURLOPT_RETURNTRANSFER, 1);
$raw = curl_exec($h);
var_dump(strlen($raw), bin2hex(substr($raw, 0, 4)), gzdecode($raw));
var_dump(curl_getinfo($h, CURLINFO_SIZE_DOWNLOAD), curl_getinfo($h, CURLINFO_SIZE_DOWNLOAD_T), curl_getinfo($h, CURLINFO_CONTENT_LENGTH_DOWNLOAD));
foreach (['', 'gzip', 'identity'] as $enc) {
    curl_setopt($h, CURLOPT_ENCODING, $enc);
    $body = curl_exec($h);
    echo "ENCODING '$enc': ";
    var_dump($body === $raw ? 'raw' : $body, curl_getinfo($h, CURLINFO_SIZE_DOWNLOAD), curl_errno($h));
}
curl_setopt($h, CURLOPT_ENCODING, null);
var_dump(strlen(curl_exec($h)));
// the header callback sees the encoded response's headers either way
curl_setopt($h, CURLOPT_ENCODING, '');
curl_setopt($h, CURLOPT_HEADERFUNCTION, function ($c, $l) {
    if (stripos($l, 'content-') === 0) {
        echo 'H: ', $l;
    }
    return strlen($l);
});
var_dump(curl_exec($h));
