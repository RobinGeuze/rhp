/* zend_shim.h — a minimal redefinition of the Zend extension API surface.
 *
 * The premise (see docs/design/extensions.md "Macro-shim"): a PHP extension's
 * .c sources almost never touch zval/HashTable/zend_string FIELDS directly; they
 * go through MACROS (PHP_FUNCTION, ZEND_PARSE_PARAMETERS, Z_PARAM, ZVAL, RETURN,
 * add_assoc / add_next_index) and a handful of inline helpers. If WE supply
 * those macros and types, the extension's real C algorithm compiles against OUR
 * definitions and runs against phrust's value model instead of the Zend engine.
 *
 * Types here are deliberately NOT ABI-compatible with real php. We never load a
 * .so; we recompile the .c. So the only contract is that the SPELLING of each
 * macro/helper the target source uses matches, and that our macro expands to
 * something that does the equivalent thing over the shim's own structs.
 *
 * Array building is delegated to Rust (option B in the task): array_init
 * allocates a phrust PhpArray behind a small integer handle, and every
 * add_assoc / add_next_index call goes back into Rust so PHP's exact key
 * normalization, insertion ordering and next-index logic are reused verbatim
 * rather than reimplemented in C.
 */
#ifndef PHRUST_ZEND_SHIM_H
#define PHRUST_ZEND_SHIM_H

#include <stddef.h>
#include <stdint.h>
#include <string.h>

/* ----------------------------------------------------------------------- */
/* Scalar typedefs                                                         */
/* ----------------------------------------------------------------------- */

typedef int64_t  zend_long;
typedef uint64_t zend_ulong;
typedef uint8_t  zend_bool;
typedef size_t   zend_size_t;

/* PHP's real type tag numbers (Zend/zend_types.h). Matching the numbers lets
 * real ext source that compares Z_TYPE_P(z) == IS_LONG etc. compile unchanged. */
#define IS_UNDEF     0
#define IS_NULL      1
#define IS_FALSE     2
#define IS_TRUE      3
#define IS_LONG      4
#define IS_DOUBLE    5
#define IS_STRING    6
#define IS_ARRAY     7
#define IS_OBJECT    8
#define IS_RESOURCE  9
#define IS_REFERENCE 10

/* ----------------------------------------------------------------------- */
/* zend_string — binary-safe, flexible-array-member byte string            */
/* ----------------------------------------------------------------------- */

typedef struct _zend_string {
    size_t len;
    size_t capacity;   /* usable bytes in `val` excluding the trailing NUL    */
    char   val[1];     /* flexible array member; always NUL-terminated        */
} zend_string;

#define ZSTR_VAL(s) ((s)->val)
#define ZSTR_LEN(s) ((s)->len)
#define ZSTR_H(s)   (0) /* precomputed hash: phrust hashes at the boundary */

/* ----------------------------------------------------------------------- */
/* zval — tagged value. value.arr is a 1-based handle into Rust's registry. */
/* ----------------------------------------------------------------------- */

typedef union _zend_value {
    zend_long    lval;
    double       dval;
    zend_string *str;
    uint32_t     arr;   /* handle (1-based; 0 == none) into the Rust registry */
} zend_value;

typedef struct _zval {
    zend_value value;
    uint8_t    u1_type;
} zval;

#define Z_TYPE(z)      ((z).u1_type)
#define Z_TYPE_P(z)    ((z)->u1_type)
#define Z_LVAL(z)      ((z).value.lval)
#define Z_LVAL_P(z)    ((z)->value.lval)
#define Z_DVAL(z)      ((z).value.dval)
#define Z_DVAL_P(z)    ((z)->value.dval)
#define Z_STR(z)       ((z).value.str)
#define Z_STR_P(z)     ((z)->value.str)
#define Z_STRVAL(z)    ZSTR_VAL((z).value.str)
#define Z_STRVAL_P(z)  ZSTR_VAL((z)->value.str)
#define Z_STRLEN(z)    ZSTR_LEN((z).value.str)
#define Z_STRLEN_P(z)  ZSTR_LEN((z)->value.str)
#define Z_ARRVAL_P(z)  ((z)->value.arr)

