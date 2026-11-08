<?php
// rhp -S (php's built-in web server) with a router: requests over raw
// sockets, then the server's log with times and ports masked.
$doc = __DIR__ . '/doc';
$log = sys_get_temp_dir() . '/rhp-cli-server-' . getmypid() . '.log';
$proc = proc_open([PHP_BINARY, '-n', '-t', $doc, '-S', '127.0.0.1:0', 'router.php'],
    [0 => ['pipe', 'r'], 1 => ['file', $log, 'w'], 2 => ['redirect', 1]], $pipes, $doc);
$addr = null;
for ($i = 0; $i < 100 && $addr === null; $i++) {
    usleep(50000);
    if (preg_match('/Development Server \(http:\/\/(\S+)\) started/', file_get_contents($log), $m)) {
        $addr = $m[1];
    }
}
function mask(string $s): string {
    $s = preg_replace('/^Date: .*\r\n/m', "Date: X\r\n", $s);
    $s = preg_replace('/127\.0\.0\.1:\d+/', '127.0.0.1:P', $s);
    // The 501 and 400 pages' reason is whatever errno held.
    $s = preg_replace('/(NOTIMPLEMENTED \/|%00\S*) - .*/', '$1 - E', $s);
    return preg_replace('/^\[[^\]]+\]/m', '[T]', $s);
}
function req(string $addr, string $request): void {
    $fp = stream_socket_client("tcp://$addr");
    fwrite($fp, $request);
    echo mask(stream_get_contents($fp)), "\n-----\n";
    fclose($fp);
}
$h = "Host: 127.0.0.1\r\n";
req($addr, "GET /hello?x=1 HTTP/1.1\r\n{$h}\r\n");
req($addr, "GET /sub/s.php/more?a=b HTTP/1.1\r\n{$h}X-A.B: 1\r\nX-A.B: 2\r\n\r\n");
req($addr, "GET /a.css HTTP/1.0\r\n\r\n");
req($addr, "GET /gone.txt HTTP/1.1\r\n{$h}\r\n");
req($addr, "GET /sub/n.php HTTP/1.1\r\n{$h}\r\n");
req($addr, "GET /sub/p.php HTTP/1.1\r\n{$h}\r\n");
req($addr, "GET /sub/f.php HTTP/1.1\r\n{$h}\r\n");
// A NUL ends the path stat() sees: 400, not the file or a fallback.
req($addr, "GET /sub/s.php%00.txt HTTP/1.1\r\n{$h}\r\n");
req($addr, "SMURF / HTTP/1.1\r\n{$h}\r\n");
req($addr, "GET / XTTP/1.1\r\n\r\n");
usleep(200000);
proc_terminate($proc);
proc_close($proc);
echo mask(file_get_contents($log));
unlink($log);
