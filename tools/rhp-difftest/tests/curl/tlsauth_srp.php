<?php
// The TLS-SRP options: libcurl accepts them only when its TLS library has SRP
// (OpenSSL 3 does, OpenSSL 4 does not).
$h = curl_init();
foreach ([CURLOPT_TLSAUTH_USERNAME, CURLOPT_TLSAUTH_PASSWORD, CURLOPT_TLSAUTH_TYPE, CURLOPT_PROXY_TLSAUTH_USERNAME,
          CURLOPT_PROXY_TLSAUTH_PASSWORD, CURLOPT_PROXY_TLSAUTH_TYPE] as $o) {
    foreach (['u', '', null, 'SRP', 1] as $v) {
        var_dump(curl_setopt($h, $o, $v));
    }
    var_dump(curl_setopt_array($h, [$o => 'x']));
}
