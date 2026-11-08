<?php
// ext/sockets' socket calls beyond the phpt corpus: a socket pair, normal
// and binary reads, select, names, errors and addrinfo on literals. Errno
// texts and the SOCK_* message differ by platform, so none is printed.

var_dump(extension_loaded('sockets'));
var_dump(socket_create_pair(AF_UNIX, SOCK_STREAM, 0, $pair));
[$a, $b] = $pair;
var_dump(socket_write($a, "line one\nline two\n"));
var_dump(socket_read($b, 100, PHP_NORMAL_READ));
var_dump(socket_read($b, 3));
$r = [$b]; $w = null; $e = null;
var_dump(socket_select($r, $w, $e, 0, 1000), count($r));
var_dump(socket_send($b, "pong", 10, 0));
var_dump(socket_recv($a, $buf, 2, 0), $buf);
var_dump(socket_recv($a, $buf, 10, MSG_DONTWAIT), $buf);
socket_set_nonblock($a);
var_dump(@socket_read($a, 10), socket_last_error($a) === SOCKET_EAGAIN);
socket_clear_error($a);
var_dump(socket_last_error($a));
var_dump(socket_shutdown($a, SHUT_WR));
var_dump(socket_read($b, 10));
socket_close($a);
try {
    socket_write($a, "x");
} catch (Error $ex) {
    echo get_class($ex), ": ", $ex->getMessage(), "\n";
}

$s = socket_create(AF_INET, SOCK_STREAM, SOL_TCP);
var_dump(socket_bind($s, "127.0.0.1", 0), socket_listen($s));
socket_getsockname($s, $addr, $port);
var_dump($addr, $port > 0);
$c = socket_create(AF_INET, SOCK_STREAM, SOL_TCP);
var_dump(socket_connect($c, "127.0.0.1", $port));
$in = socket_accept($s);
var_dump($in instanceof Socket);
socket_getpeername($in, $paddr);
var_dump($paddr);
$st = socket_export_stream($in);
var_dump(get_resource_type($st), socket_export_stream($in) === $st);
fwrite($st, "via stream");
var_dump(socket_read($c, 100));
var_dump(@socket_connect($c, "127.0.0.1", 1), socket_last_error($c) !== 0);

foreach ([
    fn() => socket_create(12345, SOCK_STREAM, 0),
    fn() => socket_bind($s, "127.0.0.1", 70000),
    fn() => socket_shutdown($s, 7),
    fn() => socket_connect(socket_create(AF_INET, SOCK_STREAM, 0), "127.0.0.1"),
    fn() => socket_select($n1, $n2, $n3, 0),
    fn() => socket_write($s, "x", -1),
    fn() => socket_addrinfo_lookup("127.0.0.1", null, ["nope" => 1]),
    fn() => socket_addrinfo_lookup("127.0.0.1", null, ["ai_family" => 1234]),
    fn() => new AddressInfo(),
] as $f) {
    try {
        var_dump($f());
    } catch (Throwable $ex) {
        echo get_class($ex), ": ", $ex->getMessage(), "\n";
    }
}

$ai = socket_addrinfo_lookup("127.0.0.1", "80", ["ai_family" => AF_INET, "ai_socktype" => SOCK_STREAM]);
$x = socket_addrinfo_explain($ai[0]);
var_dump(count($ai), $x["ai_addr"], $x["ai_family"] === AF_INET);
$bound = socket_addrinfo_bind(socket_addrinfo_lookup("127.0.0.1", "0", ["ai_family" => AF_INET, "ai_socktype" => SOCK_DGRAM])[0]);
var_dump($bound instanceof Socket);
var_dump(socket_strerror(SOCKET_ECONNREFUSED) !== "");
