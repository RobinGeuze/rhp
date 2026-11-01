<?php
// openssl_verify / openssl_pkey_free failure paths. php warns (or raises the
// #[\Deprecated] notice) here; rhp sends those to its diagnostic sink, which
// the engine does not display yet (placeholder #32), so the calls are
// @-silenced and the return values and thrown errors are compared.
$rsa = '-----BEGIN PUBLIC KEY-----
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQC8dJ1adN1iO6N5NkABfiX4aUQe
/4Fwl8ieHHju1OZzTCXeyahCs52FOvFTYOQEl4RbA2iVQx78nXt7iTk+mgZ1+tpr
xBx1QV0LDEcLLd2OaWmhFLAV5dlqnv0sbWyQX6hO4Q+be15nf/bpAo4f2Zuc8x9i
BRhJ8KfkQ/zJ0fiwVQIDAQAB
-----END PUBLIC KEY-----
';
$sig = base64_decode('T4nNTV/m3gKkoAB6TqCQ1G8lI+uDQU/9rIz8nKm26mRirvksHpJ5V4eF0Qt90+vgQRSP0jI52vGQBFAzS4hMZS1EVYIz7+WRV/tHhI8ckGp9Wz/26HgbFMs4em6zGwmway7xSouxOVhVCaa2VTK2E29AUeRk+VVG6ImIfqMKZ7o=');
$ec = '-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEb0juCnk6R2lIWkq0bSWzPBsoImrB
O4bihc90FOdq4LXCZ3weD9YelPP4XpA8W+gswhpe+pBkgCqdPqTf8vN72g==
-----END PUBLIC KEY-----
';
$ecsig = base64_decode('MEYCIQCeDBjGv00skRrACs9fccJ5XkXhh/dGyigCWCO7BzUEaQIhALlW5o/+01Bp2yU3Msp7fOtrrash+K3/Vk9nqIMKZkww');
$pub = openssl_pkey_get_public($rsa);
$ecpub = openssl_pkey_get_public($ec);
var_dump(openssl_verify('hello world', $sig, $pub, 'sha256'), openssl_verify('hello world', $ecsig, $ecpub, 'sha256'));
var_dump(@openssl_verify('hello world', $sig, $pub, 'bogus'));
var_dump(@openssl_verify('hello world', $sig, $pub, 999));
var_dump(@openssl_verify('hello world', $sig, $pub, 'sha256', 999));
var_dump(@openssl_verify('hello world', $sig, $pub, 'sha256', OPENSSL_PKCS1_OAEP_PADDING));
var_dump(@openssl_verify('hello world', $sig, 'bogus key'));
var_dump(@openssl_verify('hello world', $ecsig, $ecpub, 'sha256', OPENSSL_PKCS1_PSS_PADDING));
var_dump(@openssl_verify('hello world', $ecsig, $ec, 'sha256', OPENSSL_PKCS1_PADDING));
var_dump(@openssl_verify('hello world', $sig, null), @openssl_verify('hello world', $sig, 42), @openssl_verify('hello world', $sig, 1.5));
class Pem { function __construct(public string $p) {} function __toString(): string { return $this->p; } }
class BadPem { function __toString(): string { throw new RuntimeException('no pem'); } }
var_dump(openssl_verify('hello world', $sig, new Pem($rsa), 'sha256'), @openssl_verify('hello world', $sig, new Pem('junk')));
foreach ([new stdClass, new BadPem, [new stdClass, '']] as $bad) {
    try { var_dump(openssl_verify('hello world', $sig, $bad)); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}
try { var_dump(openssl_pkey_get_public(new stdClass)); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
var_dump(@openssl_pkey_free($pub), @openssl_free_key($ecpub));
try { @openssl_pkey_free(1); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { @openssl_free_key('x'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
