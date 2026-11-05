/*
 * What Rust needs from libmbfl beyond its exported functions: the static
 * inline helpers of mbfl_encoding.h, the two tables mbstring.c compiles in
 * (eaw_table.h, rare_cp_bitvec.h), the MBFL_VERSION_* macros, and
 * compile-time checks of every constant and struct layout src/lib.rs
 * mirrors.
 */
#include <stddef.h>

#include "mbstring.h"
#include "mbfl_convert.h"
#include "php_unicode.h"
#include "eaw_table.h"
#include "rare_cp_bitvec.h"

#define P sizeof(void *)

_Static_assert(offsetof(mbfl_encoding, no_encoding) == 0, "mbfl_encoding");
_Static_assert(offsetof(mbfl_encoding, name) == P, "mbfl_encoding");
_Static_assert(offsetof(mbfl_encoding, mime_name) == 2 * P, "mbfl_encoding");
_Static_assert(offsetof(mbfl_encoding, aliases) == 3 * P, "mbfl_encoding");
_Static_assert(offsetof(mbfl_encoding, mblen_table) == 4 * P, "mbfl_encoding");
_Static_assert(offsetof(mbfl_encoding, flag) == 5 * P, "mbfl_encoding");
_Static_assert(offsetof(mbfl_encoding, input_filter) == 6 * P, "mbfl_encoding");
_Static_assert(offsetof(mbfl_encoding, output_filter) == 7 * P, "mbfl_encoding");
_Static_assert(offsetof(mbfl_encoding, to_wchar) == 8 * P, "mbfl_encoding");
_Static_assert(offsetof(mbfl_encoding, from_wchar) == 9 * P, "mbfl_encoding");
_Static_assert(offsetof(mbfl_encoding, check) == 10 * P, "mbfl_encoding");
_Static_assert(offsetof(mbfl_encoding, cut) == 11 * P, "mbfl_encoding");
_Static_assert(sizeof(mbfl_encoding) == 12 * P, "mbfl_encoding");
_Static_assert(sizeof(enum mbfl_no_encoding) == 4, "enum size");

_Static_assert(offsetof(mb_convert_buf, out) == 0, "mb_convert_buf");
_Static_assert(offsetof(mb_convert_buf, limit) == P, "mb_convert_buf");
_Static_assert(offsetof(mb_convert_buf, state) == 2 * P, "mb_convert_buf");
_Static_assert(offsetof(mb_convert_buf, errors) == 2 * P + 4, "mb_convert_buf");
_Static_assert(offsetof(mb_convert_buf, replacement_char) == 2 * P + 8, "mb_convert_buf");
_Static_assert(offsetof(mb_convert_buf, error_mode) == 2 * P + 12, "mb_convert_buf");
_Static_assert(offsetof(mb_convert_buf, str) == 2 * P + 16, "mb_convert_buf");
_Static_assert(sizeof(mb_convert_buf) == 3 * P + 16, "mb_convert_buf");

_Static_assert(sizeof(zend_ulong) == P, "zend_ulong");
_Static_assert(offsetof(zend_string, len) == 8 + P, "zend_string");
_Static_assert(offsetof(zend_string, val) == 8 + 2 * P, "zend_string");

_Static_assert(offsetof(mbfl_string, encoding) == 0, "mbfl_string");
_Static_assert(offsetof(mbfl_string, val) == P, "mbfl_string");
_Static_assert(offsetof(mbfl_string, len) == 2 * P, "mbfl_string");
_Static_assert(sizeof(mbfl_string) == 3 * P, "mbfl_string");

_Static_assert(MBFL_ENCTYPE_SBCS == 1 && MBFL_ENCTYPE_WCS2 == 2 && MBFL_ENCTYPE_WCS4 == 4,
	"MBFL_ENCTYPE_*");
_Static_assert(MBFL_OUTPUTFILTER_ILLEGAL_MODE_NONE == 0
	&& MBFL_OUTPUTFILTER_ILLEGAL_MODE_CHAR == 1
	&& MBFL_OUTPUTFILTER_ILLEGAL_MODE_LONG == 2
	&& MBFL_OUTPUTFILTER_ILLEGAL_MODE_ENTITY == 3
	&& MBFL_OUTPUTFILTER_ILLEGAL_MODE_BADUTF8 == 4, "MBFL_OUTPUTFILTER_ILLEGAL_MODE_*");
