/* ext/sodium's constants and state sizes, evaluated against the sodium.h rhp
 * links, under the conditions php's libsodium.c and libsodium_arginfo.h
 * test (register_libsodium_symbols, in its order). */

#include <stddef.h>
#include <sodium.h>

#if (defined(__amd64) || defined(__amd64__) || defined(__x86_64__) || defined(__i386__) || \
	 defined(_M_AMD64) || defined(_M_IX86) || defined(__aarch64__) || defined(_M_ARM64))
# define HAVE_AESGCM 1
#endif

#ifndef crypto_kdf_BYTES_MIN
# define crypto_kdf_BYTES_MIN 16
# define crypto_kdf_BYTES_MAX 64
# define crypto_kdf_CONTEXTBYTES 8
# define crypto_kdf_KEYBYTES 32
#endif
#ifndef crypto_kx_SEEDBYTES
# define crypto_kx_SEEDBYTES 32
# define crypto_kx_SESSIONKEYBYTES 32
# define crypto_kx_PUBLICKEYBYTES 32
# define crypto_kx_SECRETKEYBYTES 32
#endif
#ifndef crypto_aead_chacha20poly1305_IETF_KEYBYTES
# define crypto_aead_chacha20poly1305_IETF_KEYBYTES crypto_aead_chacha20poly1305_KEYBYTES
#endif
#ifndef crypto_aead_chacha20poly1305_IETF_NSECBYTES
# define crypto_aead_chacha20poly1305_IETF_NSECBYTES crypto_aead_chacha20poly1305_NSECBYTES
#endif
#ifndef crypto_aead_chacha20poly1305_IETF_ABYTES
# define crypto_aead_chacha20poly1305_IETF_ABYTES crypto_aead_chacha20poly1305_ABYTES
#endif
#if defined(crypto_secretstream_xchacha20poly1305_ABYTES) && SODIUM_LIBRARY_VERSION_MAJOR < 10
# undef crypto_secretstream_xchacha20poly1305_ABYTES
#endif

struct rhp_sodium_const {
	const char *name;
	long long value;
};

#define L(n, v) { n, (long long) (v) },

