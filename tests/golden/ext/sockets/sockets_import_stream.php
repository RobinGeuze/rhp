<?php
// The ext/sockets bits stream-socket libraries use: socket_import_stream,
// socket_get_option / socket_set_option, socket_strerror, socket_last_error.
set_error_handler(function ($no, $msg) {
    if (error_reporting() & $no) { echo "[$no] $msg\n"; }
    return true;
});

// a tcp client socket (react/socket's TcpConnector reads SO_ERROR this way
// after an async connect)
$s = stream_socket_server('tcp://127.0.0.1:0');
$c = stream_socket_client('tcp://' . stream_socket_get_name($s, false), $e, $es, 1, STREAM_CLIENT_CONNECT | STREAM_CLIENT_ASYNC_CONNECT);
$r = null; $w = [$c]; $x = null;
var_dump(stream_select($r, $w, $x, 2));
$sock = socket_import_stream($c);
var_dump(get_class($sock));
var_dump(socket_get_option($sock, SOL_SOCKET, SO_ERROR));
var_dump(socket_set_option($sock, SOL_TCP, TCP_NODELAY, 1), socket_get_option($sock, SOL_TCP, TCP_NODELAY) !== 0);
var_dump(socket_strerror(SOCKET_ECONNREFUSED) === socket_strerror(SOCKET_ECONNREFUSED));
fclose($c);
fclose($s);

// options on a live socket
[$a, $b] = stream_socket_pair(STREAM_PF_UNIX, STREAM_SOCK_STREAM, 0);
$sa = socket_import_stream($a);
var_dump(socket_get_option($sa, SOL_SOCKET, SO_TYPE) === SOCK_STREAM);
var_dump(socket_get_option($sa, SOL_SOCKET, SO_ERROR));
var_dump(socket_get_option($sa, SOL_SOCKET, SO_LINGER));
var_dump(socket_set_option($sa, SOL_SOCKET, SO_LINGER, ['l_onoff' => 1, 'l_linger' => 2]));
var_dump(socket_get_option($sa, SOL_SOCKET, SO_LINGER)['l_onoff'] !== 0);
var_dump(socket_set_option($sa, SOL_SOCKET, SO_KEEPALIVE, 1), socket_get_option($sa, SOL_SOCKET, SO_KEEPALIVE) !== 0);
var_dump(socket_set_option($sa, SOL_SOCKET, SO_RCVTIMEO, ['sec' => 3, 'usec' => 0]));
var_dump(socket_get_option($sa, SOL_SOCKET, SO_RCVTIMEO));
var_dump(socket_last_error(), socket_last_error($sa));
var_dump(@socket_get_option($sa, 12345, 999), socket_last_error() > 0, socket_last_error($sa) > 0);
socket_clear_error($sa);
var_dump(socket_last_error($sa), socket_last_error() > 0);
socket_clear_error();
var_dump(socket_last_error());
// the stream still works
fwrite($b, "via stream");
var_dump(fread($a, 100));

// not a socket
$f = fopen('php://memory', 'r');
var_dump(socket_import_stream($f));
try { new Socket(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { socket_get_option($f, 1, 1); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(function_exists('socket_import_stream'), class_exists('Socket'));
var_dump(socket_strerror(0) !== '', SOL_SOCKET === SOL_SOCKET, SO_ERROR > 0);
