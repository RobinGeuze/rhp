/*
 * What php's build gives timelib (ext/date/config0.m4): the generated
 * timelib_config.h, which maps timelib's allocator to emalloc so its memory
 * counts against memory_limit, and the php_config.h settings timelib reads,
 * as php's configure sets them on a 64-bit Unix.
 */
#ifndef RHP_TIMELIB_CONFIG_H
#define RHP_TIMELIB_CONFIG_H

#include <inttypes.h>
#include <stddef.h>
#include <stdint.h>

/* php_config.h */
#define HAVE_ATOLL 1
#define HAVE_DIRENT_H 1
#define HAVE_STDINT_H 1
#define HAVE_STRTOLL 1
#define HAVE_SYS_TIME_H 1
#define HAVE_SYS_TYPES_H 1
#define HAVE_UNISTD_H 1
#define SIZEOF_INT __SIZEOF_INT__
#define SIZEOF_LONG __SIZEOF_LONG__
#if defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__
# define WORDS_BIGENDIAN 1
#endif

/* emalloc and efree are src/lib.rs's; the rest, php's erealloc, ecalloc,
 * estrdup and estrndup over them, are glue/timelib.c's and src/timelib.rs's. */
void *rhp_csrc_emalloc(size_t size);
void rhp_csrc_efree(void *ptr);
void *rhp_timelib_realloc(void *ptr, size_t size);
void *rhp_timelib_calloc(size_t nmemb, size_t size);
char *rhp_timelib_strdup(const char *s);
char *rhp_timelib_strndup(const char *s, size_t length);

#define timelib_malloc  rhp_csrc_emalloc
#define timelib_realloc rhp_timelib_realloc
#define timelib_calloc  rhp_timelib_calloc
#define timelib_strdup  rhp_timelib_strdup
#define timelib_strndup rhp_timelib_strndup
#define timelib_free    rhp_csrc_efree

#endif
