/* php_bin2hex.c — Stage 2: a REAL php-src function compiled against the shim.
 *
 * The three regions below are copied VERBATIM from php-8.5.8 ext/standard/string.c
 * (hexconvtab at line 55; php_bin2hex at lines 62-77; PHP_FUNCTION(bin2hex) at
 * lines 158-171). The ONLY change from upstream is `#include "zend_shim.h"` in
 * place of the ext's own `php.h`/`php_string.h`, and that `php_bin2hex` is made
 * non-static so the Rust bridge can reach it is NOT needed — only the
 * `zif_bin2hex` entry is called, and it references `php_bin2hex` within this TU.
 *
 * It compiles unmodified because every token it uses — ZEND_SET_ALIGNED,
 * zend_string, zend_string_safe_alloc, ZSTR_VAL, ZSTR_LEN, ZEND_PARSE_PARAMETERS_*,
 * Z_PARAM_STR, PHP_FUNCTION, RETURN_STR — is provided by zend_shim.h.
 */
#include "zend_shim.h"

/* --- verbatim: ext/standard/string.c:55 ------------------------------------ */
/* this is read-only, so it's ok */
ZEND_SET_ALIGNED(16, static const char hexconvtab[]) = "0123456789abcdef";

/* --- verbatim: ext/standard/string.c:62-77 --------------------------------- */
/* {{{ php_bin2hex */
static zend_string *php_bin2hex(const unsigned char *old, const size_t oldlen)
{
	zend_string *result;
	size_t i, j;

	result = zend_string_safe_alloc(oldlen, 2 * sizeof(char), 0, 0);

	for (i = j = 0; i < oldlen; i++) {
		ZSTR_VAL(result)[j++] = hexconvtab[old[i] >> 4];
		ZSTR_VAL(result)[j++] = hexconvtab[old[i] & 15];
	}
	ZSTR_VAL(result)[j] = '\0';

	return result;
}
/* }}} */

/* --- verbatim: ext/standard/string.c:158-171 ------------------------------- */
/* {{{ Converts the binary representation of data to hex */
PHP_FUNCTION(bin2hex)
{
	zend_string *result;
	zend_string *data;

	ZEND_PARSE_PARAMETERS_START(1, 1)
		Z_PARAM_STR(data)
	ZEND_PARSE_PARAMETERS_END();

	result = php_bin2hex((unsigned char *)ZSTR_VAL(data), ZSTR_LEN(data));

	RETURN_STR(result);
}
/* }}} */
