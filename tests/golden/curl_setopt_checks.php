<?php
// curl_setopt argument checks: writable stream handles, string conversion of
// option values, CURLOPT_SSL_VERIFYHOST 1, and libcurl's own range checks.
$dir = sys_get_temp_dir() . '/rhp_curl_setopt_' . getmypid();
@mkdir($dir);
$p = "$dir/f.txt";
$h = curl_init();

echo "== stream modes\n";
foreach (['r', 'rb', 'r+', 'rb+', 'w', 'wb', 'a', 'a+', 'x+', 'c'] as $mode) {
    @unlink($p);
    if ($mode[0] === 'r') {
        touch($p);
    }
    $f = fopen($p, $mode);
    $out = [];
    foreach (['FILE' => CURLOPT_FILE, 'WRITEHEADER' => CURLOPT_WRITEHEADER, 'STDERR' => CURLOPT_STDERR, 'INFILE' => CURLOPT_INFILE] as $n => $o) {
        try {
            $out[] = $n . '=' . var_export(curl_setopt($h, $o, $f), true);
        } catch (ValueError $e) {
            $out[] = $n . ': ' . $e->getMessage();
        }
    }
    echo str_pad($mode, 4), implode(' | ', $out), "\n";
    fclose($f);
}
@unlink($p);
foreach ([5, 'x', [], null] as $v) {
    try {
        var_dump(curl_setopt($h, CURLOPT_FILE, $v));
    } catch (TypeError $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
touch($p);
try {
    curl_setopt_array($h, [CURLOPT_FILE => fopen($p, 'r')]);
} catch (ValueError $e) {
    echo "array: ", $e->getMessage(), "\n";
}
unlink($p);

echo "== string conversion\n";
class Str { function __toString(): string { return "http://example.invalid/str"; } }
foreach (['URL' => CURLOPT_URL, 'POSTFIELDS' => CURLOPT_POSTFIELDS, 'USERAGENT' => CURLOPT_USERAGENT, 'CUSTOMREQUEST' => CURLOPT_CUSTOMREQUEST, 'COOKIEFILE' => CURLOPT_COOKIEFILE] as $n => $o) {
    foreach (['stdClass' => new stdClass, 'Closure' => function () {}, 'Str' => new Str] as $k => $v) {
        try {
            $r = var_export(curl_setopt($h, $o, $v), true);
            echo "$n $k: $r\n";
        } catch (Error $e) {
            echo "$n $k: ", get_class($e), ": ", $e->getMessage(), "\n";
        }
    }
}
var_dump(curl_getinfo($h, CURLINFO_EFFECTIVE_URL));
try {
    curl_setopt($h, CURLOPT_HTTPHEADER, ['A: b', new stdClass]);
} catch (Error $e) {
    echo "HTTPHEADER: ", $e->getMessage(), "\n";
}
var_dump(@curl_setopt($h, CURLOPT_URL, ['x']), curl_getinfo($h, CURLINFO_EFFECTIVE_URL));
var_dump(curl_setopt($h, CURLOPT_URL, 12.5), curl_getinfo($h, CURLINFO_EFFECTIVE_URL));

echo "== SSL_VERIFYHOST\n";
var_dump(@curl_setopt($h, CURLOPT_SSL_VERIFYHOST, 1), curl_setopt($h, CURLOPT_SSL_VERIFYHOST, 2), curl_setopt($h, CURLOPT_SSL_VERIFYHOST, 0));

echo "== libcurl range checks\n";
$cases = [
    'TIMEOUT' => [-1, 0, 30],
    'TIMEOUT_MS' => [-1, 0],
    'CONNECTTIMEOUT' => [-2, 0, 10],
    'MAXREDIRS' => [-5, -2, -1, 0, 20],
    'SSLVERSION' => [99, 2, 3, 0, 1, 6, 7, 8, CURL_SSLVERSION_TLSv1_2 | CURL_SSLVERSION_MAX_TLSv1_3, 1 | (8 << 16)],
    'PROXYTYPE' => [12345, -1, 0, 7, 8],
    'HTTP_VERSION' => [99, -1, 0, 2, 3, 4, 5, 6, 30, 31, 32],
    'IPRESOLVE' => [99, -1, 0, 1, 2, 3],
    'PORT' => [99999, -1, 0, 80, 65535, 65536],
    'FOLLOWLOCATION' => [0, 1, 2, 3, 4, -1],
    'BUFFERSIZE' => [-1, 0, 1024],
    'INFILESIZE' => [-2, -1, 0],
    'HTTPAUTH' => [0, CURLAUTH_BASIC, CURLAUTH_NTLM, CURLAUTH_ANY, CURLAUTH_ONLY, CURLAUTH_DIGEST_IE, CURLAUTH_ANYSAFE],
    'PROXYAUTH' => [CURLAUTH_NTLM, CURLAUTH_BASIC | CURLAUTH_NTLM],
    'SOCKS5_AUTH' => [0, 1, 4, 5, 2, -1],
    'NEW_FILE_PERMS' => [0644, 0777, 01000, -1],
    'ADDRESS_SCOPE' => [0, 4294967295, 4294967296, -1],
    'RTSP_REQUEST' => [0, 1, 11, 12],
    'NETRC' => [0, 2, 3],
    'ALTSVC_CTRL' => [0, 1],
    'SSL_FALSESTART' => [0, 1],
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
    $r = [];
    foreach (['nope', '', 'all', 'ALL', 'http', 'HTTP,https', 'http,,https', 'http,', 'all,http', 'http ', 'rtmp', 'smb', 'file,ftp,sftp'] as $v) {
        $r[] = "'$v'=" . var_export(curl_setopt(curl_init(), $o, $v), true);
    }
    echo $name, ": ", implode(' ', $r), "\n";
}
foreach (['INTERFACE' => ['', 'lo0'], 'SSLENGINE' => ['', 'x'], 'DNS_SERVERS' => ['', '1.1.1.1'], 'TLSAUTH_TYPE' => ['SRP', 'srp', 'x', ''], 'PROXY_TLSAUTH_TYPE' => ['SRP', 'tls']] as $name => $values) {
    $o = constant("CURLOPT_$name");
    foreach ($values as $v) {
        echo $name, " '$v' => ", var_export(curl_setopt(curl_init(), $o, $v), true), "\n";
    }
}
// curl_setopt_array stops at the first refused option
$h = curl_init();
var_dump(curl_setopt_array($h, [CURLOPT_URL => 'http://x.invalid/', CURLOPT_TIMEOUT => -1, CURLOPT_PRIVATE => 'never']));
var_dump(curl_getinfo($h, CURLINFO_PRIVATE), curl_getinfo($h, CURLINFO_EFFECTIVE_URL));

echo "== deprecated close functions\n";
var_dump(@curl_close(curl_init()), @curl_share_close(curl_share_init()));
rmdir($dir);