const struct rhp_sodium_const rhp_sodium_consts[] = {
#ifdef HAVE_AESGCM
	L("SODIUM_CRYPTO_AEAD_AES256GCM_KEYBYTES", crypto_aead_aes256gcm_KEYBYTES)
	L("SODIUM_CRYPTO_AEAD_AES256GCM_NSECBYTES", crypto_aead_aes256gcm_NSECBYTES)
	L("SODIUM_CRYPTO_AEAD_AES256GCM_NPUBBYTES", crypto_aead_aes256gcm_NPUBBYTES)
	L("SODIUM_CRYPTO_AEAD_AES256GCM_ABYTES", crypto_aead_aes256gcm_ABYTES)
#endif
#ifdef crypto_aead_aegis128l_KEYBYTES
	L("SODIUM_CRYPTO_AEAD_AEGIS128L_KEYBYTES", crypto_aead_aegis128l_KEYBYTES)
	L("SODIUM_CRYPTO_AEAD_AEGIS128L_NSECBYTES", crypto_aead_aegis128l_NSECBYTES)
	L("SODIUM_CRYPTO_AEAD_AEGIS128L_NPUBBYTES", crypto_aead_aegis128l_NPUBBYTES)
	L("SODIUM_CRYPTO_AEAD_AEGIS128L_ABYTES", crypto_aead_aegis128l_ABYTES)
#endif
#ifdef crypto_aead_aegis256_KEYBYTES
	L("SODIUM_CRYPTO_AEAD_AEGIS256_KEYBYTES", crypto_aead_aegis256_KEYBYTES)
	L("SODIUM_CRYPTO_AEAD_AEGIS256_NSECBYTES", crypto_aead_aegis256_NSECBYTES)
	L("SODIUM_CRYPTO_AEAD_AEGIS256_NPUBBYTES", crypto_aead_aegis256_NPUBBYTES)
	L("SODIUM_CRYPTO_AEAD_AEGIS256_ABYTES", crypto_aead_aegis256_ABYTES)
#endif
	L("SODIUM_CRYPTO_AEAD_CHACHA20POLY1305_KEYBYTES", crypto_aead_chacha20poly1305_KEYBYTES)
	L("SODIUM_CRYPTO_AEAD_CHACHA20POLY1305_NSECBYTES", crypto_aead_chacha20poly1305_NSECBYTES)
	L("SODIUM_CRYPTO_AEAD_CHACHA20POLY1305_NPUBBYTES", crypto_aead_chacha20poly1305_NPUBBYTES)
	L("SODIUM_CRYPTO_AEAD_CHACHA20POLY1305_ABYTES", crypto_aead_chacha20poly1305_ABYTES)
	L("SODIUM_CRYPTO_AEAD_CHACHA20POLY1305_IETF_KEYBYTES", crypto_aead_chacha20poly1305_IETF_KEYBYTES)
	L("SODIUM_CRYPTO_AEAD_CHACHA20POLY1305_IETF_NSECBYTES", crypto_aead_chacha20poly1305_IETF_NSECBYTES)
	L("SODIUM_CRYPTO_AEAD_CHACHA20POLY1305_IETF_NPUBBYTES", crypto_aead_chacha20poly1305_IETF_NPUBBYTES)
	L("SODIUM_CRYPTO_AEAD_CHACHA20POLY1305_IETF_ABYTES", crypto_aead_chacha20poly1305_IETF_ABYTES)
#ifdef crypto_aead_xchacha20poly1305_IETF_NPUBBYTES
	L("SODIUM_CRYPTO_AEAD_XCHACHA20POLY1305_IETF_KEYBYTES", crypto_aead_xchacha20poly1305_IETF_KEYBYTES)
	L("SODIUM_CRYPTO_AEAD_XCHACHA20POLY1305_IETF_NSECBYTES", crypto_aead_xchacha20poly1305_IETF_NSECBYTES)
	L("SODIUM_CRYPTO_AEAD_XCHACHA20POLY1305_IETF_NPUBBYTES", crypto_aead_xchacha20poly1305_IETF_NPUBBYTES)
	L("SODIUM_CRYPTO_AEAD_XCHACHA20POLY1305_IETF_ABYTES", crypto_aead_xchacha20poly1305_IETF_ABYTES)
#endif
	L("SODIUM_CRYPTO_AUTH_BYTES", crypto_auth_BYTES)
	L("SODIUM_CRYPTO_AUTH_KEYBYTES", crypto_auth_KEYBYTES)
	L("SODIUM_CRYPTO_BOX_SEALBYTES", crypto_box_SEALBYTES)
	L("SODIUM_CRYPTO_BOX_SECRETKEYBYTES", crypto_box_SECRETKEYBYTES)
	L("SODIUM_CRYPTO_BOX_PUBLICKEYBYTES", crypto_box_PUBLICKEYBYTES)
	L("SODIUM_CRYPTO_BOX_KEYPAIRBYTES", crypto_box_SECRETKEYBYTES + crypto_box_PUBLICKEYBYTES)
	L("SODIUM_CRYPTO_BOX_MACBYTES", crypto_box_MACBYTES)
	L("SODIUM_CRYPTO_BOX_NONCEBYTES", crypto_box_NONCEBYTES)
	L("SODIUM_CRYPTO_BOX_SEEDBYTES", crypto_box_SEEDBYTES)
	L("SODIUM_CRYPTO_KDF_BYTES_MIN", crypto_kdf_BYTES_MIN)
	L("SODIUM_CRYPTO_KDF_BYTES_MAX", crypto_kdf_BYTES_MAX)
	L("SODIUM_CRYPTO_KDF_CONTEXTBYTES", crypto_kdf_CONTEXTBYTES)
	L("SODIUM_CRYPTO_KDF_KEYBYTES", crypto_kdf_KEYBYTES)
	L("SODIUM_CRYPTO_KX_SEEDBYTES", crypto_kx_SEEDBYTES)
	L("SODIUM_CRYPTO_KX_SESSIONKEYBYTES", crypto_kx_SESSIONKEYBYTES)
	L("SODIUM_CRYPTO_KX_PUBLICKEYBYTES", crypto_kx_PUBLICKEYBYTES)
	L("SODIUM_CRYPTO_KX_SECRETKEYBYTES", crypto_kx_SECRETKEYBYTES)
	L("SODIUM_CRYPTO_KX_KEYPAIRBYTES", crypto_kx_SECRETKEYBYTES + crypto_kx_PUBLICKEYBYTES)
#ifdef crypto_secretstream_xchacha20poly1305_ABYTES
	L("SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_ABYTES", crypto_secretstream_xchacha20poly1305_ABYTES)
	L("SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_HEADERBYTES", crypto_secretstream_xchacha20poly1305_HEADERBYTES)
	L("SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_KEYBYTES", crypto_secretstream_xchacha20poly1305_KEYBYTES)
	L("SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_MESSAGEBYTES_MAX", crypto_secretstream_xchacha20poly1305_MESSAGEBYTES_MAX)
	L("SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_TAG_MESSAGE", crypto_secretstream_xchacha20poly1305_TAG_MESSAGE)
	L("SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_TAG_PUSH", crypto_secretstream_xchacha20poly1305_TAG_PUSH)
	L("SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_TAG_REKEY", crypto_secretstream_xchacha20poly1305_TAG_REKEY)
	L("SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_TAG_FINAL", crypto_secretstream_xchacha20poly1305_TAG_FINAL)
#endif
	L("SODIUM_CRYPTO_GENERICHASH_BYTES", crypto_generichash_BYTES)
	L("SODIUM_CRYPTO_GENERICHASH_BYTES_MIN", crypto_generichash_BYTES_MIN)
	L("SODIUM_CRYPTO_GENERICHASH_BYTES_MAX", crypto_generichash_BYTES_MAX)
	L("SODIUM_CRYPTO_GENERICHASH_KEYBYTES", crypto_generichash_KEYBYTES)
	L("SODIUM_CRYPTO_GENERICHASH_KEYBYTES_MIN", crypto_generichash_KEYBYTES_MIN)
	L("SODIUM_CRYPTO_GENERICHASH_KEYBYTES_MAX", crypto_generichash_KEYBYTES_MAX)
#ifdef crypto_pwhash_SALTBYTES
	L("SODIUM_CRYPTO_PWHASH_ALG_ARGON2I13", crypto_pwhash_ALG_ARGON2I13)
# ifdef crypto_pwhash_ALG_ARGON2ID13
	L("SODIUM_CRYPTO_PWHASH_ALG_ARGON2ID13", crypto_pwhash_ALG_ARGON2ID13)
# endif
	L("SODIUM_CRYPTO_PWHASH_ALG_DEFAULT", crypto_pwhash_ALG_DEFAULT)
	L("SODIUM_CRYPTO_PWHASH_SALTBYTES", crypto_pwhash_SALTBYTES)
	/* SODIUM_CRYPTO_PWHASH_STRPREFIX: rhp_sodium_str_consts */
	{ "SODIUM_CRYPTO_PWHASH_STRPREFIX", -1 },
	L("SODIUM_CRYPTO_PWHASH_OPSLIMIT_INTERACTIVE", crypto_pwhash_OPSLIMIT_INTERACTIVE)
	L("SODIUM_CRYPTO_PWHASH_MEMLIMIT_INTERACTIVE", crypto_pwhash_MEMLIMIT_INTERACTIVE)
	L("SODIUM_CRYPTO_PWHASH_OPSLIMIT_MODERATE", crypto_pwhash_OPSLIMIT_MODERATE)
	L("SODIUM_CRYPTO_PWHASH_MEMLIMIT_MODERATE", crypto_pwhash_MEMLIMIT_MODERATE)
	L("SODIUM_CRYPTO_PWHASH_OPSLIMIT_SENSITIVE", crypto_pwhash_OPSLIMIT_SENSITIVE)
	L("SODIUM_CRYPTO_PWHASH_MEMLIMIT_SENSITIVE", crypto_pwhash_MEMLIMIT_SENSITIVE)
#endif
#ifdef crypto_pwhash_scryptsalsa208sha256_SALTBYTES
	L("SODIUM_CRYPTO_PWHASH_SCRYPTSALSA208SHA256_SALTBYTES", crypto_pwhash_scryptsalsa208sha256_SALTBYTES)
	{ "SODIUM_CRYPTO_PWHASH_SCRYPTSALSA208SHA256_STRPREFIX", -1 },
	L("SODIUM_CRYPTO_PWHASH_SCRYPTSALSA208SHA256_OPSLIMIT_INTERACTIVE", crypto_pwhash_scryptsalsa208sha256_OPSLIMIT_INTERACTIVE)
	L("SODIUM_CRYPTO_PWHASH_SCRYPTSALSA208SHA256_MEMLIMIT_INTERACTIVE", crypto_pwhash_scryptsalsa208sha256_MEMLIMIT_INTERACTIVE)
	L("SODIUM_CRYPTO_PWHASH_SCRYPTSALSA208SHA256_OPSLIMIT_SENSITIVE", crypto_pwhash_scryptsalsa208sha256_OPSLIMIT_SENSITIVE)
	L("SODIUM_CRYPTO_PWHASH_SCRYPTSALSA208SHA256_MEMLIMIT_SENSITIVE", crypto_pwhash_scryptsalsa208sha256_MEMLIMIT_SENSITIVE)
#endif
	L("SODIUM_CRYPTO_SCALARMULT_BYTES", crypto_scalarmult_BYTES)
	L("SODIUM_CRYPTO_SCALARMULT_SCALARBYTES", crypto_scalarmult_SCALARBYTES)
	L("SODIUM_CRYPTO_SHORTHASH_BYTES", crypto_shorthash_BYTES)
	L("SODIUM_CRYPTO_SHORTHASH_KEYBYTES", crypto_shorthash_KEYBYTES)
	L("SODIUM_CRYPTO_SECRETBOX_KEYBYTES", crypto_secretbox_KEYBYTES)
	L("SODIUM_CRYPTO_SECRETBOX_MACBYTES", crypto_secretbox_MACBYTES)
	L("SODIUM_CRYPTO_SECRETBOX_NONCEBYTES", crypto_secretbox_NONCEBYTES)
	L("SODIUM_CRYPTO_SIGN_BYTES", crypto_sign_BYTES)
	L("SODIUM_CRYPTO_SIGN_SEEDBYTES", crypto_sign_SEEDBYTES)
	L("SODIUM_CRYPTO_SIGN_PUBLICKEYBYTES", crypto_sign_PUBLICKEYBYTES)
	L("SODIUM_CRYPTO_SIGN_SECRETKEYBYTES", crypto_sign_SECRETKEYBYTES)
	L("SODIUM_CRYPTO_SIGN_KEYPAIRBYTES", crypto_sign_SECRETKEYBYTES + crypto_sign_PUBLICKEYBYTES)
	L("SODIUM_CRYPTO_STREAM_NONCEBYTES", crypto_stream_NONCEBYTES)
	L("SODIUM_CRYPTO_STREAM_KEYBYTES", crypto_stream_KEYBYTES)
#ifdef crypto_stream_xchacha20_KEYBYTES
	L("SODIUM_CRYPTO_STREAM_XCHACHA20_NONCEBYTES", crypto_stream_xchacha20_NONCEBYTES)
	L("SODIUM_CRYPTO_STREAM_XCHACHA20_KEYBYTES", crypto_stream_xchacha20_KEYBYTES)
#endif
#ifdef sodium_base64_VARIANT_ORIGINAL
	L("SODIUM_BASE64_VARIANT_ORIGINAL", sodium_base64_VARIANT_ORIGINAL)
	L("SODIUM_BASE64_VARIANT_ORIGINAL_NO_PADDING", sodium_base64_VARIANT_ORIGINAL_NO_PADDING)
	L("SODIUM_BASE64_VARIANT_URLSAFE", sodium_base64_VARIANT_URLSAFE)
	L("SODIUM_BASE64_VARIANT_URLSAFE_NO_PADDING", sodium_base64_VARIANT_URLSAFE_NO_PADDING)
#endif
#ifdef crypto_core_ristretto255_HASHBYTES
	L("SODIUM_CRYPTO_SCALARMULT_RISTRETTO255_BYTES", crypto_scalarmult_ristretto255_BYTES)
	L("SODIUM_CRYPTO_SCALARMULT_RISTRETTO255_SCALARBYTES", crypto_scalarmult_ristretto255_SCALARBYTES)
	L("SODIUM_CRYPTO_CORE_RISTRETTO255_BYTES", crypto_core_ristretto255_BYTES)
	L("SODIUM_CRYPTO_CORE_RISTRETTO255_HASHBYTES", crypto_core_ristretto255_HASHBYTES)
	L("SODIUM_CRYPTO_CORE_RISTRETTO255_SCALARBYTES", crypto_core_ristretto255_SCALARBYTES)
	L("SODIUM_CRYPTO_CORE_RISTRETTO255_NONREDUCEDSCALARBYTES", crypto_core_ristretto255_NONREDUCEDSCALARBYTES)
#endif
	{ 0, 0 }
};

