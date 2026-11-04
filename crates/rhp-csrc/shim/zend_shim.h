/*
 * The one stand-in for the Zend and PHP headers that vendored php-src C
 * includes (through shim/php.h and friends). It defines only what those files
 * use, with the meaning a release (NTS, non-debug) php build gives it. Every
 * library in this crate shares it: extend this file, never add a second shim.
 */
#ifndef RHP_ZEND_SHIM_H
#define RHP_ZEND_SHIM_H

#include <inttypes.h>
#include <stdarg.h>
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
#define HAVE_STRINGS_H 1
#define HAVE_STRCASECMP 1

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
#define ZEND_API
#define BEGIN_EXTERN_C()
#define END_EXTERN_C()
#define EXPECTED(c) __builtin_expect(!!(c), 1)
#define UNEXPECTED(c) __builtin_expect(!!(c), 0)
#define ZEND_ASSERT(c) ((void)0)
#define ZEND_UNREACHABLE() __builtin_unreachable()
#define EMPTY_SWITCH_DEFAULT_CASE() default: ZEND_UNREACHABLE(); break;
#define zend_always_inline inline __attribute__((always_inline))
#define ZEND_ATTRIBUTE_UNUSED __attribute__((unused))
#define ZEND_NO_SANITIZE_ADDRESS
#if defined(__has_attribute) && __has_attribute(__fallthrough__)
# define ZEND_FALLTHROUGH __attribute__((__fallthrough__))
#else
# define ZEND_FALLTHROUGH ((void)0)
#endif
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
void *rhp_csrc_erealloc(void *ptr, size_t size);
char *rhp_csrc_estrdup(const char *s);
void *rhp_csrc_ecalloc(size_t nmemb, size_t size);
#define emalloc(size) rhp_csrc_emalloc(size)
#define efree(ptr) rhp_csrc_efree(ptr)
#define erealloc(ptr, size) rhp_csrc_erealloc((ptr), (size))
#define estrdup(s) rhp_csrc_estrdup(s)
#define ecalloc(nmemb, size) rhp_csrc_ecalloc((nmemb), (size))

/* nmemb * size + offset, or SIZE_MAX after recording php's "Possible
 * integer overflow in memory allocation" error for the VM (php bails out;
 * C here can't, so a caller that allocates SIZE_MAX aborts). */
size_t rhp_csrc_safe_address(size_t nmemb, size_t size, size_t offset);
#define zend_safe_address_guarded(nmemb, size, offset) \
	rhp_csrc_safe_address((nmemb), (size), (offset))

/* The fallible realloc (zend_alloc.h's _safe_ferealloc): past memory_limit,
 * or on overflow, it records the error for the VM and returns NULL, leaving
 * ptr alone. Every vendored caller passes fallible = true; the array size
 * turns any other constant into a compile error. */
void *rhp_csrc_safe_ferealloc(void *ptr, size_t nmemb, size_t size, size_t offset);
#define safe_ferealloc(ptr, nmemb, size, offset, fallible) \
	((void)sizeof(char[(fallible) ? 1 : -1]), \
	 rhp_csrc_safe_ferealloc((ptr), (nmemb), (size), (offset)))

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

/* zend_types.h */
typedef enum { SUCCESS = 0, FAILURE = -1 } zend_result;

typedef struct _zend_refcounted_h {
	uint32_t refcount;
	union {
		uint32_t type_info;
	} u;
} zend_refcounted_h;

/* Zend's layout: C code does pointer arithmetic on it (_ZSTR_STRUCT_SIZE). */
typedef struct _zend_string {
	zend_refcounted_h gc;
	zend_ulong h;
	size_t len;
	char val[1];
} zend_string;

#define GC_SET_REFCOUNT(p, rc) ((p)->gc.refcount = (rc))
#define GC_TYPE_INFO(p) (p)->gc.u.type_info
#define GC_ADD_FLAGS(p, flags) ((p)->gc.u.type_info |= (flags))
#define GC_NOT_COLLECTABLE (1 << 4)
#define GC_STRING (6 | GC_NOT_COLLECTABLE)
#define IS_STR_INTERNED (1 << 6)
#define IS_STR_VALID_UTF8 (1 << 9)

/* zend_string.h. Strings are emalloc'd; nothing is interned except the
 * empty string, which must never be freed. */
#define ZSTR_VAL(zstr) (zstr)->val
#define ZSTR_LEN(zstr) (zstr)->len
#define ZSTR_H(zstr) (zstr)->h
#define _ZSTR_HEADER_SIZE offsetof(zend_string, val)
#define _ZSTR_STRUCT_SIZE(len) (_ZSTR_HEADER_SIZE + len + 1)
#define ZEND_MM_ALIGNED_SIZE(size) (((size) + 7) & ~(size_t)7)

#define zend_empty_string rhp_zend_empty_string
extern zend_string *zend_empty_string;

static zend_always_inline zend_string *zend_string_alloc(size_t len, bool persistent)
{
	zend_string *ret = (zend_string *)emalloc(ZEND_MM_ALIGNED_SIZE(_ZSTR_STRUCT_SIZE(len)));
	(void)persistent;
	GC_SET_REFCOUNT(ret, 1);
	GC_TYPE_INFO(ret) = GC_STRING;
	ZSTR_H(ret) = 0;
	ZSTR_LEN(ret) = len;
	return ret;
}

/* php returns interned strings for lengths 0 and 1; only the empty one is
 * kept here. */
static zend_always_inline zend_string *zend_string_init_fast(const char *str, size_t len)
{
	if (len == 0) {
		return zend_empty_string;
	}
	zend_string *ret = zend_string_alloc(len, false);
	memcpy(ZSTR_VAL(ret), str, len);
	ZSTR_VAL(ret)[len] = '\0';
	return ret;
}

/* zend_bitset.h */
static zend_always_inline int zend_ulong_ntz(zend_ulong num)
{
#if UINTPTR_MAX > 0xffffffffu
	return __builtin_ctzll(num);
#else
	return __builtin_ctzl(num);
#endif
}

#endif
