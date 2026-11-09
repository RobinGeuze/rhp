/*
 * The one stand-in for the Zend and PHP headers that vendored php-src C
 * includes (through shim/php.h and friends). It defines only what those files
 * use, with the meaning a release (NTS, non-debug) php build gives it. Every
 * library in this crate shares it, and so does rhp-mbfl-sys (through
 * DEP_RHP_CSRC_SHIM): extend this file, never add a second shim.
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
# define ZEND_LONG_MAX INT64_MAX
# define ZEND_LONG_MIN INT64_MIN
# define ZEND_LONG_FMT "%" PRId64
# define ZEND_ULONG_FMT "%" PRIu64
# define ZEND_STRTOUL(s0, s1, base) strtoull((s0), (s1), (base))
#else
typedef int32_t zend_long;
typedef uint32_t zend_ulong;
# define ZEND_LONG_MAX INT32_MAX
# define ZEND_LONG_MIN INT32_MIN
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

/* The fallible mallocs (_safe_femalloc, femalloc): a new block, through
 * safe_ferealloc. */
#define safe_femalloc(nmemb, size, offset, fallible) \
	((void)sizeof(char[(fallible) ? 1 : -1]), \
	 rhp_csrc_safe_ferealloc(NULL, (nmemb), (size), (offset)))
#define femalloc(size, fallible) safe_femalloc(1, (size), 0, (fallible))

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

/* zend_string.h, continued */
static zend_always_inline zend_string *zend_string_init(const char *str, size_t len, bool persistent)
{
	zend_string *ret = zend_string_alloc(len, persistent);
	memcpy(ZSTR_VAL(ret), str, len);
	ZSTR_VAL(ret)[len] = '\0';
	return ret;
}
#define ZSTR_INIT_LITERAL(s, persistent) zend_string_init((s), sizeof(s) - 1, (persistent))

/* Nothing here shares strings: a release frees, except the interned empty
 * string. */
static zend_always_inline void zend_string_release(zend_string *s)
{
	if (s != zend_empty_string) {
		efree(s);
	}
}
#define zend_string_release_ex(s, persistent) zend_string_release(s)

/* zend_alloc.h, continued */
char *rhp_csrc_estrndup(const char *s, size_t length);
#define estrndup(s, length) rhp_csrc_estrndup((s), (length))

/* zend_operators.h, continued: zend_str_tolower_dup (ASCII only, as
 * zend_tolower_ascii) and zend_memnstr. */
char *rhp_csrc_str_tolower_dup(const char *source, size_t length);
#define zend_str_tolower_dup(source, length) rhp_csrc_str_tolower_dup((source), (length))
const char *rhp_csrc_memnstr(const char *haystack, const char *needle, size_t needle_len, const char *end);
#define php_memnstr(haystack, needle, needle_len, end) \
	rhp_csrc_memnstr((haystack), (needle), (needle_len), (end))

/* zend_errors.h */
#define E_ERROR (1 << 0L)
#define E_WARNING (1 << 1L)
#define E_NOTICE (1 << 3L)
#define E_CORE_ERROR (1 << 4L)
#define E_DEPRECATED (1 << 13L)

/* php_error_docref, raised by the active builtin through the hook the
 * library's Rust side installs (see glue/). php's E_ERROR bails out; C here
 * can't, so zend_error_noreturn aborts. */
void rhp_csrc_error_docref(const char *docref, int type, const char *format, ...)
	__attribute__((format(printf, 3, 4)));
#define php_error_docref rhp_csrc_error_docref
_Noreturn void rhp_csrc_error_noreturn(int type, const char *format, ...)
	__attribute__((format(printf, 2, 3)));
#define zend_error_noreturn rhp_csrc_error_noreturn
/* zend_error, as php_error: only ever E_ERROR here. */
#define php_error(type, ...) rhp_csrc_error_noreturn((type), __VA_ARGS__)

/* main/snprintf.h and main/spprintf.h: php's own printf (shim/php_printf.c),
 * which php.h puts in place of the C library's for all php-src C. php's
 * builds have vasprintf and asprintf, so those stay the C library's. */
int rhp_csrc_snprintf(char *buf, size_t len, const char *format, ...)
	__attribute__((format(printf, 3, 4)));
