<?php
// FOLLOWLOCATION to another host drops the user's Authorization / Cookie
// headers, CURLOPT_USERPWD and CURLOPT_COOKIE (libcurl's
// this_is_a_follow / allow_auth_to_host), unless CURLOPT_UNRESTRICTED_AUTH.
$base = '__BASE__';
function req($path, array $opts) {
    global $base;
    $h = curl_init($base . $path);
    curl_setopt_array($h, $opts + [CURLOPT_RETURNTRANSFER => 1, CURLOPT_FOLLOWLOCATION => 1]);
    $body = curl_exec($h);
    echo $body, "\n[redirects=", curl_getinfo($h, CURLINFO_REDIRECT_COUNT), " code=", curl_getinfo($h, CURLINFO_HTTP_CODE), "]\n\n";
}
echo "== custom headers, other host\n";
req('/xhost', [CURLOPT_HTTPHEADER => ['Authorization: Bearer SECRET', 'Cookie: s=1', 'X-Keep: yes']]);
echo "== USERPWD + COOKIE, other host\n";
req('/xhost', [CURLOPT_USERPWD => 'u:p', CURLOPT_COOKIE => 'c=2']);
echo "== USERNAME/PASSWORD, other host\n";
req('/xhost', [CURLOPT_USERNAME => 'user', CURLOPT_PASSWORD => 'pw']);
echo "== same host keeps everything (custom Authorization wins)\n";
req('/same', [CURLOPT_USERPWD => 'u:p', CURLOPT_COOKIE => 'c=2', CURLOPT_HTTPHEADER => ['Authorization: Bearer SECRET', 'Cookie: s=1']]);
echo "== UNRESTRICTED_AUTH\n";
req('/xhost', [CURLOPT_UNRESTRICTED_AUTH => 1, CURLOPT_USERPWD => 'u:p', CURLOPT_COOKIE => 'c=2', CURLOPT_HTTPHEADER => ['Authorization: Bearer SECRET', 'Cookie: s=1']]);
echo "== no follow\n";
req('/xhost', [CURLOPT_FOLLOWLOCATION => 0, CURLOPT_USERPWD => 'u:p']);
