<?php
// ext/sodium beyond its phpt: deterministic outputs, argument errors and
// the argument-free traces. No AEGIS or AES-GCM (they depend on the
// libsodium version and CPU).

function show(callable $f) {
    try {
        var_dump($f());
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
        foreach ($e->getTrace() as $fr) {
            echo "  ", strtok($fr['function'], ':'), " args=", json_encode($fr['args'] ?? null), "\n";
        }
    }
}

var_dump(extension_loaded('sodium'), SODIUM_CRYPTO_BOX_KEYPAIRBYTES, SODIUM_CRYPTO_PWHASH_STRPREFIX);

$seed = str_repeat("\x01", SODIUM_CRYPTO_SIGN_SEEDBYTES);
$kp = sodium_crypto_sign_seed_keypair($seed);
$sk = sodium_crypto_sign_secretkey($kp);
$pk = sodium_crypto_sign_publickey($kp);
echo bin2hex($pk), "\n";
$sig = sodium_crypto_sign_detached("hello", $sk);
echo bin2hex($sig), "\n";
var_dump(sodium_crypto_sign_verify_detached($sig, "hello", $pk));
var_dump(sodium_crypto_sign_verify_detached($sig, "hellp", $pk));
echo bin2hex(sodium_crypto_sign_ed25519_pk_to_curve25519($pk)), "\n";

echo bin2hex(sodium_crypto_generichash("abc")), "\n";
echo bin2hex(sodium_crypto_generichash("abc", str_repeat("k", 32), 16)), "\n";
$st = sodium_crypto_generichash_init();
sodium_crypto_generichash_update($st, "a");
sodium_crypto_generichash_update($st, "bc");
echo bin2hex(sodium_crypto_generichash_final($st)), "\n";
var_dump($st);

$key = str_repeat("\x02", SODIUM_CRYPTO_KDF_KEYBYTES);
echo bin2hex(sodium_crypto_kdf_derive_from_key(32, 7, "ctx12345", $key)), "\n";
echo bin2hex(sodium_crypto_shorthash("msg", str_repeat("s", 16))), "\n";
echo bin2hex(sodium_crypto_auth("msg", str_repeat("a", 32))), "\n";

$n = str_repeat("\0", SODIUM_CRYPTO_SECRETBOX_NONCEBYTES);
$k = str_repeat("\3", SODIUM_CRYPTO_SECRETBOX_KEYBYTES);
$c = sodium_crypto_secretbox("secret", $n, $k);
echo bin2hex($c), "\n";
var_dump(sodium_crypto_secretbox_open($c, $n, $k));
var_dump(sodium_crypto_secretbox_open("short", $n, $k));

$an = str_repeat("\0", SODIUM_CRYPTO_AEAD_XCHACHA20POLY1305_IETF_NPUBBYTES);
$ak = str_repeat("\4", SODIUM_CRYPTO_AEAD_XCHACHA20POLY1305_IETF_KEYBYTES);
$ac = sodium_crypto_aead_xchacha20poly1305_ietf_encrypt("m", "ad", $an, $ak);
echo bin2hex($ac), "\n";
var_dump(sodium_crypto_aead_xchacha20poly1305_ietf_decrypt($ac, "ad", $an, $ak));
var_dump(sodium_crypto_aead_xchacha20poly1305_ietf_decrypt($ac, "xx", $an, $ak));

foreach ([SODIUM_BASE64_VARIANT_ORIGINAL, SODIUM_BASE64_VARIANT_ORIGINAL_NO_PADDING,
          SODIUM_BASE64_VARIANT_URLSAFE, SODIUM_BASE64_VARIANT_URLSAFE_NO_PADDING] as $v) {
    $b = sodium_bin2base64("\xff\xfe\x00 hi", $v);
    var_dump($b, sodium_base642bin($b, $v));
}
var_dump(sodium_hex2bin("41:42", ":"), sodium_bin2hex("AB"));
var_dump(bin2hex(sodium_pad("abc", 8)), sodium_unpad(sodium_pad("abc", 8), 8));

$x = "\xff\x00";
sodium_increment($x);
var_dump(bin2hex($x));
$y = "\x01\x02";
sodium_add($y, "\x01\x01");
var_dump(bin2hex($y), sodium_compare("\x01", "\x02"), sodium_memcmp("a", "a"));
$z = "secret";
sodium_memzero($z);
var_dump($z);

show(fn() => sodium_hex2bin("zz"));
show(fn() => sodium_bin2base64("x", 0));
show(fn() => sodium_crypto_box_publickey("short"));
show(fn() => sodium_crypto_generichash("x", "", 3));
show(fn() => sodium_compare("a", "bb"));
show(fn() => sodium_crypto_stream(0, "", ""));
show(fn() => sodium_unpad("abc", 8));
show(fn() => sodium_crypto_kdf_derive_from_key(8, 1, "ctx12345", $key));
show(function () { $s = 5; return sodium_crypto_generichash_update($s, "x"); });
show(fn() => sodium_crypto_secretbox([], "", ""));
show(fn() => array_map(fn($f) => $f['args'] ?? null, (new SodiumException("made"))->getTrace()));
show(fn() => strlen(str_repeat("x", -1)));
