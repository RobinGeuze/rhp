<?php
// TCP socket streams: server / client / accept, names, select, blocking and
// non-blocking reads, read timeouts, meta data, shutdown and feof. Ports are
// picked by the kernel, so they are masked in the output.
function port($n) { return is_string($n) ? preg_replace('/:\d+$/', ':PORT', $n) : $n; }

$s = stream_socket_server('tcp://127.0.0.1:0', $errno, $errstr);
var_dump($s, $errno, $errstr, get_resource_type($s));
$name = stream_socket_get_name($s, false);
var_dump(port($name), stream_socket_get_name($s, true));
var_dump(stream_get_meta_data($s));

$c = stream_socket_client('tcp://' . $name, $errno, $errstr, 5);
var_dump($c, $errno, $errstr);
$a = stream_socket_accept($s, 5, $peer);
var_dump($a, port($peer));
var_dump(port(stream_socket_get_name($a, true)) === port(stream_socket_get_name($c, false)));
var_dump(port(stream_socket_get_name($c, true)));

// select: keys are preserved, unready entries dropped
var_dump(fwrite($c, "hello\nworld\n"));
$r = ['srv' => $s, 7 => $a]; $w = null; $e = null;
var_dump(stream_select($r, $w, $e, 1), $r);
var_dump(fgets($a));
// the rest of the data is already buffered: readable without a syscall
$r = [$a]; $w = []; $e = null;
var_dump(stream_select($r, $w, $e, 0), count($r), $w);
var_dump(fgets($a));

// writable
$r = null; $w = [3 => $c]; $e = null;
var_dump(stream_select($r, $w, $e, 0), array_keys($w));

// nothing to read: select times out
$r = [$a]; $w = null;
var_dump(stream_select($r, $w, $e, 0, 1000), $r);

// non-blocking read with nothing there
var_dump(stream_set_blocking($a, false));
var_dump(fread($a, 10), fgets($a), feof($a));
$m = stream_get_meta_data($a);
var_dump($m['blocked'], $m['timed_out'], $m['eof'], $m['stream_type'], $m['mode'], $m['seekable']);

// blocking read with a short timeout: false and timed_out
var_dump(stream_set_blocking($a, true));
var_dump(stream_set_timeout($a, 0, 100000));
var_dump(fread($a, 10));
var_dump(stream_get_meta_data($a)['timed_out']);
fwrite($c, "abc");
var_dump(fread($a, 10), stream_get_meta_data($a)['timed_out']);

// stream_get_contents with a limit on a socket
fwrite($c, "0123456789");
var_dump(stream_get_contents($a, 4), fread($a, 100));

// half-close: the reader sees eof
var_dump(stream_socket_shutdown($c, STREAM_SHUT_WR));
var_dump(fread($a, 10), feof($a), stream_get_meta_data($a)['eof']);
var_dump(fwrite($a, "still writable"));
var_dump(fread($c, 100));

// closing the peer: feof notices without a read
$c2 = stream_socket_client('tcp://' . $name);
$a2 = stream_socket_accept($s);
var_dump(feof($a2));
fclose($c2);
$r = [$a2]; $w = null;
stream_select($r, $w, $e, 1);
var_dump(feof($a2), stream_get_meta_data($a2)['eof']);
fclose($a2);

// fsockopen
$port = (int) substr($name, strrpos($name, ':') + 1);
$f = fsockopen('127.0.0.1', $port, $errno, $errstr, 2);
$af = stream_socket_accept($s);
var_dump($f, $errno, $errstr);
fwrite($af, "pong\n");
var_dump(fgets($f));
fclose($f);
fclose($af);

// async connect
$c3 = stream_socket_client('tcp://' . $name, $errno, $errstr, 1, STREAM_CLIENT_CONNECT | STREAM_CLIENT_ASYNC_CONNECT);
var_dump(is_resource($c3));
$r = null; $w = [$c3];
var_dump(stream_select($r, $w, $e, 2), count($w), port(stream_socket_get_name($c3, true)));
$a3 = stream_socket_accept($s);
fwrite($c3, "async\n");
var_dump(fgets($a3));

fclose($c);
fclose($a);
fclose($c3);
fclose($a3);
fclose($s);
var_dump(is_resource($s), get_resource_type($s));