int rhp_csrc_vsnprintf(char *buf, size_t len, const char *format, va_list ap);
int rhp_csrc_slprintf(char *buf, size_t len, const char *format, ...)
	__attribute__((format(printf, 3, 4)));
int rhp_csrc_vslprintf(char *buf, size_t len, const char *format, va_list ap);
size_t rhp_csrc_spprintf(char **pbuf, size_t max_len, const char *format, ...)
	__attribute__((format(printf, 3, 4)));
size_t rhp_csrc_vspprintf(char **pbuf, size_t max_len, const char *format, va_list ap);
#undef snprintf
#define snprintf rhp_csrc_snprintf
#undef vsnprintf
#define vsnprintf rhp_csrc_vsnprintf
#define slprintf rhp_csrc_slprintf
#define vslprintf rhp_csrc_vslprintf
#define spprintf rhp_csrc_spprintf
#define vspprintf rhp_csrc_vspprintf

/* The parts of the Zend and stream APIs ext/fileinfo's libmagic calls, as
 * types and declarations only: glue/fileinfo.c defines them, on hooks into
 * rhp's streams and PCRE (src/fileinfo). */
#include <sys/stat.h>
#include <sys/types.h>
#include <time.h>

/* zend_stream.h, zend_virtual_cwd.h: no virtual cwd (php's CLI and
 * CGI builds), so these are the system calls. */
typedef struct stat zend_stat_t;
typedef off_t zend_off_t;
#define zend_stat(path, buf) stat((path), (buf))
#define zend_fstat(fd, buf) fstat((fd), (buf))
#define php_sys_stat(path, buf) stat((path), (buf))
#define VCWD_ACCESS(path, mode) access((path), (mode))

/* main/php_reentrancy.h: the C library's, as on every POSIX build. */
#define php_localtime_r localtime_r
#define php_gmtime_r gmtime_r
#define php_asctime_r asctime_r
#define php_ctime_r ctime_r

/* main/php_streams.h */
typedef struct _php_stream php_stream;
typedef struct _php_stream_context php_stream_context;
typedef struct _php_stream_statbuf {
	zend_stat_t sb;
} php_stream_statbuf;
#ifndef MAXPATHLEN
# include <sys/param.h>
#endif
typedef struct _php_stream_dirent {
	char d_name[MAXPATHLEN];
	unsigned char d_type;
} php_stream_dirent;
#define REPORT_ERRORS 0x00000008
#define PHP_STREAM_AS_FD 1
typedef int php_socket_t;

php_stream *rhp_csrc_stream_open(const char *path, const char *mode, int options);
#define php_stream_open_wrapper(path, mode, options, opened) \
	rhp_csrc_stream_open((path), (mode), (options))
int rhp_csrc_stream_close(php_stream *stream);
#define php_stream_close(stream) rhp_csrc_stream_close(stream)
ssize_t rhp_csrc_stream_read(php_stream *stream, char *buf, size_t count);
#define php_stream_read(stream, buf, count) rhp_csrc_stream_read((stream), (buf), (count))
ssize_t rhp_csrc_stream_write(php_stream *stream, const char *buf, size_t count);
#define php_stream_write(stream, buf, count) rhp_csrc_stream_write((stream), (buf), (count))
char *rhp_csrc_stream_get_line(php_stream *stream, char *buf, size_t maxlen, size_t *returned_len);
#define php_stream_get_line(stream, buf, maxlen, retlen) \
	rhp_csrc_stream_get_line((stream), (buf), (maxlen), (retlen))
int rhp_csrc_stream_stat(php_stream *stream, php_stream_statbuf *ssb);
#define php_stream_stat(stream, ssb) rhp_csrc_stream_stat((stream), (ssb))
int rhp_csrc_stream_stat_path(const char *path, php_stream_statbuf *ssb);
#define php_stream_stat_path_ex(path, flags, ssb, context) rhp_csrc_stream_stat_path((path), (ssb))
int rhp_csrc_stream_cast(php_stream *stream, int castas, void **ret);
#define php_stream_cast(stream, as, ret, show_err) rhp_csrc_stream_cast((stream), (as), (ret))
php_stream *rhp_csrc_stream_opendir(const char *path, int options);
#define php_stream_opendir(path, options, context) rhp_csrc_stream_opendir((path), (options))
php_stream_dirent *rhp_csrc_stream_readdir(php_stream *dirstream, php_stream_dirent *ent);
#define php_stream_readdir(dirstream, dirent) rhp_csrc_stream_readdir((dirstream), (dirent))
#define php_stream_closedir(dirstream) rhp_csrc_stream_close(dirstream)

