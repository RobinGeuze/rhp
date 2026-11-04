//! The TLS stream transports and the http(s):// wrapper against php. The
//! peer is a TLS server run by php itself with a self-signed certificate
//! generated here; both engines talk to the same server. OpenSSL's error
//! texts and the negotiated protocol depend on the OpenSSL build, so these
//! cannot be goldens. Skips without `php`.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use rhp_difftest::{diff_source, php_available, php_bin};

/// Prints warnings and notices: the harness runs php with error_reporting=0.
const PRELUDE: &str = r#"<?php
set_error_handler(function ($no, $str) { echo "W[$no]: $str\n"; return true; });
"#;

/// A certificate for localhost / 127.0.0.1 (server.pem: cert and key;
/// ca.pem: the certificate alone, for `cafile`).
const GENCERT: &str = r#"<?php
$dir = $argv[1];
$conf = "$dir/openssl.cnf";
file_put_contents($conf, "[req]\ndistinguished_name=dn\n[dn]\n[v3]\nsubjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=CA:TRUE\n");
$key = openssl_pkey_new(['private_key_bits' => 2048, 'private_key_type' => OPENSSL_KEYTYPE_RSA, 'config' => $conf]);
$csr = openssl_csr_new(['commonName' => 'localhost'], $key, ['config' => $conf, 'digest_alg' => 'sha256']);
$cert = openssl_csr_sign($csr, null, $key, 3650, ['config' => $conf, 'x509_extensions' => 'v3', 'digest_alg' => 'sha256'], 1);
openssl_x509_export($cert, $c);
openssl_pkey_export($key, $k, null, ['config' => $conf]);
file_put_contents("$dir/server.pem", $c . $k);
file_put_contents("$dir/ca.pem", $c);
"#;

/// Serves TLS connections until killed. A connection that starts with
/// `ECHO` gets its next line echoed; anything else is an HTTP request.
const SERVER: &str = r#"<?php
$dir = $argv[1];
$ctx = stream_context_create(['ssl' => ['local_cert' => "$dir/server.pem"]]);
$srv = stream_socket_server('ssl://127.0.0.1:0', $errno, $errstr, STREAM_SERVER_BIND | STREAM_SERVER_LISTEN, $ctx);
if (!$srv) { fwrite(STDERR, "listen: $errstr\n"); exit(1); }
$port = explode(':', stream_socket_get_name($srv, false))[1];
echo "$port\n";
fflush(STDOUT);
while (true) {
    $c = @stream_socket_accept($srv, 60);
    if (!$c) { continue; }
    $first = fgets($c);
    if ($first === false) { fclose($c); continue; }
    if (rtrim($first) === 'ECHO') {
        $line = fgets($c);
        fwrite($c, "echo: $line");
        fclose($c);
        continue;
    }
    [$method, $path] = explode(' ', $first) + [1 => '/'];
    $len = 0;
    $head = [];
    while (($h = fgets($c)) !== false && rtrim($h) !== '') {
        $head[] = rtrim($h);
        if (stripos($h, 'Content-Length:') === 0) { $len = (int) substr($h, 15); }
    }
    $body = $len > 0 ? fread($c, $len) : '';
    $base = "https://127.0.0.1:$port";
    switch ($path) {
        case '/r1': $out = "HTTP/1.1 302 Found\r\nLocation: r2\r\nContent-Length: 0\r\n\r\n"; break;
        case '/r2': $out = "HTTP/1.0 301 Moved Permanently\r\nLocation: $base/r3\r\n\r\n"; break;
        case '/r3': $out = "HTTP/1.1 200 OK\r\nX-Hop: 3\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nfinal"; break;
        case '/chunked': $out = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n"; break;
        case '/missing': $out = "HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\n\r\nnope"; break;
        case '/echo':
            $text = "$method " . implode("|", array_filter($head, fn($h) => stripos($h, 'Host:') !== 0)) . " [$body]";
            $out = "HTTP/1.1 200 OK\r\nContent-Length: " . strlen($text) . "\r\n\r\n$text";
            break;
        default: $out = "HTTP/1.1 500 Unknown\r\n\r\n";
    }
    fwrite($c, $out);
    fclose($c);
}
"#;

