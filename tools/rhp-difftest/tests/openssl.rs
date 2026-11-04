//! ext/openssl against php. rhp links the OpenSSL php links, so cipher
//! lists, error-queue text and printed certificates match the php on the
//! same machine, but they differ between OpenSSL releases (the mac's 4.1 and
//! CI's 3.5), so these cases cannot be goldens. Output that is random by
//! design (padding, salts, ECDSA nonces) is only round-tripped. Skips
//! without `php`.

use rhp_difftest::{diff_source, php_available};

fn check(name: &str, body: &str) {
    if !php_available() {
        eprintln!("[{name}] no php — skipping");
        return;
    }
    let src = PRELUDE.to_string() + body;
    if let Some(why) = diff_source(src.as_bytes()).failure() {
        panic!("[{name}] {why}");
    }
}

/// Warnings become output (the harness runs php with error_reporting=0);
/// `errors()` drains openssl_error_string().
const PRELUDE: &str = r#"<?php
set_error_handler(function ($no, $str) { echo "W[$no]: $str\n"; return true; });
function errors() { $n = 0; while (($e = openssl_error_string()) !== false && $n++ < 20) { echo "E: $e\n"; } }
function show($label, $v) { echo $label, ': '; var_dump($v); }
$rsa = "-----BEGIN PRIVATE KEY-----
MIICdQIBADANBgkqhkiG9w0BAQEFAASCAl8wggJbAgEAAoGBAKZ2pZN8JDgh8ZrJ
HpqfXC7/X9goMOGbhcRgJGYjvKz6D+GcQosM96SgWpI8li8h7B0M7kJg+oxsyKj+
8Rvp5VkLAdouole3wFNwxswWclGc/moHnMb5Xyz7zfsSaszvF8G8cEbdbPwnoNAg
DifnhF9onaYsMPxwk3MJdTXTzUDhAgMBAAECgYByU9ymbcWmc9GvIGCPi9HRSX23
ROtFl/4IgIufRv4nw1sy9ZJSVCalf6iU9V2ko2S6Ew50soyB85jhUK+iH0Nvh8W7
8kJ6qWj/+snL05pHSyNdB2WO5dz9aOsTpHctuei/2xM/LByZYb21fRf7QCmBfh0I
Cdo6yYqSUfsZtxyjAQJBANUlhkQG5BLX35Be9uo5ImDVsPjPOJcKt6V/lcpsTWFE
uXt/GsJL8LTjh67jakU8QqWCSQEIDE6q74Tr9beOY7ECQQDH7mGrOSwVroGVrFLh
mlDyt+YmUUKJ9gb6OQ2N3bbkkrPIsNKQzgKXQKsnyL+29/AJrhm8/0pfIdPT5xzq
NewxAkA848VhdZ5+C5FgSuC6tKuvghxWV8xQyc0AyFvgQeQxH1Llhr/8NXmEE2PV
gD4BnV9krL1yP/xUFNzr2gSN9W1hAkAmECc+DuNquU9obGdXFicFHqfNcbkGRPWt
G8GN2cEhISDjO27LLL1niGNNbrW5ItjrLt8tkTjahbkrZZump19xAkBeZD1VTtFN
9uVkuBhngws3RJY4x/RR/+wEAqB/ODXjW/jOqH+2BdkjndBaIYO3i/Go8Cyenp8S
lpDKq9gmzHi9
-----END PRIVATE KEY-----
";
$ec = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg8ZuyE3pGijFDD2n+
OFt5rev6s7+txjsMz/8CRcGJ6vWhRANCAATQyDz8lPl2b/Qe/K3DgiXS6XsW3bzN
IVIUV/J7Fwhxk/SUh8p3qbGz0FxORZkoTZZp0lbk7kiVP430899AHCzc
-----END PRIVATE KEY-----
";
$crt = "-----BEGIN CERTIFICATE-----
MIICwTCCAiqgAwIBAgIJASNFZ4mrze8BMA0GCSqGSIb3DQEBCwUAMDcxCzAJBgNV
BAYTAk5MMRIwEAYDVQQKDAlyaHAgdGVzdHMxFDASBgNVBAMMC3JocC5leGFtcGxl
MCAXDTI2MTEwMzIyNTg1MFoYDzIxMjYxMDEwMjI1ODUwWjA3MQswCQYDVQQGEwJO
TDESMBAGA1UECgwJcmhwIHRlc3RzMRQwEgYDVQQDDAtyaHAuZXhhbXBsZTCBnzAN
BgkqhkiG9w0BAQEFAAOBjQAwgYkCgYEApnalk3wkOCHxmskemp9cLv9f2Cgw4ZuF
xGAkZiO8rPoP4ZxCiwz3pKBakjyWLyHsHQzuQmD6jGzIqP7xG+nlWQsB2i6iV7fA
U3DGzBZyUZz+agecxvlfLPvN+xJqzO8XwbxwRt1s/Ceg0CAOJ+eEX2idpiww/HCT
cwl1NdPNQOECAwEAAaOB0jCBzzBQBgNVHREESTBHggtyaHAuZXhhbXBsZYINKi5y
aHAuZXhhbXBsZYcEfwAAAYENdEByaHAuZXhhbXBsZYYUaHR0cHM6Ly9yaHAuZXhh
bXBsZS8wDwYDVR0TAQH/BAUwAwEB/zALBgNVHQ8EBAMCAoQwHQYDVR0lBBYwFAYI
KwYBBQUHAwEGCCsGAQUFBwMCMB0GA1UdDgQWBBSZezUcS6R7PEfQq5Ig34Wn13jW
JzAfBgNVHSMEGDAWgBSZezUcS6R7PEfQq5Ig34Wn13jWJzANBgkqhkiG9w0BAQsF
AAOBgQBtNdICrS5zmu+enIywcFJqYpOwR5c5gY2HAgW+dcyRerWPOsBnFYB2pHi8
mYAZTdf+7v5aiO41Nct0f5LXkfbwbfHPeJ3XzcGrc5zjybb8V5nnPlFvZ8g74WXE
G+BCAXm2lYO56wMtYtTo83CPBq3hSUU35kG8T64fOxqJP7M2tg==
-----END CERTIFICATE-----
";
"#;

