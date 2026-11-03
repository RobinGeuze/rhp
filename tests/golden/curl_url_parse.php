<?php
// libcurl 8.23's URL parser as php shows it: CURLINFO_EFFECTIVE_URL before a
// transfer, CURLOPT_PATH_AS_IS, and curl_exec on a URL the parser rejects
// (CURLE_URL_MALFORMAT, no transfer, no effective URL afterwards). No
// network: transfers that parse are stopped by CURLOPT_PROTOCOLS_STR.
function pre(string $u, bool $asis = false): void
{
    $h = curl_init();
    if ($asis) {
        curl_setopt($h, CURLOPT_PATH_AS_IS, true);
    }
    curl_setopt($h, CURLOPT_URL, $u);
    $i = curl_getinfo($h);
    echo json_encode(bin2hex($u)), ' => ';
    var_dump(curl_getinfo($h, CURLINFO_EFFECTIVE_URL), $i['url'] ?? '(no url key)');
}
foreach (['http://x/a/../b', 'http://x:000080/', 'http://x:0000000000080/', 'http://x:00000/', 'http://x:065536/',
          'http://[fe80::1%eth0]/', 'http://[fe80::1%25]/', 'http://[fe80::1%]/', 'http://[fe80::1%2]/',
          'http://[::1%2525x]/', 'http://[::1%25%2541]/', 'http://[%3a%3a1]/', 'http://[::1%25abcdefghijklmnop]/',
          'http://[::1.2.3.4]/', 'http://[::0.0.1.0]/', 'http://[::ffff:0:1]/', 'http://[::ABCD]/',
          'http://[1:0:0:1:0:0:0:1]/', 'http://[::ffff:1.2.3.04]/',
          'http://x../', 'http://x.../', 'http://./', 'http://..x/', 'http://x.y./', 'http://x|y/', 'http://x%7cy/',
          'http://a%2eb/', 'http://x%2e./', 'http://x/%2e%2e/a', 'http://x/a/%2E/b', 'http://x/a/.%2e/b',
          'http://x/%2e%2e%2f/a', "http://x/\x80", "http://x\x80/", "http://x/a\x80b?\x81#\x82", "http://x/\x7f",
          'http://u @x/', 'http://x/?a b'] as $u) {
    pre($u);
}
foreach (['http://x/a/../b', 'http://x', 'http://x/%2e%2e/a', 'http://x/./a/', 'file:///a/../b'] as $u) {
    pre($u, true);
}
function ex($h, string $label): void
{
    curl_setopt($h, CURLOPT_PROTOCOLS_STR, 'file');
    curl_setopt($h, CURLOPT_RETURNTRANSFER, true);
    $r = curl_exec($h);
    $i = curl_getinfo($h);
    echo $label, ': ';
    var_dump($r, curl_errno($h), curl_error($h), curl_getinfo($h, CURLINFO_EFFECTIVE_URL), $i['url'] ?? '(no url key)', count($i));
}
ex(curl_init(), 'unset');
ex(curl_init(''), 'empty');
foreach (['http://a/b c', 'http://', 'http://x:99999/', 'http:x', 'http://x\ty/', 'x y', '//x/', 'data:text/plain,hi',
          'http://x/?a=1#f g', 'http://x/a b?c d', 'http://a@b@c/', 'file://h/x', 'file:/', 'http:////x/',
          'http://[zz]/', 'http://[::1]x/', 'http://x|y/', 'http://u:p w@x/', "http://x/\x01", 'x:/'] as $u) {
    ex(curl_init($u), json_encode($u));
}
foreach (['nope', 'HTTP://Nope:80', 'x/../a', 'mailto:a@b', '12.5', 'FTP.x/y'] as $u) {
    ex(curl_init($u), json_encode($u));
}
$h = curl_init('x/a/../b');
curl_setopt($h, CURLOPT_PATH_AS_IS, true);
ex($h, 'as-is');
