<?php
// Edges of the socket-stream and ext/sockets slice: crypto on non-tcp
// streams, Socket objects (uncomparable, uncloneable, bound to the
// descriptor they imported), sendto address parsing, pcntl_sigprocmask's
// signal coercion.
set_error_handler(function ($no, $msg) {
    if (error_reporting() & $no) { echo "[$no] $msg\n"; }
    return true;
});

// stream_socket_enable_crypto: a stream without the crypto API warns; a
// disable then reads as success, an enable as failure.
$f = fopen('php://memory', 'r+');
var_dump(stream_socket_enable_crypto($f, false));
var_dump(stream_socket_enable_crypto($f, true, STREAM_CRYPTO_METHOD_TLS_CLIENT));
try { stream_socket_enable_crypto($f, true); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { stream_socket_enable_crypto($f, []); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { stream_socket_enable_crypto($f, false, null, 5); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
[$p0, $p1] = stream_socket_pair(STREAM_PF_UNIX, STREAM_SOCK_STREAM, 0);
var_dump(stream_socket_enable_crypto($p0, false));
// a plain tcp connection: disabling crypto that was never on fails quietly
$s = stream_socket_server('tcp://127.0.0.1:0');
$c = stream_socket_client('tcp://' . stream_socket_get_name($s, false));
var_dump(stream_socket_enable_crypto($c, false));
fclose($c);
fclose($s);

// Socket objects
$a = socket_import_stream($p0);
$b = socket_import_stream($p1);
var_dump($a == $b, $a == $a, $a < $b);
try { $x = clone $a; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { socket_last_error(new stdClass); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { socket_get_option(null, SOL_SOCKET, SO_TYPE); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
fclose($p0);
fclose($p1);

// stream_socket_sendto target parsing
$u = stream_socket_server('udp://127.0.0.1:0', $e, $es, STREAM_SERVER_BIND);
$port = (int) substr(strrchr(stream_socket_get_name($u, false), ':'), 1);
$uc = stream_socket_server('udp://127.0.0.1:0', $e, $es, STREAM_SERVER_BIND);
var_dump(stream_socket_sendto($uc, "one", 0, "127.0.0.1:$port"));
var_dump(stream_socket_recvfrom($u, 10));
var_dump(stream_socket_sendto($uc, "x", 0, "garbage"));
var_dump(stream_socket_sendto($uc, "x", 0, "[::1"));
var_dump(stream_socket_sendto($uc, "x", 0, "127.0.0.1\0zzz:$port"));
var_dump(stream_socket_recvfrom($u, 10));
fclose($u);
fclose($uc);

// pcntl_sigprocmask coerces signal numbers like an int parameter
var_dump(pcntl_sigprocmask(SIG_BLOCK, [10.0, "12", true], $old));
try { pcntl_sigprocmask(SIG_BLOCK, ["x"]); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { pcntl_sigprocmask(SIG_BLOCK, [[]]); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(pcntl_sigprocmask(SIG_BLOCK, [10.5], $old));
var_dump(pcntl_sigprocmask(SIG_BLOCK, ["10 "], $old));
var_dump(pcntl_sigprocmask(SIG_SETMASK, [], $old));

// a Socket holds its stream: freeing the Socket frees the stream with it
[$a, $b] = stream_socket_pair(STREAM_PF_UNIX, STREAM_SOCK_STREAM, 0);
$s = socket_import_stream($a);
unset($a);
var_dump(feof($b));
unset($s);
var_dump(fread($b, 10), feof($b));
