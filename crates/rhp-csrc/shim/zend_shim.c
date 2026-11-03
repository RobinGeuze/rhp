/* Out-of-line parts of zend_shim.h. */
#if defined(__APPLE__)
# define __STDC_WANT_LIB_EXT1__ 1
#endif
#include <stdio.h>
#include <string.h>

#include "zend_shim.h"

/* rhp_csrc_emalloc and rhp_csrc_efree are Rust (src/lib.rs). */

char *rhp_csrc_estrdup(const char *s)
{
	size_t n = strlen(s) + 1;
	return memcpy(rhp_csrc_emalloc(n), s, n);
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