/* zend_types.h: the zvals php_pcre_match_impl fills, and only those (a
 * long, a string, a packed array). */
#define IS_UNDEF 0
#define IS_NULL 1
#define IS_FALSE 2
#define IS_TRUE 3
#define IS_LONG 4
#define IS_STRING 6
#define IS_ARRAY 7
typedef struct _zval zval;
typedef struct _zend_array {
	uint32_t nNumUsed;
	zval *arPacked;
} HashTable;
struct _zval {
	union {
		zend_long lval;
		zend_string *str;
		HashTable *arr;
	} value;
	uint8_t type;
};
#define Z_TYPE(zv) (zv).type
#define Z_TYPE_P(zv_p) Z_TYPE(*(zv_p))
#define Z_LVAL(zv) (zv).value.lval
#define Z_LVAL_P(zv_p) Z_LVAL(*(zv_p))
#define Z_STR(zv) (zv).value.str
#define Z_STRLEN_P(zv_p) ZSTR_LEN(Z_STR(*(zv_p)))
#define Z_ARRVAL(zv) (zv).value.arr
#define Z_ARRVAL_P(zv_p) Z_ARRVAL(*(zv_p))
#define ZVAL_NULL(z) ((z)->type = IS_NULL)
zval *rhp_csrc_hash_index_find(const HashTable *ht, zend_ulong h);
#define zend_hash_index_find(ht, h) rhp_csrc_hash_index_find((ht), (h))
void rhp_csrc_zval_ptr_dtor(zval *zv);
#define zval_ptr_dtor(zv) rhp_csrc_zval_ptr_dtor(zv)

/* ext/pcre/php_pcre.h: patterns compile and match through rhp's PCRE
 * extension (its cache, ini limits and preg_last_error), as php's do. */
typedef struct _pcre_cache_entry pcre_cache_entry;
typedef struct pcre2_real_code_8 pcre2_code;
typedef struct pcre2_real_match_data_8 pcre2_match_data;
typedef struct pcre2_real_match_context_8 pcre2_match_context;
typedef const unsigned char *PCRE2_SPTR;
#define PCRE2_CASELESS 0x00000008u
#define PCRE2_MULTILINE 0x00000400u
pcre_cache_entry *rhp_csrc_pcre_get_compiled_regex_cache(zend_string *regex);
#define pcre_get_compiled_regex_cache(regex) rhp_csrc_pcre_get_compiled_regex_cache(regex)
#define pcre_get_compiled_regex_cache_ex(regex, locale_aware) \
	rhp_csrc_pcre_get_compiled_regex_cache(regex)
#define php_pcre_pce_re(pce) ((pcre2_code *)(pce))
#define php_pcre_create_match_data(num_subpats, re) ((pcre2_match_data *)(re))
#define php_pcre_free_match_data(match_data) ((void)(match_data))
#define php_pcre_mctx() ((pcre2_match_context *)NULL)
int rhp_csrc_pcre2_match(const pcre2_code *code, PCRE2_SPTR subject, size_t length,
	size_t start_offset, uint32_t options, pcre2_match_data *match_data, pcre2_match_context *mcontext);
#define pcre2_match rhp_csrc_pcre2_match
void rhp_csrc_pcre_match_impl(pcre_cache_entry *pce, zend_string *subject_str, zval *return_value,
	zval *subpats, int global, zend_long flags, zend_long start_offset);
#define php_pcre_match_impl rhp_csrc_pcre_match_impl
zend_string *rhp_csrc_pcre_replace_impl(pcre_cache_entry *pce, zend_string *subject_str, const char *subject,
	size_t subject_len, zend_string *replace_str, size_t limit, size_t *replace_count);
#define php_pcre_replace_impl rhp_csrc_pcre_replace_impl

#endif