/* ----------------------------------------------------------------------- */
/* Calling convention                                                      */
/* ----------------------------------------------------------------------- */

typedef struct _zend_execute_data {
    uint32_t argc;
    zval    *argv;
} zend_execute_data;

#define INTERNAL_FUNCTION_PARAMETERS zend_execute_data *execute_data, zval *return_value
#define INTERNAL_FUNCTION_PARAM_PASSTHRU execute_data, return_value
#define ZEND_NUM_ARGS() (execute_data->argc)

/* PHP_FUNCTION(foo) / ZEND_FUNCTION(foo) → void zif_foo(INTERNAL_FUNCTION_PARAMETERS) */
#define ZEND_FN(name)             zif_##name
#define ZEND_NAMED_FUNCTION(name) void name(INTERNAL_FUNCTION_PARAMETERS)
#define ZEND_FUNCTION(name)       ZEND_NAMED_FUNCTION(ZEND_FN(name))
#define PHP_FUNCTION              ZEND_FUNCTION
#define PHP_NAMED_FUNCTION        ZEND_NAMED_FUNCTION

/* ----------------------------------------------------------------------- */
/* Memory manager → tracked malloc arena (freed after each call)           */
/* ----------------------------------------------------------------------- */

void  *zshim_malloc(size_t n);
void  *zshim_calloc(size_t n, size_t s);
void  *zshim_realloc(void *p, size_t n);
void   zshim_free(void *p);
char  *zshim_strdup(const char *s);
char  *zshim_strndup(const char *s, size_t n);

#define emalloc(n)         zshim_malloc(n)
#define ecalloc(n, s)      zshim_calloc((n), (s))
#define erealloc(p, n)     zshim_realloc((p), (n))
#define efree(p)           zshim_free(p)
#define estrdup(s)         zshim_strdup(s)
#define estrndup(s, n)     zshim_strndup((s), (n))
#define safe_emalloc(n, s, l) zshim_malloc((n) * (s) + (l))
#define pemalloc(n, persistent) zshim_malloc(n)
#define pefree(p, persistent)   zshim_free(p)

/* ----------------------------------------------------------------------- */
/* zend_string construction                                                */
/* ----------------------------------------------------------------------- */

zend_string *zshim_string_init(const char *s, size_t len);
zend_string *zshim_string_alloc(size_t len);
zend_string *zshim_string_safe_alloc(size_t n, size_t m, size_t l);
const char  *zshim_str_ptr(const zend_string *s); /* for the Rust boundary */
size_t       zshim_str_len(const zend_string *s); /* for the Rust boundary */

#define zend_string_init(s, len, persistent) zshim_string_init((s), (len))
#define zend_string_alloc(len, persistent)   zshim_string_alloc(len)
#define zend_string_safe_alloc(n, m, l, persistent) zshim_string_safe_alloc((n), (m), (l))
#define zend_string_release(s)      ((void)0) /* arena-freed */
#define zend_string_release_ex(s,p) ((void)0)
#define zend_string_free(s)         ((void)0)
#define zend_string_efree(s)        ((void)0)

/* ----------------------------------------------------------------------- */
/* ZVAL_* setters                                                          */
/* ----------------------------------------------------------------------- */

#define ZVAL_LONG(z, n)    do { (z)->value.lval = (zend_long)(n); (z)->u1_type = IS_LONG; } while (0)
#define ZVAL_DOUBLE(z, d)  do { (z)->value.dval = (double)(d); (z)->u1_type = IS_DOUBLE; } while (0)
#define ZVAL_NULL(z)       do { (z)->u1_type = IS_NULL; } while (0)
#define ZVAL_TRUE(z)       do { (z)->u1_type = IS_TRUE; } while (0)
#define ZVAL_FALSE(z)      do { (z)->u1_type = IS_FALSE; } while (0)
#define ZVAL_BOOL(z, b)    do { (z)->u1_type = (b) ? IS_TRUE : IS_FALSE; } while (0)
#define ZVAL_STR(z, s)     do { (z)->value.str = (s); (z)->u1_type = IS_STRING; } while (0)
#define ZVAL_STRING(z, s)  do { (z)->value.str = zshim_string_init((s), strlen(s)); (z)->u1_type = IS_STRING; } while (0)
#define ZVAL_STRINGL(z, s, l) do { (z)->value.str = zshim_string_init((s), (l)); (z)->u1_type = IS_STRING; } while (0)