/* The string constants marked -1 above. */
const char *rhp_sodium_pwhash_strprefix(void)
{
#ifdef crypto_pwhash_SALTBYTES
	return crypto_pwhash_STRPREFIX;
#else
	return "";
#endif
}

const char *rhp_sodium_scrypt_strprefix(void)
{
#ifdef crypto_pwhash_scryptsalsa208sha256_SALTBYTES
	return crypto_pwhash_scryptsalsa208sha256_STRPREFIX;
#else
	return "";
#endif
}

/* SODIUM_VERSION_STRING (phpinfo's "libsodium headers version"). */
const char *rhp_sodium_header_version(void)
{
	return SODIUM_VERSION_STRING;
}

/* Which optional parts php compiles (the stub's #ifdefs): bit 0 AES-GCM,
 * 1 AEGIS-128L, 2 AEGIS-256, 3 XChaCha20-Poly1305 AEAD, 4 secretstream,
 * 5 pwhash (Argon2), 6 scrypt, 7 XChaCha20 stream, 8 base64, 9 Ristretto255,
 * 10 pwhash_str_needs_rehash, 11 Argon2id, 12 crypto_kdf_PRIMITIVE. */
unsigned rhp_sodium_features(void)
{
	unsigned f = 0;
#ifdef HAVE_AESGCM
	f |= 1u << 0;
#endif
#ifdef crypto_aead_aegis128l_KEYBYTES
	f |= 1u << 1;
#endif
#ifdef crypto_aead_aegis256_KEYBYTES
	f |= 1u << 2;
#endif
#ifdef crypto_aead_xchacha20poly1305_IETF_NPUBBYTES
	f |= 1u << 3;
#endif
#ifdef crypto_secretstream_xchacha20poly1305_ABYTES
	f |= 1u << 4;
#endif
#ifdef crypto_pwhash_SALTBYTES
	f |= 1u << 5;
#endif
#ifdef crypto_pwhash_scryptsalsa208sha256_SALTBYTES
	f |= 1u << 6;
#endif
#ifdef crypto_stream_xchacha20_KEYBYTES
	f |= 1u << 7;
#endif
#ifdef sodium_base64_VARIANT_ORIGINAL
	f |= 1u << 8;
#endif
#ifdef crypto_core_ristretto255_HASHBYTES
	f |= 1u << 9;
#endif
#if SODIUM_LIBRARY_VERSION_MAJOR > 9 || (SODIUM_LIBRARY_VERSION_MAJOR == 9 && SODIUM_LIBRARY_VERSION_MINOR >= 6)
	f |= 1u << 10;
#endif
#ifdef crypto_pwhash_ALG_ARGON2ID13
	f |= 1u << 11;
#endif
#ifdef crypto_kdf_PRIMITIVE
	f |= 1u << 12;
#endif
	return f;
}

