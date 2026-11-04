<?php
// Socket pairs, unix-domain and udp sockets, transport errors and the
// socket-only stream knobs.
function port($n) { return is_string($n) ? preg_replace('/:\d+$/', ':PORT', $n) : $n; }
// Diagnostics without the (absolute) script path.
set_error_handler(function ($no, $msg) {
    if (error_reporting() & $no) { echo "[$no] ", port($msg), "\n"; }
    return true;
});

// constants
var_dump(STREAM_SERVER_BIND, STREAM_SERVER_LISTEN, STREAM_CLIENT_CONNECT,
    STREAM_CLIENT_ASYNC_CONNECT, STREAM_CLIENT_PERSISTENT, STREAM_SHUT_RD,
    STREAM_SHUT_WR, STREAM_SHUT_RDWR, STREAM_PF_INET, STREAM_PF_UNIX,
    STREAM_SOCK_STREAM, STREAM_SOCK_DGRAM, STREAM_IPPROTO_IP, STREAM_IPPROTO_TCP,
    STREAM_IPPROTO_UDP, STREAM_PEEK, STREAM_OOB, STREAM_CRYPTO_METHOD_TLS_CLIENT,
    STREAM_CRYPTO_METHOD_TLS_SERVER);

// socket pair
$p = stream_socket_pair(STREAM_PF_UNIX, STREAM_SOCK_STREAM, STREAM_IPPROTO_IP);
var_dump($p);
var_dump(fwrite($p[0], "one\ntwo\n"));
var_dump(fgets($p[1]));
var_dump(stream_get_meta_data($p[1]));
var_dump(stream_socket_get_name($p[0], true));
fclose($p[0]);
var_dump(stream_get_contents($p[1]), feof($p[1]));
fclose($p[1]);

// unix-domain server
$path = sys_get_temp_dir() . '/rhp-golden-' . getmypid() . '.sock';
@unlink($path);
$us = stream_socket_server('unix://' . $path, $errno, $errstr);
var_dump(is_resource($us), $errno, $errstr);
var_dump(stream_socket_get_name($us, false) === $path);
var_dump(stream_get_meta_data($us)['stream_type']);
$uc = stream_socket_client('unix://' . $path, $errno, $errstr, 1);
$ua = stream_socket_accept($us, 1);
var_dump(stream_socket_get_name($uc, true) === $path);
fwrite($uc, "over unix\n");
var_dump(fgets($ua));
fwrite($ua, "and back");
var_dump(fread($uc, 100));
// the path is taken
var_dump(@stream_socket_server('unix://' . $path, $errno, $errstr), $errno, $errstr);
fclose($uc);
fclose($ua);
fclose($us);
unlink($path);
var_dump(@stream_socket_client('unix://' . $path, $errno, $errstr, 1), $errno > 0, $errstr !== '');

// udp
$u = stream_socket_server('udp://127.0.0.1:0', $errno, $errstr, STREAM_SERVER_BIND);
var_dump(is_resource($u), stream_get_meta_data($u)['stream_type']);
$un = stream_socket_get_name($u, false);
$c = stream_socket_client('udp://' . $un);
var_dump(stream_get_meta_data($c)['stream_type']);
var_dump(fwrite($c, "datagram"));
var_dump(stream_socket_recvfrom($u, 4, STREAM_PEEK, $from), port($from));
var_dump(stream_socket_recvfrom($u, 100, 0, $from), port($from));
var_dump(stream_socket_sendto($u, "reply", 0, $from));
var_dump(fread($c, 100));
var_dump(@stream_socket_sendto($u, "x", 0, "not an address"));
fclose($c);
fclose($u);

// transport errors
// ($errno is left uninitialized by php for an unknown transport: not shown)
var_dump(stream_socket_client('foo://bar', $errno, $errstr), $errstr);
var_dump(stream_socket_client('tcp://nocolon', $errno, $errstr), $errno, $errstr);
var_dump(stream_socket_client('tcp://[::1', $errno, $errstr), $errno, $errstr);
var_dump(stream_socket_server('bogus://x', $errno, $errstr), $errno, $errstr);
var_dump(stream_socket_server('tcp://127.0.0.1', $errno, $errstr), $errno, $errstr);
$s = stream_socket_server('tcp://127.0.0.1:0');
$taken = stream_socket_get_name($s, false);
var_dump(@stream_socket_server('tcp://' . $taken, $errno, $errstr), $errno, $errstr);
var_dump(@stream_socket_accept($s, 0.05));
// later resources: failed opens consumed ids
$t = fopen('php://memory', 'r');
var_dump($t);

// knobs
var_dump(stream_set_timeout($t, 1), stream_set_timeout($s, 2, 500));
var_dump(stream_set_chunk_size($s, 100), stream_set_chunk_size($s, 5));
var_dump(stream_set_read_buffer($s, 0));
var_dump(stream_socket_enable_crypto($s, false));
var_dump(stream_socket_get_name($t, false), stream_socket_shutdown($t, STREAM_SHUT_RDWR));
try { stream_socket_shutdown($s, 7); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { stream_set_chunk_size($s, 0); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { stream_socket_recvfrom($s, 0); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { stream_socket_client('tcp://127.0.0.1:1', $errno, $errstr, INF); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
fclose($s);
fclose($t);