/* ----------------------------------------------------------------------- */
/* RETVAL_* / RETURN_*                                                     */
/* ----------------------------------------------------------------------- */

#define RETVAL_LONG(n)      ZVAL_LONG(return_value, n)
#define RETVAL_DOUBLE(d)    ZVAL_DOUBLE(return_value, d)
#define RETVAL_NULL()       ZVAL_NULL(return_value)
#define RETVAL_TRUE         ZVAL_TRUE(return_value)
#define RETVAL_FALSE        ZVAL_FALSE(return_value)
#define RETVAL_BOOL(b)      ZVAL_BOOL(return_value, b)
#define RETVAL_STR(s)       ZVAL_STR(return_value, s)
#define RETVAL_STRING(s)    ZVAL_STRING(return_value, s)
#define RETVAL_STRINGL(s,l) ZVAL_STRINGL(return_value, s, l)

#define RETURN_LONG(n)      do { RETVAL_LONG(n); return; } while (0)
#define RETURN_DOUBLE(d)    do { RETVAL_DOUBLE(d); return; } while (0)
#define RETURN_NULL()       do { RETVAL_NULL(); return; } while (0)
#define RETURN_TRUE         do { RETVAL_TRUE; return; } while (0)
#define RETURN_FALSE        do { RETVAL_FALSE; return; } while (0)
#define RETURN_BOOL(b)      do { RETVAL_BOOL(b); return; } while (0)
#define RETURN_STR(s)       do { RETVAL_STR(s); return; } while (0)
#define RETURN_STRING(s)    do { RETVAL_STRING(s); return; } while (0)
#define RETURN_STRINGL(s,l) do { RETVAL_STRINGL(s, l); return; } while (0)

/* ----------------------------------------------------------------------- */
/* array_init + add_assoc_* / add_index_* / add_next_index_*               */
/* ----------------------------------------------------------------------- */

void zshim_array_init(zval *zv);
void zshim_add_assoc_long(zval *arr, const char *key, zend_long n);
void zshim_add_assoc_double(zval *arr, const char *key, double d);
void zshim_add_assoc_bool(zval *arr, const char *key, int b);
void zshim_add_assoc_null(zval *arr, const char *key);
void zshim_add_assoc_string(zval *arr, const char *key, const char *str);
void zshim_add_assoc_stringl(zval *arr, const char *key, const char *str, size_t len);
void zshim_add_assoc_zval(zval *arr, const char *key, zval *val);
void zshim_add_index_long(zval *arr, zend_ulong idx, zend_long n);
void zshim_add_next_index_long(zval *arr, zend_long n);
void zshim_add_next_index_double(zval *arr, double d);
void zshim_add_next_index_string(zval *arr, const char *str);
void zshim_add_next_index_stringl(zval *arr, const char *str, size_t len);
void zshim_add_next_index_zval(zval *arr, zval *val);

#define array_init(zv)                       zshim_array_init(zv)
#define array_init_size(zv, n)               zshim_array_init(zv)
#define add_assoc_long(arr, k, n)            zshim_add_assoc_long((arr), (k), (n))
#define add_assoc_double(arr, k, d)          zshim_add_assoc_double((arr), (k), (d))
#define add_assoc_bool(arr, k, b)            zshim_add_assoc_bool((arr), (k), (b))
#define add_assoc_null(arr, k)               zshim_add_assoc_null((arr), (k))
#define add_assoc_string(arr, k, s)          zshim_add_assoc_string((arr), (k), (s))
#define add_assoc_stringl(arr, k, s, l)      zshim_add_assoc_stringl((arr), (k), (s), (l))
#define add_assoc_zval(arr, k, v)            zshim_add_assoc_zval((arr), (k), (v))
#define add_index_long(arr, idx, n)          zshim_add_index_long((arr), (idx), (n))
#define add_next_index_long(arr, n)          zshim_add_next_index_long((arr), (n))
#define add_next_index_double(arr, d)        zshim_add_next_index_double((arr), (d))
#define add_next_index_string(arr, s)        zshim_add_next_index_string((arr), (s))
#define add_next_index_stringl(arr, s, l)    zshim_add_next_index_stringl((arr), (s), (l))
#define add_next_index_zval(arr, v)          zshim_add_next_index_zval((arr), (v))

