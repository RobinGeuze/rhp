<?php
// The request header set libcurl sends: no User-Agent or Accept-Encoding
// unless asked for, "Name:" removes a default, "Name;" sends an empty one.
$base = '__BASE__';
function show(array $opts) {
    global $base;
    $h = curl_init($base . '/echo');
    curl_setopt_array($h, [CURLOPT_RETURNTRANSFER => 1] + $opts);
    echo curl_exec($h), "\n==\n";
}
show([]);
show([CURLOPT_HTTPHEADER => ['Accept:']]);
show([CURLOPT_HTTPHEADER => ['Accept: text/x', 'User-Agent;', 'X-Empty;', 'X-Gone:', 'Host: example.test']]);
show([CURLOPT_ENCODING => '']);
show([CURLOPT_ENCODING => 'gzip']);
show([CURLOPT_USERAGENT => 'ua/1', CURLOPT_REFERER => 'http://ref.test/', CURLOPT_POSTFIELDS => 'a=b']);
show([CURLOPT_USERAGENT => '', CURLOPT_POSTFIELDS => 'x=1', CURLOPT_HTTPHEADER => ['Content-Type: text/plain']]);
show([CURLOPT_CUSTOMREQUEST => 'PATCH', CURLOPT_POSTFIELDS => '{}', CURLOPT_HTTPHEADER => ['Content-Type: application/json', 'Accept: application/json']]);
