<?php
// curl_setopt under open_basedir: file options are checked, the file
// protocol can't be turned on.
ini_set('open_basedir', __DIR__);
set_error_handler(function ($no, $msg) {
    echo str_replace(__DIR__, 'DIR', $msg), "\n";
    return true;
});
$ch = curl_init();
var_dump(curl_setopt($ch, CURLOPT_COOKIEFILE, ''));
var_dump(curl_setopt($ch, CURLOPT_COOKIEFILE, __DIR__ . '/cookies'));
var_dump(curl_setopt($ch, CURLOPT_COOKIEJAR, '/nonexistent/jar'));
var_dump(curl_setopt($ch, CURLOPT_FOLLOWLOCATION, true));
var_dump(curl_setopt($ch, CURLOPT_PROTOCOLS, CURLPROTO_FILE | CURLPROTO_HTTP));
var_dump(curl_setopt($ch, CURLOPT_REDIR_PROTOCOLS, CURLPROTO_HTTPS));
var_dump(curl_setopt($ch, CURLOPT_PROTOCOLS_STR, 'http,FILE'));
var_dump(curl_setopt($ch, CURLOPT_REDIR_PROTOCOLS_STR, 'ALL'));
var_dump(curl_setopt($ch, CURLOPT_PROTOCOLS_STR, 'https'));