/* ----------------------------------------------------------------------- */
/* zend_parse_parameters — fast ZPP form                                   */
/* ----------------------------------------------------------------------- */

zend_long zshim_get_long(const zval *z);
double    zshim_get_double(const zval *z);
int       zshim_get_bool(const zval *z);
void      zshim_get_string(const zval *z, char **out, size_t *out_len);
zend_string *zshim_get_str(const zval *z);
void      zshim_throw_arg_count(int min_c, int max_c, int got);

#define ZEND_PARSE_PARAMETERS_START(min_count, max_count)                       \
    do {                                                                        \
        uint32_t _zpp_i = 0; (void)_zpp_i;                                      \
        uint32_t _zpp_argc = ZEND_NUM_ARGS();                                   \
        if (_zpp_argc < (uint32_t)(min_count) ||                                \
            _zpp_argc > (uint32_t)(max_count)) {                                \
            zshim_throw_arg_count((min_count), (max_count), (int)_zpp_argc);    \
            return;                                                             \
        }

#define ZEND_PARSE_PARAMETERS_START_EX(flags, min_count, max_count)             \
    ZEND_PARSE_PARAMETERS_START((min_count), (max_count))

#define ZEND_PARSE_PARAMETERS_END() } while (0)
#define ZEND_PARSE_PARAMETERS_NONE() ZEND_PARSE_PARAMETERS_START(0, 0) ZEND_PARSE_PARAMETERS_END()

#define Z_PARAM_OPTIONAL /* arg boundary marker — no-op in the shim */

/* These expand to plain brace blocks (not do/while): a block is a complete
 * statement that needs no trailing ';', so consecutive Z_PARAM_* macros can be
 * juxtaposed with no separators, exactly as ext source writes them. `_zpp_i`
 * lives in the enclosing ZEND_PARSE_PARAMETERS_START scope. */
#define Z_PARAM_LONG(dest) \
    { (dest) = zshim_get_long(&execute_data->argv[_zpp_i]); _zpp_i++; }
#define Z_PARAM_DOUBLE(dest) \
    { (dest) = zshim_get_double(&execute_data->argv[_zpp_i]); _zpp_i++; }
#define Z_PARAM_BOOL(dest) \
    { (dest) = zshim_get_bool(&execute_data->argv[_zpp_i]); _zpp_i++; }
#define Z_PARAM_STRING(dest, dest_len) \
    { zshim_get_string(&execute_data->argv[_zpp_i], &(dest), &(dest_len)); _zpp_i++; }
#define Z_PARAM_STR(dest) \
    { (dest) = zshim_get_str(&execute_data->argv[_zpp_i]); _zpp_i++; }
#define Z_PARAM_PATH(dest, dest_len) Z_PARAM_STRING((dest), (dest_len))

/* ----------------------------------------------------------------------- */
/* Exceptions → a thread-local pending error the Rust boundary reads back  */
/* ----------------------------------------------------------------------- */

void zshim_throw(const char *class_name, const char *message);
int  zshim_has_exception(void);

#define zend_throw_exception(ce, message, code) zshim_throw("Exception", (message))
#define zend_value_error(...)                   zshim_throw("ValueError", "value error")
#define zend_argument_value_error(argn, ...)    zshim_throw("ValueError", "value error")

/* assorted no-op / trivial Zend helpers some sources reference */
#define ZEND_ASSERT(c)   ((void)0)
#define EXPECTED(c)      (c)
#define UNEXPECTED(c)    (c)
#ifndef ZEND_API
#define ZEND_API
#endif
#ifndef PHPAPI
#define PHPAPI
#endif
#ifndef ZEND_COLD
#define ZEND_COLD
#endif
#ifndef ZEND_SET_ALIGNED
#define ZEND_SET_ALIGNED(alignment, decl) decl
#endif

#endif /* PHRUST_ZEND_SHIM_H */
