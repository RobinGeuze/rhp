<?php
// Response header lines are passed on exactly as the server sent them, in
// CURLOPT_HEADER output, HEADERFUNCTION lines and every redirect hop.
$base = '__BASE__';
$h = curl_init($base . '/hello');
curl_setopt_array($h, [CURLOPT_RETURNTRANSFER => 1, CURLOPT_HEADER => 1]);
echo curl_exec($h), "\n==\n";
var_dump(curl_getinfo($h, CURLINFO_CONTENT_TYPE), curl_getinfo($h, CURLINFO_HEADER_SIZE));

$h = curl_init($base . '/hop');
$lines = [];
curl_setopt_array($h, [
    CURLOPT_RETURNTRANSFER => 1,
    CURLOPT_FOLLOWLOCATION => 1,
    CURLOPT_HEADERFUNCTION => function ($c, $l) use (&$lines) { $lines[] = $l; return strlen($l); },
]);
var_dump(curl_exec($h));
echo implode('', $lines), "==\n";
var_dump(curl_getinfo($h, CURLINFO_HEADER_SIZE), curl_getinfo($h, CURLINFO_REDIRECT_COUNT));

$h = curl_init($base . '/hop');
curl_setopt_array($h, [CURLOPT_RETURNTRANSFER => 1, CURLOPT_HEADER => 1, CURLOPT_FOLLOWLOCATION => 1]);
echo curl_exec($h), "\n";