struct Server {
    child: Child,
    port: u16,
    dir: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn start(name: &str) -> Option<Server> {
    if !php_available() {
        eprintln!("[{name}] no php — skipping");
        return None;
    }
    let has_openssl = Command::new(php_bin())
        .args(["-n", "-r", "exit(extension_loaded('openssl') ? 0 : 1);"])
        .status()
        .is_ok_and(|s| s.success());
    if !has_openssl {
        eprintln!("[{name}] php -n has no openssl — skipping");
        return None;
    }
    let dir = std::env::temp_dir().join(format!("rhp-tls-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let write = |file: &str, body: &str| std::fs::write(dir.join(file), body).expect("write");
    write("gencert.php", GENCERT);
    write("server.php", SERVER);
    let ok = Command::new(php_bin())
        .args(["-n"])
        .arg(dir.join("gencert.php"))
        .arg(&dir)
        .status()
        .is_ok_and(|s| s.success());
    assert!(ok, "[{name}] certificate generation failed");
    let mut child = Command::new(php_bin())
        .args(["-n"])
        .arg(dir.join("server.php"))
        .arg(&dir)
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn php server");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout"))
        .read_line(&mut line)
        .expect("server port");
    let port = line.trim().parse().expect("port");
    Some(Server { child, port, dir })
}

fn check(name: &str, body: &str) {
    let Some(srv) = start(name) else {
        return;
    };
    let src = format!(
        "{PRELUDE}$port = {};\n$ca = '{}';\n{body}",
        srv.port,
        srv.dir.join("ca.pem").display()
    );
    if let Some(why) = diff_source(src.as_bytes()).failure() {
        panic!("[{name}] {why}");
    }
}

#[test]
fn ssl_transport() {
    check(
        "ssl_transport",
        r#"
$ctx = stream_context_create(['ssl' => ['cafile' => $ca]]);
$c = stream_socket_client("ssl://127.0.0.1:$port", $errno, $errstr, 5, STREAM_CLIENT_CONNECT, $ctx);
var_dump(is_resource($c), $errno, $errstr);
$m = stream_get_meta_data($c);
var_dump($m['stream_type'], isset($m['crypto']['protocol']), $m['crypto']['cipher_bits'] > 0);
fwrite($c, "ECHO\nhello\n");
var_dump(fgets($c), fgets($c), feof($c));
fclose($c);

// tls:// with a pinned protocol range
$ctx = stream_context_create(['ssl' => ['cafile' => $ca, 'peer_name' => 'localhost',
    'min_proto_version' => STREAM_CRYPTO_PROTO_TLSv1_2, 'max_proto_version' => STREAM_CRYPTO_PROTO_TLSv1_2]]);
$c = stream_socket_client("tls://127.0.0.1:$port", $errno, $errstr, 5, STREAM_CLIENT_CONNECT, $ctx);
var_dump(stream_get_meta_data($c)['crypto']['protocol']);
fwrite($c, "ECHO\npinned\n");
var_dump(fgets($c));
"#,
    );
}

#[test]
fn verification_failures() {
    check(
        "verification_failures",
        r#"
// no CA for the self-signed certificate
var_dump(stream_socket_client("ssl://127.0.0.1:$port", $errno, $errstr, 5));
var_dump($errno, $errstr);
// the name does not match
$ctx = stream_context_create(['ssl' => ['cafile' => $ca, 'peer_name' => 'wrong.example']]);
var_dump(stream_socket_client("ssl://127.0.0.1:$port", $errno, $errstr, 5, STREAM_CLIENT_CONNECT, $ctx));
// allowed: self-signed without verifying the name
$ctx = stream_context_create(['ssl' => ['allow_self_signed' => true, 'verify_peer' => false, 'verify_peer_name' => false]]);
$c = stream_socket_client("ssl://127.0.0.1:$port", $errno, $errstr, 5, STREAM_CLIENT_CONNECT, $ctx);
var_dump(is_resource($c));
fclose($c);
// a fingerprint that does not match
$ctx = stream_context_create(['ssl' => ['cafile' => $ca, 'peer_fingerprint' => str_repeat('0', 40)]]);
var_dump(stream_socket_client("ssl://127.0.0.1:$port", $errno, $errstr, 5, STREAM_CLIENT_CONNECT, $ctx));
// https:// fails the same way
var_dump(file_get_contents("https://127.0.0.1:$port/r3"));
var_dump(http_get_last_response_headers());
"#,
    );
}

#[test]
fn enable_crypto_and_capture() {
    check(
        "enable_crypto_and_capture",
        r#"
$ctx = stream_context_create(['ssl' => ['cafile' => $ca, 'peer_name' => 'localhost', 'capture_peer_cert' => true]]);
$c = stream_socket_client("tcp://127.0.0.1:$port", $errno, $errstr, 5, STREAM_CLIENT_CONNECT, $ctx);
var_dump(stream_get_meta_data($c)['stream_type'], isset(stream_get_meta_data($c)['crypto']));
var_dump(stream_socket_enable_crypto($c, true, STREAM_CRYPTO_METHOD_TLS_CLIENT));
var_dump(stream_socket_enable_crypto($c, true, STREAM_CRYPTO_METHOD_TLS_CLIENT));
$cert = stream_context_get_options($ctx)['ssl']['peer_certificate'];
var_dump($cert instanceof OpenSSLCertificate, openssl_x509_parse($cert)['subject']['CN']);
fwrite($c, "ECHO\nupgraded\n");
var_dump(fgets($c));
try {
    stream_socket_enable_crypto($c, true);
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
$p = stream_socket_pair(STREAM_PF_UNIX, STREAM_SOCK_STREAM, 0);
var_dump(stream_socket_enable_crypto($p[0], true, STREAM_CRYPTO_METHOD_TLS_CLIENT));
"#,
    );
}

#[test]
fn https_redirect_chain() {
    check(
        "https_redirect_chain",
        r#"
$ctx = stream_context_create(['ssl' => ['cafile' => $ca, 'peer_name' => 'localhost']]);
// $http_response_header lands in the caller's scope. Assigning it first
// keeps php 8.5's compile-time deprecation of a plain read away.
function fetch($url, $ctx) {
    $http_response_header = null;
    $body = file_get_contents($url, false, $ctx);
    var_dump($body, $http_response_header);
}
fetch("https://127.0.0.1:$port/r1", $ctx);
var_dump(http_get_last_response_headers());
http_clear_last_response_headers();
var_dump(http_get_last_response_headers());

// stop at the first hop
$noFollow = stream_context_create(['ssl' => ['cafile' => $ca, 'peer_name' => 'localhost'],
    'http' => ['follow_location' => 0]]);
var_dump(file_get_contents("https://127.0.0.1:$port/r1", false, $noFollow));
var_dump(http_get_last_response_headers());
$limited = stream_context_create(['ssl' => ['cafile' => $ca, 'peer_name' => 'localhost'],
    'http' => ['max_redirects' => 1]]);
var_dump(file_get_contents("https://127.0.0.1:$port/r1", false, $limited));
"#,
    );
}

#[test]
fn https_streams() {
    check(
        "https_streams",
        r#"
$ctx = stream_context_create(['ssl' => ['cafile' => $ca, 'peer_name' => 'localhost']]);
$f = fopen("https://127.0.0.1:$port/chunked", 'rb', false, $ctx);
$m = stream_get_meta_data($f);
unset($m['crypto']);
var_dump($m);
var_dump(fread($f, 3), stream_get_contents($f), feof($f));
fclose($f);

var_dump(file_get_contents("https://127.0.0.1:$port/missing", false, $ctx));
var_dump(http_get_last_response_headers());
$ignore = stream_context_create(['ssl' => ['cafile' => $ca, 'peer_name' => 'localhost'],
    'http' => ['ignore_errors' => true]]);
var_dump(file_get_contents("https://127.0.0.1:$port/missing", false, $ignore));

$post = stream_context_create(['ssl' => ['cafile' => $ca, 'peer_name' => 'localhost'],
    'http' => ['method' => 'POST', 'content' => 'a=1&b=2', 'user_agent' => 'rhp-test',
               'header' => ["X-One: 1", "X-Two: 2"], 'protocol_version' => 1.0]]);
var_dump(file_get_contents("https://127.0.0.1:$port/echo", false, $post));
$f = fopen("https://127.0.0.1:$port/echo", 'w', false, $ctx);
var_dump($f);
"#,
    );
}

#[test]
fn notifications() {
    check(
        "notifications",
        r#"
$events = [];
$ctx = stream_context_create(['ssl' => ['cafile' => $ca, 'peer_name' => 'localhost']], [
    'notification' => function ($code, $severity, $message, $message_code, $sofar, $max) use (&$events) {
        $events[] = "$code/$severity/" . var_export($message, true) . "/$message_code/$sofar/$max";
    },
]);
var_dump(file_get_contents("https://127.0.0.1:$port/r1", false, $ctx));
print_r($events);
$events = [];
var_dump(file_get_contents("https://127.0.0.1:$port/missing", false, $ctx));
print_r($events);
var_dump(array_keys(stream_context_get_params($ctx)));
"#,
    );
}