_Static_assert((uint32_t)MBFL_BAD_INPUT == 0xFFFFFFFFu, "MBFL_BAD_INPUT");
_Static_assert(MBSTRING_MIN_WCHAR_BUFSIZE == 5, "MBSTRING_MIN_WCHAR_BUFSIZE");
_Static_assert(FIRST_DOUBLEWIDTH_CODEPOINT == 0x1100, "FIRST_DOUBLEWIDTH_CODEPOINT");
_Static_assert(MBFL_VERSION_MAJOR == 1 && MBFL_VERSION_MINOR == 3 && MBFL_VERSION_TEENY == 2,
	"MBFL_VERSION_*");

_Static_assert(mbfl_no_encoding_invalid == -1, "mbfl_no_encoding");
_Static_assert(mbfl_no_encoding_qprint == 5, "mbfl_no_encoding");
_Static_assert(mbfl_no_encoding_charset_min == 8, "mbfl_no_encoding");
_Static_assert(mbfl_no_encoding_utf8 == 21, "mbfl_no_encoding");
_Static_assert(mbfl_no_encoding_utf8_sb == 25, "mbfl_no_encoding");
_Static_assert(mbfl_no_encoding_utf7 == 26, "mbfl_no_encoding");
_Static_assert(mbfl_no_encoding_utf7imap == 27, "mbfl_no_encoding");
_Static_assert(mbfl_no_encoding_jis == 41, "mbfl_no_encoding");
_Static_assert(mbfl_no_encoding_2022jpms == 45, "mbfl_no_encoding");
_Static_assert(mbfl_no_encoding_cp50220 == 79, "mbfl_no_encoding");
_Static_assert(mbfl_no_encoding_cp50222 == 81, "mbfl_no_encoding");

_Static_assert(PHP_UNICODE_CASE_UPPER == 0 && PHP_UNICODE_CASE_FOLD_SIMPLE == 7
	&& PHP_UNICODE_CASE_MODE_MAX == 8, "php_case_mode");

void rhp_mb_convert_buf_init(mb_convert_buf *buf, size_t initsize, uint32_t repl_char,
	unsigned int err_mode)
{
	mb_convert_buf_init(buf, initsize, repl_char, err_mode);
}

zend_string *rhp_mb_convert_buf_result(mb_convert_buf *buf, const mbfl_encoding *enc)
{
	return mb_convert_buf_result(buf, enc);
}

zend_string *rhp_mb_convert_buf_result_raw(mb_convert_buf *buf)
{
	return mb_convert_buf_result_raw(buf);
}

void rhp_mb_convert_buf_free(mb_convert_buf *buf)
{
	mb_convert_buf_free(buf);
}

/* MB_CONVERT_BUF_ENSURE_EX plus a copy, as mb_trim_string appends its trim
 * marker. False when the buffer can't grow (memory_limit). */
bool rhp_mb_convert_buf_append(mb_convert_buf *buf, const unsigned char *s, size_t len)
{
	if (!MB_CONVERT_BUF_ENSURE_EX(buf, buf->out, buf->limit, len)) {
		return false;
	}
	buf->out = zend_mempcpy(buf->out, s, len);
	return true;
}

const void *rhp_mbfl_eaw_table(size_t *len)
{
	_Static_assert(sizeof(mbfl_eaw_table[0]) == 8, "eaw table entries");
	*len = sizeof(mbfl_eaw_table) / sizeof(mbfl_eaw_table[0]);
	return mbfl_eaw_table;
}

const uint32_t *rhp_mbfl_rare_cp_bitvec(size_t *len)
{
	*len = sizeof(rare_codepoint_bitvec) / sizeof(rare_codepoint_bitvec[0]);
	return rare_codepoint_bitvec;
}

#define RHP_STR_(x) #x
#define RHP_STR(x) RHP_STR_(x)

/* "MBFL_VERSION_MAJOR.MINOR.TEENY", as PHP_MINFO(mbstring) prints it. */
const char *rhp_mbfl_version(void)
{
	return RHP_STR(MBFL_VERSION_MAJOR) "." RHP_STR(MBFL_VERSION_MINOR) "." RHP_STR(MBFL_VERSION_TEENY);
}
