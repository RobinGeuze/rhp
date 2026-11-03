/*
 * Force-included (-include) into every C file of the crypt archive. Gives each
 * global symbol the vendored php-src crypt C defines an rhp_ prefix, so no
 * other static C library can clash with it or bind to it. Macros only: this
 * comes before every system header. src/lib.rs tests that the archive
 * exports nothing else.
 */
#ifndef RHP_PREFIX_CRYPT_H
#define RHP_PREFIX_CRYPT_H

#define __php_stpncpy rhp_php_stpncpy
#define _crypt_extended_init rhp_crypt_extended_init
#define _crypt_extended_r rhp_crypt_extended_r
#define php_crypt_blowfish_rn rhp_php_crypt_blowfish_rn
#define php_sha256_crypt rhp_php_sha256_crypt
#define php_sha256_crypt_r rhp_php_sha256_crypt_r
#define php_sha512_crypt rhp_php_sha512_crypt
#define php_sha512_crypt_r rhp_php_sha512_crypt_r

#endif
