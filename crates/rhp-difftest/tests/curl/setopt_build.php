<?php
// curl_setopt results that depend on how libcurl was built: HTTP/3, NTLM,
// SRP, RTMP and SMB support, and the width of CURLAUTH_ANY.
$cases = [
    'HTTP_VERSION' => [30, 31],
    'HTTPAUTH' => [CURLAUTH_NTLM, CURLAUTH_ANY, CURLAUTH_ANYSAFE],
    'PROXYAUTH' => [CURLAUTH_NTLM, CURLAUTH_BASIC | CURLAUTH_NTLM],
];
foreach ($cases as $name => $values) {
    $o = constant("CURLOPT_$name");
    $r = [];
    foreach ($values as $v) {
        $r[] = $v . '=' . var_export(curl_setopt(curl_init(), $o, $v), true);
    }
    echo str_pad($name, 15), implode(' ', $r), "\n";
}
foreach (['PROTOCOLS_STR', 'REDIR_PROTOCOLS_STR'] as $name) {
    $o = constant("CURLOPT_$name");
    foreach (['rtmp', 'smb'] as $v) {
        echo $name, " '$v' => ", var_export(curl_setopt(curl_init(), $o, $v), true), "\n";
    }
}
foreach (['TLSAUTH_TYPE' => ['SRP', 'srp'], 'PROXY_TLSAUTH_TYPE' => ['SRP']] as $name => $values) {
    $o = constant("CURLOPT_$name");
    foreach ($values as $v) {
        echo $name, " '$v' => ", var_export(curl_setopt(curl_init(), $o, $v), true), "\n";
    }
}
// The disabled-protocol error texts.
$f = tempnam(sys_get_temp_dir(), 'rhpcurl');
foreach (['ftp://x/' => CURLPROTO_HTTP | CURLPROTO_HTTPS, "file://$f" => CURLPROTO_HTTP] as $u => $p) {
    $h = curl_init($u);
    curl_setopt($h, CURLOPT_PROTOCOLS, $p);
    curl_setopt($h, CURLOPT_RETURNTRANSFER, true);
    var_dump(curl_exec($h), curl_errno($h), str_replace($f, 'FILE', curl_error($h)));
}
unlink($f);
