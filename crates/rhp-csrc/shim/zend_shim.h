/*
 * The one stand-in for the Zend and PHP headers that vendored php-src C
 * includes (through shim/php.h and friends). It defines only what those files
 * use, with the meaning a release (NTS, non-debug) php build gives it. Every
 * library in this crate shares it: extend this file, never add a second shim.
 */
#ifndef RHP_ZEND_SHIM_H
#define RHP_ZEND_SHIM_H

#include <inttypes.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#if !defined(__APPLE__)
# include <alloca.h>
#endif

/* php_config.h */
#if defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__
# define WORDS_BIGENDIAN 1
#endif
#define HAVE_ALIGNOF 1

#define PHPAPI
#define ZEND_DEBUG 0

/* zend_long.h */
#if UINTPTR_MAX > 0xffffffffu
typedef int64_t zend_long;
typedef uint64_t zend_ulong;
# define ZEND_LONG_FMT "%" PRId64
# define ZEND_ULONG_FMT "%" PRIu64
# define ZEND_STRTOUL(s0, s1, base) strtoull((s0), (s1), (base))
#else
typedef int32_t zend_long;
typedef uint32_t zend_ulong;
# define ZEND_LONG_FMT "%" PRId32
# define ZEND_ULONG_FMT "%" PRIu32
# define ZEND_STRTOUL(s0, s1, base) strtoul((s0), (s1), (base))
#endif

/* zend_portability.h */
#define EXPECTED(c) __builtin_expect(!!(c), 1)
#define UNEXPECTED(c) __builtin_expect(!!(c), 0)
#define ZEND_ASSERT(c) ((void)0)
/* Thread-local as in ZTS builds (NTS has plain static): rhp runs one VM per
 * thread. */
#define ZEND_TLS static _Thread_local
#define ZEND_SET_ALIGNED(alignment, decl) decl __attribute__((__aligned__(alignment)))
#if defined(__has_attribute) && __has_attribute(nonstring)
# define ZEND_NONSTRING __attribute__((nonstring))
#else
# define ZEND_NONSTRING
#endif
#ifndef MIN
# define MIN(a, b) (((a) < (b)) ? (a) : (b))
#endif
#ifndef MAX
# define MAX(a, b) (((a) > (b)) ? (a) : (b))
#endif

/* zend_alloc.h: Rust's global allocator (src/lib.rs), so this memory
 * counts against memory_limit as php's emalloc does. Like a non-fallible
 * emalloc it never returns NULL. */
void *rhp_csrc_emalloc(size_t size);
void rhp_csrc_efree(void *ptr);
char *rhp_csrc_estrdup(const char *s);
#define emalloc(size) rhp_csrc_emalloc(size)
#define efree(ptr) rhp_csrc_efree(ptr)
#define estrdup(s) rhp_csrc_estrdup(s)

/* Stack buffers up to 32 KiB, heap above, as php does (always heap on
 * macOS, where php's do_alloca never uses alloca). */
#if !defined(__APPLE__)
# define ZEND_ALLOCA_MAX_SIZE (32 * 1024)
# define ALLOCA_FLAG(name) bool name;
# define SET_ALLOCA_FLAG(name) name = true
# define do_alloca_ex(size, limit, use_heap) \
	((use_heap = (UNEXPECTED((size) > (limit)))) ? emalloc(size) : alloca(size))
# define do_alloca(size, use_heap) do_alloca_ex(size, ZEND_ALLOCA_MAX_SIZE, use_heap)
# define free_alloca(p, use_heap) \
	do { if (UNEXPECTED(use_heap)) efree(p); } while (0)
#else
# define ALLOCA_FLAG(name)
# define SET_ALLOCA_FLAG(name)
# define do_alloca(size, use_heap) emalloc(size)
# define free_alloca(p, use_heap) efree(p)
#endif

/* A memset the compiler may not drop. */
void rhp_csrc_secure_zero(void *ptr, size_t size);
#define ZEND_SECURE_ZERO(var, size) rhp_csrc_secure_zero((var), (size))

/* zend_operators.h */
static inline void *zend_mempcpy(void *dest, const void *src, size_t n)
{
	return (char *)memcpy(dest, src, n) + n;
}

#endif