/* sizeof the states php keeps in strings. */
size_t rhp_sodium_generichash_statebytes(void)
{
	return sizeof(crypto_generichash_state);
}

size_t rhp_sodium_secretstream_statebytes(void)
{
#ifdef crypto_secretstream_xchacha20poly1305_ABYTES
	return sizeof(crypto_secretstream_xchacha20poly1305_state);
#else
	return 0;
#endif
}

/* AEGIS (libsodium 1.0.19+), through the shim so an older libsodium
 * still links; rhp_sodium_features() says whether they exist. */
#define AEGIS(v, have) \
int rhp_sodium_##v##_encrypt(unsigned char *c, unsigned long long *clen, const unsigned char *m, \
	unsigned long long mlen, const unsigned char *ad, unsigned long long adlen, \
	const unsigned char *nsec, const unsigned char *npub, const unsigned char *k) \
{ return have ? AEGIS_CALL(v, encrypt, c, clen, m, mlen, ad, adlen, nsec, npub, k) : -1; } \
int rhp_sodium_##v##_decrypt(unsigned char *m, unsigned long long *mlen, unsigned char *nsec, \
	const unsigned char *c, unsigned long long clen, const unsigned char *ad, \
	unsigned long long adlen, const unsigned char *npub, const unsigned char *k) \
{ return have ? AEGIS_CALL(v, decrypt, m, mlen, nsec, c, clen, ad, adlen, npub, k) : -1; } \
void rhp_sodium_##v##_keygen(unsigned char *k) { if (have) AEGIS_KEYGEN(v, k); }