#[test]
fn x509_functions_match_php() {
    check(
        "x509",
        r#"
var_dump(openssl_x509_parse($crt));
var_dump(openssl_x509_parse($crt, false));
$x = openssl_x509_read($crt);
var_dump(get_class($x), openssl_x509_read($x) == $x);
show('fp', [openssl_x509_fingerprint($x), openssl_x509_fingerprint($crt, 'sha256'), bin2hex(openssl_x509_fingerprint($x, 'md5', true)), openssl_x509_fingerprint($x, 'bogus')]);
show('export', [openssl_x509_export($x, $out), $out]);
show('export text', [openssl_x509_export($crt, $out, false), $out]);
show('check key', [openssl_x509_check_private_key($x, $rsa), openssl_x509_check_private_key($x, $ec)]);
show('verify', [openssl_x509_verify($x, openssl_pkey_get_public($x)), openssl_x509_verify($x, $ec)]);
show('purpose', [openssl_x509_checkpurpose($x, X509_PURPOSE_SSL_SERVER), openssl_x509_checkpurpose($x, X509_PURPOSE_ANY, [])]);
errors();
foreach (['garbage', '', "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n", 'file:///nonexistent/x.crt', 42] as $bad) {
    show('read', openssl_x509_read($bad));
    show('parse', openssl_x509_parse($bad));
    errors();
}
try { openssl_x509_read([]); } catch (TypeError $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { openssl_x509_export_to_file($x, "a\0b"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
openssl_x509_free($x);
"#,
    );
}

#[test]
fn keys_match_php() {
    check(
        "pkey",
        r#"
foreach ([$rsa, $ec] as $pem) {
    $k = openssl_pkey_get_private($pem);
    $d = openssl_pkey_get_details($k);
    foreach ($d as $f => $v) { if (is_array($v)) { foreach ($v as $kk => $vv) { $d[$f][$kk] = bin2hex($vv); } } }
    var_dump($d);
    show('export', [openssl_pkey_export($k, $out), $out]);
    $pub = openssl_pkey_get_public($d['key']);
    var_dump(openssl_pkey_get_details($pub)['key'] === $d['key']);
    show('public as private', openssl_pkey_get_private($pub));
    show('private as public', openssl_pkey_get_public($k));
    errors();
}
show('cert pubkey', openssl_pkey_get_details(openssl_pkey_get_public($crt))['bits']);
show('array form', openssl_pkey_get_private([$rsa, '']) !== false);
try { openssl_pkey_get_private([$rsa]); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
show('garbage', [openssl_pkey_get_private('garbage'), openssl_pkey_get_public('garbage')]);
errors();
// a passphrase round trip (the PEM itself has a random salt)
$k = openssl_pkey_get_private($rsa);
openssl_pkey_export($k, $enc, 'secret');
show('encrypted', [str_contains($enc, 'ENCRYPTED'), openssl_pkey_get_private($enc, 'secret') !== false, openssl_pkey_get_private($enc, 'wrong')]);
errors();
foreach (['rsa' => ['n' => "\x01", 'e' => "\x03"], 'ec' => ['curve_name' => 'nope'], 'dsa' => []] as $t => $p) {
    show("new $t", openssl_pkey_new([$t => $p]));
    errors();
}
$d = openssl_pkey_get_details(openssl_pkey_get_private($ec));
$k2 = openssl_pkey_new(['ec' => $d['ec']]);
var_dump(openssl_pkey_get_details($k2)['key'] === $d['key']);
$gen = openssl_pkey_new(['private_key_type' => OPENSSL_KEYTYPE_EC, 'curve_name' => 'prime256v1']);
show('generated', [openssl_pkey_get_details($gen)['bits'], openssl_pkey_get_details($gen)['type']]);
show('too short', openssl_pkey_new(['private_key_bits' => 128]));
show('bad type', openssl_pkey_new(['private_key_type' => 99]));
foreach (['x25519', 'ed25519'] as $c) {
    $k = openssl_pkey_new([$c => []]);
    show($c, array_keys(openssl_pkey_get_details($k)[$c]));
}
show('compare', [$k == $k, $k == $gen]);
try { new OpenSSLAsymmetricKey(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { serialize($k); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
try { clone $k; } catch (Error $e) { echo $e->getMessage(), "\n"; }
openssl_pkey_free($k);
"#,
    );
}

#[test]
fn signatures_and_rsa_match_php() {
    check(
        "sign",
        r#"
$k = openssl_pkey_get_private($rsa);
$pub = openssl_pkey_get_public($crt);
foreach ([OPENSSL_ALGO_SHA1, OPENSSL_ALGO_SHA256, OPENSSL_ALGO_MD5, 'sha512', 'SHA3-256', 0, 99, 'bogus'] as $algo) {
    $ok = openssl_sign('data', $sig, $k, $algo);
    show("sign $algo", [$ok, $ok ? bin2hex($sig) : null, $ok ? openssl_verify('data', $sig, $pub, $algo) : null]);
    errors();
}
openssl_sign('data', $sig, $k, 'sha256', OPENSSL_PKCS1_PSS_PADDING);
show('pss', [openssl_verify('data', $sig, $pub, 'sha256', OPENSSL_PKCS1_PSS_PADDING), openssl_verify('data', $sig, $pub, 'sha256')]);
// The PSS salt is random, so the PKCS#1 check fails with "invalid padding" or
// (first byte 0) "block type is not 01": drop those errors unprinted.
while (openssl_error_string() !== false) {}
show('bad padding', openssl_verify('data', $sig, $pub, 'sha256', 99));
show('bad sig', openssl_verify('data', 'xx', $pub, 'sha256'));
errors();
$e = openssl_pkey_get_private($ec);
openssl_sign('data', $sig, $e, 'sha256');
show('ecdsa', [openssl_verify('data', $sig, openssl_pkey_get_public(openssl_pkey_get_details($e)['key']), 'sha256'), openssl_verify('data', $sig, $e, 'sha256', 6)]);
show('no key', [openssl_sign('data', $sig, 'nope'), openssl_verify('data', 'x', 'nope')]);
errors();
foreach ([OPENSSL_PKCS1_PADDING, OPENSSL_PKCS1_OAEP_PADDING] as $pad) {
    show("pub enc $pad", [openssl_public_encrypt('secret', $c, $pub, $pad), strlen($c), openssl_private_decrypt($c, $p, $k, $pad), $p]);
}
show('oaep sha256', [openssl_public_encrypt('secret', $c, $pub, OPENSSL_PKCS1_OAEP_PADDING, 'sha256'), openssl_private_decrypt($c, $p, $k, OPENSSL_PKCS1_OAEP_PADDING, 'sha256'), $p]);
show('bad digest', openssl_public_encrypt('secret', $c, $pub, OPENSSL_PKCS1_OAEP_PADDING, 'bogus'));
$raw = str_repeat("\x01", 128);
show('no padding', [openssl_private_encrypt($raw, $c, $k, OPENSSL_NO_PADDING), bin2hex($c), openssl_public_decrypt($c, $p, $pub, OPENSSL_NO_PADDING), $p === $raw]);
show('priv enc', [openssl_private_encrypt('secret', $c, $k), bin2hex($c), openssl_public_decrypt($c, $p, $pub), $p]);
show('decrypt junk', [openssl_private_decrypt('junk', $p, $k), openssl_public_decrypt('junk', $p, $pub)]);
errors();
show('seal', [openssl_seal('message', $sealed, $ekeys, [$pub], 'aes-128-cbc', $iv), strlen($iv), count($ekeys)]);
show('open', [openssl_open($sealed, $opened, $ekeys[0], $k, 'aes-128-cbc', $iv), $opened]);
show('open bad iv', openssl_open($sealed, $opened, $ekeys[0], $k, 'aes-128-cbc', 'short'));
try { openssl_seal('m', $s, $e2, [], 'aes-128-cbc', $iv); } catch (ValueError $ex) { echo $ex->getMessage(), "\n"; }
try { openssl_seal('m', $s, $e2, [$pub], 'aes-128-cbc'); } catch (ValueError $ex) { echo $ex->getMessage(), "\n"; }
errors();
"#,
    );
}

#[test]
fn ciphers_and_digests_match_php() {
    check(
        "cipher",
        r#"
$key = 'k3y-0123456789abcdef0123456789ab';
foreach (openssl_get_cipher_methods() as $m) {
    if (preg_match('/(gcm|ccm|ocb|poly1305|siv|wrap|xts|hmac)/', $m)) { continue; }
    $iv = str_repeat("\x07", max(openssl_cipher_iv_length($m), 0));
    $c = openssl_encrypt('The quick brown fox', $m, $key, 0, $iv);
    $p = $c === false ? false : openssl_decrypt($c, $m, $key, 0, $iv);
    echo "$m ", openssl_cipher_iv_length($m), ' ', openssl_cipher_key_length($m), " $c ", var_export($p, true), "\n";
}
errors();
foreach (['aes-128-gcm', 'aes-256-gcm', 'chacha20-poly1305', 'aes-128-ocb', 'aes-128-ccm'] as $m) {
    $tag = null;
    $c = openssl_encrypt('authenticated', $m, $key, OPENSSL_RAW_DATA, '123456789012', $tag, 'aad', 12);
    show($m, [bin2hex((string) $c), bin2hex((string) $tag), openssl_decrypt($c, $m, $key, OPENSSL_RAW_DATA, '123456789012', $tag, 'aad'),
        openssl_decrypt($c, $m, $key, OPENSSL_RAW_DATA, '123456789012', 'wrongtag1234', 'aad')]);
    show("$m no tag", openssl_encrypt('x', $m, $key, 0, '123456789012'));
    show("$m no iv", openssl_decrypt('x', $m, $key, 0, '', 'tag'));
}
show('iv short', openssl_encrypt('x', 'aes-128-cbc', $key, 0, 'abc'));
show('iv long', openssl_encrypt('x', 'aes-128-cbc', $key, 0, str_repeat('i', 20)));
show('no iv', openssl_encrypt('x', 'aes-128-cbc', $key));
show('zero padding', [openssl_encrypt(str_repeat('a', 16), 'aes-128-cbc', $key, OPENSSL_ZERO_PADDING, str_repeat('i', 16)),
    openssl_encrypt('a', 'aes-128-cbc', $key, OPENSSL_ZERO_PADDING, str_repeat('i', 16))]);
show('short key', [openssl_encrypt('x', 'aes-256-cbc', 'k', 0, str_repeat('i', 16)), openssl_encrypt('x', 'aes-256-cbc', 'k', OPENSSL_DONT_ZERO_PAD_KEY, str_repeat('i', 16))]);
show('tag non-aead', openssl_decrypt('x', 'aes-128-cbc', $key, 0, str_repeat('i', 16), 'tag'));
show('bad base64', openssl_decrypt('!!!', 'aes-128-cbc', $key, 0, str_repeat('i', 16)));
show('bad final', openssl_decrypt(base64_encode(str_repeat('x', 16)), 'aes-128-cbc', $key, 0, str_repeat('i', 16)));
show('unknown', [openssl_encrypt('x', 'bogus', $key), openssl_cipher_iv_length('bogus'), openssl_cipher_key_length('bogus')]);
try { openssl_decrypt('x', '', $key); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { openssl_cipher_iv_length(''); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
errors();
var_dump(openssl_get_cipher_methods(true), openssl_get_md_methods(), openssl_get_md_methods(true), openssl_get_curve_names());
foreach (openssl_get_md_methods() as $md) {
    echo $md, ' ', openssl_digest('abc', $md), ' ', strlen(openssl_digest('abc', $md, true)), "\n";
}
show('digest bogus', openssl_digest('abc', 'bogus'));
show('pbkdf2', [bin2hex(openssl_pbkdf2('pw', 'salt', 16, 100)), bin2hex(openssl_pbkdf2('pw', 'salt', 20, 1000, 'sha256')), openssl_pbkdf2('pw', 'salt', 16, 100, 'bogus')]);
try { openssl_pbkdf2('pw', 'salt', 0, 1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
show('random', [strlen(openssl_random_pseudo_bytes(33, $strong)), $strong]);
try { openssl_random_pseudo_bytes(0); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
errors();
"#,
    );
}

#[test]
fn csr_and_pkcs_match_php() {
    check(
        "csr",
        r#"
$k = openssl_pkey_get_private($rsa);
// a configuration of our own: the default openssl.cnf differs per machine
$dir = sys_get_temp_dir() . '/rhp-openssl-' . getmypid();
@mkdir($dir);
file_put_contents("$dir/openssl.cnf", "[ req ]\ndistinguished_name = dn\nattributes = attrs\n[ dn ]\nO_default = Default Org\n[ attrs ]\nunstructuredName = rhp\n[ v3 ]\nbasicConstraints = CA:FALSE\nsubjectAltName = DNS:csr.example\n");
$conf = ['config' => "$dir/openssl.cnf"];
$dn = ['countryName' => 'NL', 'organizationName' => ['A', 'B'], 'commonName' => 'csr.example', 'bogusName' => 'x'];
$csr = openssl_csr_new($dn, $k, $conf + ['digest_alg' => 'sha256'], ['challengePassword' => 'pw', 'nope' => 'x', 3 => 'y']);
var_dump(get_class($csr));
show('export', [openssl_csr_export($csr, $out), $out]);
show('export text', [openssl_csr_export($csr, $out, false), $out]);
var_dump(openssl_csr_get_subject($csr), openssl_csr_get_subject($out, false));
show('pubkey', openssl_pkey_get_details(openssl_csr_get_public_key($csr))['key'] === openssl_pkey_get_details($k)['key']);
$x = openssl_csr_sign($csr, null, $k, 30, $conf + ['digest_alg' => 'sha256', 'x509_extensions' => 'v3'], 1234);
$p = openssl_x509_parse($x);
unset($p['validFrom'], $p['validTo'], $p['validFrom_time_t'], $p['validTo_time_t']);
var_dump($p);
$x2 = openssl_csr_sign($csr, $crt, $k, 1, $conf, 0, 'ABCDEF0123');
show('serial hex', openssl_x509_parse($x2)['serialNumberHex']);
show('wrong ca key', openssl_csr_sign($csr, $crt, $ec, 1, $conf));
show('bad days', openssl_csr_sign($csr, null, $k, -1, $conf));
show('bad csr', [openssl_csr_get_subject('garbage'), openssl_csr_export('garbage', $o)]);
errors();
// PKCS#12 and S/MIME round trips (salts and signing times are random)
show('p12', [openssl_pkcs12_export($crt, $p12, $rsa, 'pw', ['friendly_name' => 'rhp', 'extracerts' => [$crt]]),
    openssl_pkcs12_read($p12, $certs, 'pw'), array_keys($certs), $certs['cert'] === $crt]);
show('p12 wrong pw', openssl_pkcs12_read($p12, $certs, 'wrong'));
show('p12 wrong key', openssl_pkcs12_export($crt, $p12, $ec, 'pw'));
errors();
file_put_contents("$dir/msg.txt", "Subject: test\n\nhello\n");
show('p7 sign', openssl_pkcs7_sign("$dir/msg.txt", "$dir/signed.txt", $crt, $rsa, ['To' => 'a@b'], PKCS7_DETACHED));
show('p7 verify', openssl_pkcs7_verify("$dir/signed.txt", PKCS7_NOVERIFY, "$dir/signers.pem"));
show('signers', file_get_contents("$dir/signers.pem") === $crt);
show('p7 encrypt', openssl_pkcs7_encrypt("$dir/msg.txt", "$dir/enc.txt", $crt, ['To' => 'a@b']));
show('p7 decrypt', [openssl_pkcs7_decrypt("$dir/enc.txt", "$dir/dec.txt", $crt, $rsa), file_get_contents("$dir/dec.txt")]);
show('cms sign', openssl_cms_sign("$dir/msg.txt", "$dir/cms.pem", $crt, $rsa, null, 0, OPENSSL_ENCODING_PEM));
show('cms verify', openssl_cms_verify("$dir/cms.pem", OPENSSL_CMS_NOVERIFY, null, [], null, null, null, null, OPENSSL_ENCODING_PEM));
show('cms read', [openssl_cms_read(file_get_contents("$dir/cms.pem"), $c), $c[0] === $crt]);
show('cms encrypt', openssl_cms_encrypt("$dir/msg.txt", "$dir/cms_enc.txt", $crt, null));
show('cms decrypt', [openssl_cms_decrypt("$dir/cms_enc.txt", "$dir/cms_dec.txt", $crt, $rsa), file_get_contents("$dir/cms_dec.txt")]);
show('missing', [openssl_pkcs7_verify("$dir/none.txt", 0), openssl_cms_verify("$dir/none.txt", 0)]);
errors();
array_map('unlink', glob("$dir/*"));
rmdir($dir);
"#,
    );
}

#[test]
fn spki_and_info_match_php() {
    check(
        "spki",
        r#"
$k = openssl_pkey_get_private($rsa);
$spkac = openssl_spki_new($k, 'challenge', OPENSSL_ALGO_SHA256);
show('spki', [strlen($spkac) > 10, openssl_spki_verify($spkac), openssl_spki_export_challenge($spkac), openssl_spki_export($spkac) === openssl_pkey_get_details($k)['key']]);
show('spki bad', [openssl_spki_verify(''), openssl_spki_verify('garbage'), openssl_spki_export('garbage'), openssl_spki_export_challenge('garbage')]);
errors();
$peer = openssl_pkey_get_public(openssl_pkey_get_details(openssl_pkey_get_private($ec))['key']);
$mine = openssl_pkey_new(['ec' => ['curve_name' => 'prime256v1', 'd' => str_repeat("\x05", 32)]]);
show('derive', bin2hex(openssl_pkey_derive($peer, $mine)));
show('derive len', bin2hex(openssl_pkey_derive($peer, $mine, 8)));
errors();
var_dump(array_keys(openssl_get_cert_locations()));
// not the header version: a bottle built before an openssl@4 update
// reports older headers than rhp's
ob_start(); phpinfo(INFO_MODULES); $info = ob_get_clean();
foreach (explode("\n", $info) as $line) {
    if (preg_match('/^(OpenSSL support|OpenSSL Library Version|Openssl default config|openssl\.)/', $line)) { echo $line, "\n"; }
}
"#,
    );
}
