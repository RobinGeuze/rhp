/* Out-of-line parts of zend_shim.h. */
#if defined(__APPLE__)
# define __STDC_WANT_LIB_EXT1__ 1
#endif
#include <stdio.h>
#include <string.h>

#include "zend_shim.h"

/* rhp_csrc_emalloc, _efree, _erealloc, _safe_address and _safe_ferealloc
 * are Rust (src/lib.rs). */

char *rhp_csrc_estrdup(const char *s)
{
	size_t n = strlen(s) + 1;
	return memcpy(rhp_csrc_emalloc(n), s, n);
}

void *rhp_csrc_ecalloc(size_t nmemb, size_t size)
{
	size_t n = rhp_csrc_safe_address(nmemb, size, 0);
	return memset(rhp_csrc_emalloc(n), 0, n);
}

void rhp_csrc_secure_zero(void *ptr, size_t size)
{
#if defined(__APPLE__)
	memset_s(ptr, size, 0, size);
#else
	volatile unsigned char *p = ptr;
	while (size--) {
		*p++ = 0;
	}
#endif
}

/* zend_string.c's interned empty string. */
static struct {
	zend_string s;
} empty_string = {{{1, {GC_STRING | IS_STR_INTERNED}}, 0, 0, {0}}};

zend_string *zend_empty_string = &empty_string.s;

char *rhp_csrc_estrndup(const char *s, size_t length)
{
	char *p = rhp_csrc_emalloc(length + 1);
	memcpy(p, s, length);
	p[length] = '\0';
	return p;
}

/* zend_str_tolower_dup: zend_tolower_ascii on each byte. */
char *rhp_csrc_str_tolower_dup(const char *source, size_t length)
{
	char *p = rhp_csrc_emalloc(length + 1);
	for (size_t i = 0; i < length; i++) {
		unsigned char c = (unsigned char)source[i];
		p[i] = (char)((c >= 'A' && c <= 'Z') ? c + ('a' - 'A') : c);
	}
	p[length] = '\0';
	return p;
}

/* zend_memnstr: the first occurrence of needle in [haystack, end). */
const char *rhp_csrc_memnstr(const char *haystack, const char *needle, size_t needle_len, const char *end)
{
	if (needle_len == 0) {
		return haystack;
	}
	size_t n = (size_t)(end - haystack);
	if (needle_len > n) {
		return NULL;
	}
	const char *last = end - needle_len;
	for (const char *p = haystack; p <= last; p++) {
		p = memchr(p, *needle, (size_t)(last - p) + 1);
		if (p == NULL) {
			return NULL;
		}
		if (memcmp(p, needle, needle_len) == 0) {
			return p;
		}
	}
	return NULL;
}
