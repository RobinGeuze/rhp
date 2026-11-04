//! ext/curl against php. rhp links the libcurl php links, so error texts,
//! the URL parser, TLS options and `curl_version()` match the php on the
//! same machine, but differ between libcurl releases (the mac's 8.23 and
//! CI's 8.18) and TLS libraries: these cannot be goldens. The transfers run
//! against a local HTTP server started here. Skips without `php`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

use rhp_difftest::{diff_source, php_available};

fn check(name: &str, src: &str) {
    if !php_available() {
        eprintln!("[{name}] no php — skipping");
        return;
    }
    if let Some(why) = diff_source(src.as_bytes()).failure() {
        panic!("[{name}] {why}");
    }
}

/// Prints warnings and notices: the harness runs php with error_reporting=0.
const PRELUDE: &str = r#"<?php
set_error_handler(function ($no, $str) { echo "W[$no]: $str\n"; return true; });
"#;

/// One keep-alive connection: answers requests until the client closes it.
fn serve_conn(stream: TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut out = stream;
    loop {
        let mut head: Vec<String> = Vec::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let line = line.trim_end_matches(['\r', '\n']).to_string();
            if line.is_empty() {
                break;
            }
            head.push(line);
        }
        let Some(request) = head.first().cloned() else {
            return;
        };
        let header = |name: &str| {
            head.iter()
                .skip(1)
                .filter_map(|l| l.split_once(':'))
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.trim().to_string())
        };
        let len: usize = header("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut body = vec![0; len];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        let mut parts = request.split(' ');
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("").to_string();
        let mut extra: Vec<String> = Vec::new();
        let (status, payload): (&str, Vec<u8>) = match path.as_str() {
            "/cookie" => {
                extra.push("Set-Cookie: session=abc123; Path=/".into());
                ("200 OK", b"cookie set".to_vec())
            }
            "/redirect" => {
                extra.push("Location: /hello".into());
                ("302 Found", b"moved".to_vec())
            }
            "/missing" => ("404 Not Found", b"no such page".to_vec()),
            p if p.starts_with("/slow/") => {
                let ms: u64 = p[6..].parse().unwrap_or(0);
                std::thread::sleep(std::time::Duration::from_millis(ms));
                ("200 OK", format!("slept {ms}").into_bytes())
            }
            "/echo" => {
                // The multipart boundary is random: show it as BOUNDARY.
                let mut b = String::from_utf8_lossy(&body).into_owned();
                if let Some(ct) = header("content-type") {
                    if let Some((_, bound)) = ct.split_once("boundary=") {
                        b = b.replace(bound, "BOUNDARY");
                    }
                }
                let text = format!(
                    "{method} {path}\ncookie: {}\ncontent-type: {}\nlength: {len}\n{b}",
                    header("cookie").unwrap_or_default(),
                    header("content-type")
                        .map(|c| c.split("boundary=").next().unwrap_or("").to_string())
                        .unwrap_or_default(),
                );
                ("200 OK", text.into_bytes())
            }
            _ => ("200 OK", format!("hello from {path}").into_bytes()),
        };
        let mut resp = format!("HTTP/1.1 {status}\r\nContent-Type: text/plain\r\n");
        for e in &extra {
            resp.push_str(e);
            resp.push_str("\r\n");
        }
        resp.push_str(&format!("Content-Length: {}\r\n\r\n", payload.len()));
        let mut bytes = resp.into_bytes();
        bytes.extend_from_slice(&payload);
        if out.write_all(&bytes).is_err() {
            return;
        }
    }
}

/// A local HTTP/1.1 server; returns its base URL.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            std::thread::spawn(move || serve_conn(s));
        }
    });
    format!("http://127.0.0.1:{port}")
}

#[test]
fn strerror_matches_php() {
    check(
        "strerror",
        r#"<?php
for ($c = -1; $c <= 110; $c++) { echo $c, ': ', var_export(curl_strerror($c), true), "\n"; }
for ($c = -2; $c <= 14; $c++) { echo 'm', $c, ': ', var_export(curl_multi_strerror($c), true), "\n"; }
for ($c = -1; $c <= 8; $c++) { echo 's', $c, ': ', var_export(curl_share_strerror($c), true), "\n"; }
"#,
    );
}

#[test]
fn version_and_phpinfo_match_php() {
    check(
        "version",
        r#"<?php
var_dump(curl_version());
ob_start();
phpinfo(INFO_MODULES);
$info = ob_get_clean();
$start = strpos($info, "\ncurl\n");
$end = strpos($info, "\ndate\n", $start);
echo substr($info, $start, $end - $start);
"#,
    );
}

