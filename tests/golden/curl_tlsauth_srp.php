<?php
// The TLS-SRP options: libcurl 8.23 on OpenSSL 4 (no SRP) refuses every value.
$h = curl_init();
foreach ([CURLOPT_TLSAUTH_USERNAME, CURLOPT_TLSAUTH_PASSWORD, CURLOPT_TLSAUTH_TYPE, CURLOPT_PROXY_TLSAUTH_USERNAME,
          CURLOPT_PROXY_TLSAUTH_PASSWORD, CURLOPT_PROXY_TLSAUTH_TYPE] as $o) {
    foreach (['u', '', null, 'SRP', 1] as $v) {
        var_dump(curl_setopt($h, $o, $v));
    }
    var_dump(curl_setopt_array($h, [$o => 'x']));
}
