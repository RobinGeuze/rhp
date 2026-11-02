<?php
// Socket-stream review fixes: persistent client streams, stream_select's
// non-stream elements, the alias names in errors, stream_set_write_buffer /
// stream_set_blocking answers, Socket objects being strict and
// unserializable, socket_set_option / socket_strerror argument handling.
set_error_handler(function ($no, $msg) {
    if (error_reporting() & $no) { echo "[$no] ", preg_replace('/:\d+\b/', ':PORT', $msg), "\n"; }
    return true;
});
function show(callable $f) {
    try { var_dump($f()); } catch (Throwable $t) { echo get_class($t), ": ", $t->getMessage(), "\n"; }
}

// persistent client streams are reused while alive
$srv = stream_socket_server('tcp://127.0.0.1:0');
$name = stream_socket_get_name($srv, false);
$port = (int) substr(strrchr($name, ':'), 1);
$F = STREAM_CLIENT_CONNECT | STREAM_CLIENT_PERSISTENT;
$p = stream_socket_client("tcp://$name", $e, $s, 1, $F);
var_dump($p, get_resource_type($p));
$p2 = stream_socket_client("tcp://$name", $e, $s, 1, $F);
var_dump($p === $p2);
$pf = pfsockopen('127.0.0.1', $port);
var_dump($pf, $pf === $p);
var_dump(pfsockopen('127.0.0.1', $port) === $pf);
fclose($p);
$p3 = stream_socket_client("tcp://$name", $e, $s, 1, $F);
var_dump($p3, $p3 === $p);
// the peer went away: a fresh connection
$a1 = stream_socket_accept($srv);
$a2 = stream_socket_accept($srv);
$a3 = stream_socket_accept($srv);
fclose($a1); fclose($a2); fclose($a3);
usleep(50000);
$p4 = stream_socket_client("tcp://$name", $e, $s, 1, $F);
var_dump($p4 === $p3);
$a4 = stream_socket_accept($srv);
fwrite($p4, "fresh");
var_dump(fread($a4, 10));
var_dump(stream_get_meta_data($p4)['stream_type']);

// stream_select: non-stream elements are a TypeError after the select ran
[$x, $y] = stream_socket_pair(STREAM_PF_UNIX, STREAM_SOCK_STREAM, 0);
fwrite($y, "r");
$r = ['k' => $x, 'n' => 1]; $w = null; $ex = null;
show(fn () => stream_select($r, $w, $ex, 0));
var_dump($r);
$m = fopen('php://memory', 'r');
fclose($m);
$r = [$m, $x];
show(fn () => stream_select($r, $w, $ex, 0));
var_dump(count($r));
$r = ['nope'];
show(fn () => stream_select($r, $w, $ex, 0));

// aliases report their own names
show(fn () => socket_set_blocking('x', true));
show(fn () => socket_get_status(1));
show(fn () => socket_set_timeout(null, 1));
show(fn () => stream_set_blocking($x, []));
show(fn () => stream_set_write_buffer('x', 0));
show(fn () => set_file_buffer($x, 0));
// write buffering is only a FILE* stdio option
$mem = fopen('php://memory', 'r+');
$tmp = tmpfile();
var_dump(stream_set_write_buffer($x, 0), stream_set_write_buffer($mem, 0), stream_set_write_buffer($tmp, 8192));
var_dump(stream_set_write_buffer(STDERR, 0));
// a memory stream has no blocking mode
var_dump(stream_set_blocking($mem, false), stream_get_meta_data($mem)['blocked']);
var_dump(stream_set_blocking($x, false), stream_get_meta_data($x)['blocked']);
fclose($tmp);

// Socket objects: no dynamic properties, not serializable
$sock = socket_import_stream($x);
show(function () use ($sock) { $sock->prop = 1; });
show(fn () => isset($sock->prop));
show(fn () => serialize($sock));
show(fn () => unserialize('O:6:"Socket":0:{}'));
show(fn () => unserialize('O:10:"CurlHandle":0:{}'));

// socket_set_option / socket_get_option
show(fn () => socket_set_option($sock, SOL_SOCKET, SO_KEEPALIVE, new stdClass));
show(fn () => socket_set_option($sock, SOL_SOCKET, SO_LINGER, ['l_onoff' => -1, 'l_linger' => 0]));
$o = new stdClass; $o->l_onoff = 1; $o->l_linger = 3;
show(fn () => socket_set_option($sock, SOL_SOCKET, SO_LINGER, $o));
show(fn () => socket_get_option($sock, SOL_SOCKET, SO_LINGER));
$t = new stdClass; $t->sec = 2;
show(fn () => socket_set_option($sock, SOL_SOCKET, SO_RCVTIMEO, $t));
show(fn () => socket_strerror(PHP_INT_MAX));
show(fn () => socket_strerror(-2147483649));
show(fn () => socket_strerror(-10001) !== '');