#[test]
fn url_parser_matches_php() {
    check("url_parse", include_str!("curl/url_parse.php"));
}

#[test]
fn tls_srp_options_match_php() {
    check("tlsauth_srp", include_str!("curl/tlsauth_srp.php"));
}

#[test]
fn build_dependent_options_match_php() {
    check("setopt_build", include_str!("curl/setopt_build.php"));
}

#[test]
fn failures_match_php() {
    check(
        "failures",
        r#"<?php
foreach (['http://nonexistent-host-xyz.invalid/', 'http://127.0.0.1:1/', 'htp://example.com/', 'http://[::1/',
          'file:///nonexistent/rhp-curl', 'http://x:99999/', ''] as $u) {
    $h = curl_init($u);
    curl_setopt($h, CURLOPT_RETURNTRANSFER, true);
    curl_setopt($h, CURLOPT_CONNECTTIMEOUT, 5);
    var_dump(curl_exec($h), curl_errno($h), preg_replace('/after \d+ ms/', 'after N ms', curl_error($h)));
    $i = curl_getinfo($h);
    var_dump($i['url'] ?? '(no url)', $i['http_code'], $i['effective_method'] ?? null);
}
"#,
    );
}

/// What Composer's CurlDownloader does: one multi handle, a share for DNS,
/// cookies and TLS sessions, handles added and removed as they finish.
#[test]
fn multi_and_share_match_php() {
    let base = serve();
    let src = PRELUDE.to_string()
        + &r#"
$base = '__BASE__';
$sh = curl_share_init();
foreach ([CURL_LOCK_DATA_DNS, CURL_LOCK_DATA_COOKIE, CURL_LOCK_DATA_SSL_SESSION] as $d) {
    var_dump(curl_share_setopt($sh, CURLSHOPT_SHARE, $d));
}
$mh = curl_multi_init();
var_dump(curl_multi_setopt($mh, CURLMOPT_PIPELINING, CURLPIPE_MULTIPLEX));
var_dump(curl_multi_setopt($mh, CURLMOPT_MAX_HOST_CONNECTIONS, 8));
$headers = [];
function job($mh, $sh, $url, &$headers) {
    $h = curl_init();
    curl_setopt_array($h, [
        CURLOPT_URL => $url,
        CURLOPT_RETURNTRANSFER => true,
        CURLOPT_FOLLOWLOCATION => true,
        CURLOPT_SHARE => $sh,
        CURLOPT_COOKIEFILE => '',
        CURLOPT_ENCODING => '',
        CURLOPT_PRIVATE => $url,
        CURLOPT_HEADERFUNCTION => function ($ch, $line) use (&$headers) {
            $headers[curl_getinfo($ch, CURLINFO_PRIVATE)][] = rtrim($line);
            return strlen($line);
        },
    ]);
    var_dump(curl_multi_add_handle($mh, $h));
    return $h;
}
function run($mh, $base, &$headers) {
    $done = [];
    do {
        $st = curl_multi_exec($mh, $running);
        if ($st !== CURLM_OK) { echo "exec: $st\n"; break; }
        if ($running && curl_multi_select($mh, 1.0) === -1) { usleep(1000); }
        while ($m = curl_multi_info_read($mh)) {
            $h = $m['handle'];
            $i = curl_getinfo($h);
            $done[str_replace($base, 'BASE', $i['url'])] = [
                'msg' => $m['msg'], 'result' => $m['result'], 'errno' => curl_errno($h),
                'code' => $i['http_code'], 'redirects' => $i['redirect_count'],
                'type' => $i['content_type'], 'body' => curl_multi_getcontent($h),
                'headers' => preg_replace('/^Date: .*/', 'Date: X', $headers[curl_getinfo($h, CURLINFO_PRIVATE)] ?? []),
            ];
            var_dump(curl_multi_remove_handle($mh, $h));
        }
    } while ($running);
    ksort($done);
    var_dump($done);
}
foreach (['/cookie', '/slow/50', '/redirect', '/missing', '/slow/10'] as $p) {
    job($mh, $sh, $base . $p, $headers);
}
run($mh, $base, $headers);
// The cookie the first batch received is shared with every handle.
foreach (['/echo', '/hello'] as $p) {
    job($mh, $sh, $base . $p, $headers);
}
run($mh, $base, $headers);
var_dump(curl_multi_info_read($mh), curl_multi_get_handles($mh), curl_multi_errno($mh));

// A persistent share for DNS and connections.
$ps = curl_share_init_persistent([CURL_LOCK_DATA_CONNECT, CURL_LOCK_DATA_DNS]);
var_dump($ps->options, $ps === curl_share_init_persistent([CURL_LOCK_DATA_DNS, CURL_LOCK_DATA_CONNECT]));
foreach (['/a', '/b'] as $p) {
    $h = curl_init($base . $p);
    curl_setopt($h, CURLOPT_SHARE, $ps);
    curl_setopt($h, CURLOPT_RETURNTRANSFER, true);
    var_dump(curl_exec($h), curl_getinfo($h, CURLINFO_RESPONSE_CODE));
}

// An exception in a callback ends curl_multi_exec with it.
$h = curl_init($base . '/hello');
curl_setopt($h, CURLOPT_WRITEFUNCTION, function () { throw new RuntimeException('stop'); });
curl_multi_add_handle($mh, $h);
try {
    do { curl_multi_exec($mh, $running); curl_multi_select($mh, 0.2); } while ($running);
    echo "no exception\n";
} catch (RuntimeException $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
while ($m = curl_multi_info_read($mh)) { var_dump($m['result']); }

// Uploads through mime.
$f = tempnam(sys_get_temp_dir(), 'rhpcurl');
file_put_contents($f, "file contents\n");
$h = curl_init($base . '/echo');
curl_setopt($h, CURLOPT_RETURNTRANSFER, true);
curl_setopt($h, CURLOPT_POSTFIELDS, [
    'name' => 'value',
    'list' => ['a', 'b'],
    'upload' => new CURLFile($f, 'text/plain', 'up.txt'),
    'str' => new CURLStringFile('string data', 'data.bin'),
]);
echo curl_exec($h), "\n";
$c = curl_copy_handle($h);
echo curl_exec($c), "\n";
unlink($f);
echo curl_exec($h), "\n";
var_dump(curl_errno($h), curl_error($h));
"#
        .replace("__BASE__", &base);
    check("multi_share", &src);
}

/// Callbacks: read, progress, debug, header-out, and handles reused.
#[test]
fn callbacks_match_php() {
    let base = serve();
    let src = PRELUDE.to_string()
        + &r#"
$base = '__BASE__';
$h = curl_init($base . '/echo');
$chunks = ['first,', 'second,', 'third'];
curl_setopt_array($h, [
    CURLOPT_RETURNTRANSFER => true,
    CURLOPT_UPLOAD => true,
    CURLOPT_CUSTOMREQUEST => 'POST',
    CURLOPT_INFILESIZE => strlen(implode('', $chunks)),
    CURLOPT_READFUNCTION => function ($ch, $fp, $len) use (&$chunks) { return array_shift($chunks) ?? ''; },
    CURLOPT_NOPROGRESS => false,
    CURLOPT_XFERINFOFUNCTION => function ($ch, $dt, $dn, $ut, $un) { return 0; },
    CURLINFO_HEADER_OUT => true,
]);
echo curl_exec($h), "\n";
var_dump(curl_errno($h));
echo preg_replace('/127\.0\.0\.1:\d+/', 'HOST', curl_getinfo($h, CURLINFO_HEADER_OUT));
$h = curl_init($base . '/hello');
$types = [];
curl_setopt($h, CURLOPT_DEBUGFUNCTION, function ($ch, $type, $data) use (&$types) { $types[$type] = true; });
curl_setopt($h, CURLOPT_VERBOSE, true);
curl_setopt($h, CURLOPT_RETURNTRANSFER, true);
var_dump(curl_exec($h));
ksort($types);
var_dump(array_keys($types));
$fp = fopen('php://memory', 'w+');
$h = curl_init($base . '/hello');
curl_setopt($h, CURLOPT_FILE, $fp);
curl_setopt($h, CURLOPT_WRITEHEADER, $fp);
var_dump(curl_exec($h));
rewind($fp);
echo preg_replace('/Date: .*/', 'Date: X', stream_get_contents($fp)), "\n";
curl_setopt($h, CURLOPT_URL, $base . '/second');
curl_setopt($h, CURLOPT_RETURNTRANSFER, true);
var_dump(curl_exec($h), curl_getinfo($h, CURLINFO_EFFECTIVE_URL) === $base . '/second');
curl_reset($h);
curl_setopt($h, CURLOPT_URL, $base . '/third');
var_dump(curl_exec($h));
"#
        .replace("__BASE__", &base);
    check("callbacks", &src);
}