#ifdef crypto_aead_aegis256_KEYBYTES
# define AEGIS_CALL(v, f, ...) crypto_aead_##v##_##f(__VA_ARGS__)
# define AEGIS_KEYGEN(v, k) crypto_aead_##v##_keygen(k)
AEGIS(aegis128l, 1)
AEGIS(aegis256, 1)
#else
/* An older libsodium: the wrappers exist but are never called. */
# pragma GCC diagnostic ignored "-Wunused-parameter"
# define AEGIS_CALL(v, f, ...) -1
# define AEGIS_KEYGEN(v, k) (void) (k)
AEGIS(aegis128l, 0)
AEGIS(aegis256, 0)
#endif

/* php's internal limits, not constants of its own. */
const struct rhp_sodium_const rhp_sodium_internal[] = {
#ifdef crypto_pwhash_SALTBYTES
	L("crypto_pwhash_OPSLIMIT_MIN", crypto_pwhash_OPSLIMIT_MIN)
	L("crypto_pwhash_MEMLIMIT_MIN", crypto_pwhash_MEMLIMIT_MIN)
	L("crypto_pwhash_STRBYTES", crypto_pwhash_STRBYTES)
#endif
#ifdef crypto_pwhash_scryptsalsa208sha256_SALTBYTES
	L("crypto_pwhash_scryptsalsa208sha256_STRBYTES", crypto_pwhash_scryptsalsa208sha256_STRBYTES)
#endif
	L("crypto_generichash_blake2b_PERSONALBYTES", crypto_generichash_blake2b_PERSONALBYTES)
	L("crypto_generichash_blake2b_SALTBYTES", crypto_generichash_blake2b_SALTBYTES)
	{ 0, 0 }
};
