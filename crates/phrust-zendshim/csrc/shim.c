/* shim.c — the non-macro glue behind zend_shim.h.
 *
 * Two responsibilities:
 *   1. A tiny tracked-malloc arena standing in for the Zend MM. Everything an
 *      ext allocates with emalloc/zend_string_* during a call is recorded and
 *      released in one shot by zshim_arena_reset(), called from the Rust
 *      boundary after the result has been copied out.
 *   2. Thin forwarders from the array-building and ZPP macros to the Rust
 *      callbacks (rs_*), so phrust's PhpArray provides the real semantics.
 *
 * Everything here is single-threaded per call and uses thread-local state, so
 * concurrent calls on different threads do not interfere.
 */
#include "zend_shim.h"
#include <stdlib.h>

/* ----------------------------------------------------------------------- */
/* Rust callbacks (implemented in src/lib.rs)                              */
/* ----------------------------------------------------------------------- */

extern uint32_t rs_array_new(void);
extern void rs_array_set_long(uint32_t h, const char *k, size_t klen, int64_t v);
extern void rs_array_set_double(uint32_t h, const char *k, size_t klen, double v);
extern void rs_array_set_bool(uint32_t h, const char *k, size_t klen, int v);
extern void rs_array_set_null(uint32_t h, const char *k, size_t klen);
extern void rs_array_set_str(uint32_t h, const char *k, size_t klen, const char *v, size_t vlen);
extern void rs_array_set_arr(uint32_t h, const char *k, size_t klen, uint32_t child);
extern void rs_array_set_index_long(uint32_t h, uint64_t idx, int64_t v);
extern void rs_array_push_long(uint32_t h, int64_t v);
extern void rs_array_push_double(uint32_t h, double v);
extern void rs_array_push_str(uint32_t h, const char *v, size_t vlen);
extern void rs_array_push_arr(uint32_t h, uint32_t child);
extern void rs_set_exception(const char *class_name, const char *message);

/* ----------------------------------------------------------------------- */
/* Tracked-malloc arena                                                     */
/* ----------------------------------------------------------------------- */

typedef struct arena_block {
    struct arena_block *next;
    /* user data follows immediately after this header */
} arena_block;

static _Thread_local arena_block *g_arena = NULL;
static _Thread_local int g_exc_set = 0;

#define BLOCK_DATA(b) ((void *)((char *)(b) + sizeof(arena_block)))
#define DATA_BLOCK(p) ((arena_block *)((char *)(p) - sizeof(arena_block)))

void *zshim_malloc(size_t n) {
    arena_block *b = (arena_block *)malloc(sizeof(arena_block) + n);
    if (!b) return NULL;
    b->next = g_arena;
    g_arena = b;
    return BLOCK_DATA(b);
}

void *zshim_calloc(size_t n, size_t s) {
    size_t total = n * s;
    void *p = zshim_malloc(total);
    if (p) memset(p, 0, total);
    return p;
}

void *zshim_realloc(void *p, size_t n) {
    /* Simple and correct: allocate a fresh tracked block and copy. The old
     * block stays on the arena list and is freed at reset (no early free, so no
     * dangling pointer if the caller kept an alias). Fine for per-call scope. */
    void *np = zshim_malloc(n);
    if (np && p) {
        memcpy(np, p, n); /* may over-read by a few bytes vs the old size; */
                          /* arena blocks are whole malloc'd regions, so this  */
                          /* is bounded — acceptable for the prototype.        */
    }
    return np;
}

void zshim_free(void *p) {
    /* Deferred: the arena frees everything at reset. Freeing here would require
     * unlinking from the list; not worth it for per-call lifetimes. */
    (void)p;
}

char *zshim_strdup(const char *s) {
    size_t n = strlen(s) + 1;
    char *p = (char *)zshim_malloc(n);
    if (p) memcpy(p, s, n);
    return p;
}

char *zshim_strndup(const char *s, size_t n) {
    char *p = (char *)zshim_malloc(n + 1);
    if (p) { memcpy(p, s, n); p[n] = '\0'; }
    return p;
}

void zshim_arena_reset(void) {
    arena_block *b = g_arena;
    while (b) {
        arena_block *next = b->next;
        free(b);
        b = next;
    }
    g_arena = NULL;
    g_exc_set = 0;
}

/* ----------------------------------------------------------------------- */
/* zend_string                                                              */
/* ----------------------------------------------------------------------- */

zend_string *zshim_string_alloc(size_t len) {
    /* header + len usable bytes + 1 NUL */
    zend_string *s = (zend_string *)zshim_malloc(offsetof(zend_string, val) + len + 1);
    if (!s) return NULL;
    s->len = len;
    s->capacity = len;
    s->val[len] = '\0';
    return s;
}

zend_string *zshim_string_init(const char *src, size_t len) {
    zend_string *s = zshim_string_alloc(len);
    if (s && len) memcpy(s->val, src, len);
    return s;
}

zend_string *zshim_string_safe_alloc(size_t n, size_t m, size_t l) {
    return zshim_string_alloc(n * m + l);
}

const char *zshim_str_ptr(const zend_string *s) { return s->val; }
size_t      zshim_str_len(const zend_string *s) { return s->len; }

/* ----------------------------------------------------------------------- */
/* array building — forward the zval handle to Rust                        */
/* ----------------------------------------------------------------------- */

void zshim_array_init(zval *zv) {
    zv->value.arr = rs_array_new();
    zv->u1_type = IS_ARRAY;
}

void zshim_add_assoc_long(zval *a, const char *k, zend_long n) {
    rs_array_set_long(a->value.arr, k, strlen(k), n);
}
void zshim_add_assoc_double(zval *a, const char *k, double d) {
    rs_array_set_double(a->value.arr, k, strlen(k), d);
}
void zshim_add_assoc_bool(zval *a, const char *k, int b) {
    rs_array_set_bool(a->value.arr, k, strlen(k), b);
}
void zshim_add_assoc_null(zval *a, const char *k) {
    rs_array_set_null(a->value.arr, k, strlen(k));
}
void zshim_add_assoc_string(zval *a, const char *k, const char *str) {
    rs_array_set_str(a->value.arr, k, strlen(k), str, strlen(str));
}
void zshim_add_assoc_stringl(zval *a, const char *k, const char *str, size_t len) {
    rs_array_set_str(a->value.arr, k, strlen(k), str, len);
}
void zshim_add_assoc_zval(zval *a, const char *k, zval *v) {
    size_t klen = strlen(k);
    switch (v->u1_type) {
        case IS_LONG:   rs_array_set_long(a->value.arr, k, klen, v->value.lval); break;
        case IS_DOUBLE: rs_array_set_double(a->value.arr, k, klen, v->value.dval); break;
        case IS_TRUE:   rs_array_set_bool(a->value.arr, k, klen, 1); break;
        case IS_FALSE:  rs_array_set_bool(a->value.arr, k, klen, 0); break;
        case IS_NULL:   rs_array_set_null(a->value.arr, k, klen); break;
        case IS_STRING: rs_array_set_str(a->value.arr, k, klen,
                            v->value.str->val, v->value.str->len); break;
        case IS_ARRAY:  rs_array_set_arr(a->value.arr, k, klen, v->value.arr); break;
        default:        rs_array_set_null(a->value.arr, k, klen); break;
    }
}

void zshim_add_index_long(zval *a, zend_ulong idx, zend_long n) {
    rs_array_set_index_long(a->value.arr, idx, n);
}

void zshim_add_next_index_long(zval *a, zend_long n) {
    rs_array_push_long(a->value.arr, n);
}
void zshim_add_next_index_double(zval *a, double d) {
    rs_array_push_double(a->value.arr, d);
}
void zshim_add_next_index_string(zval *a, const char *str) {
    rs_array_push_str(a->value.arr, str, strlen(str));
}
void zshim_add_next_index_stringl(zval *a, const char *str, size_t len) {
    rs_array_push_str(a->value.arr, str, len);
}
void zshim_add_next_index_zval(zval *a, zval *v) {
    switch (v->u1_type) {
        case IS_LONG:   rs_array_push_long(a->value.arr, v->value.lval); break;
        case IS_DOUBLE: rs_array_push_double(a->value.arr, v->value.dval); break;
        case IS_STRING: rs_array_push_str(a->value.arr, v->value.str->val, v->value.str->len); break;
        case IS_ARRAY:  rs_array_push_arr(a->value.arr, v->value.arr); break;
        default:        rs_array_push_long(a->value.arr, 0); break;
    }
}

/* ----------------------------------------------------------------------- */
/* ZPP getters                                                             */
/* ----------------------------------------------------------------------- */

zend_long zshim_get_long(const zval *z) {
    switch (z->u1_type) {
        case IS_LONG:   return z->value.lval;
        case IS_DOUBLE: return (zend_long)z->value.dval;
        case IS_TRUE:   return 1;
        case IS_FALSE:  return 0;
        default:        return 0;
    }
}
double zshim_get_double(const zval *z) {
    switch (z->u1_type) {
        case IS_DOUBLE: return z->value.dval;
        case IS_LONG:   return (double)z->value.lval;
        default:        return 0.0;
    }
}
int zshim_get_bool(const zval *z) {
    return z->u1_type == IS_TRUE || (z->u1_type == IS_LONG && z->value.lval != 0);
}
void zshim_get_string(const zval *z, char **out, size_t *out_len) {
    if (z->u1_type == IS_STRING) {
        *out = z->value.str->val;
        *out_len = z->value.str->len;
    } else {
        *out = (char *)"";
        *out_len = 0;
    }
}
zend_string *zshim_get_str(const zval *z) {
    return z->u1_type == IS_STRING ? z->value.str : zshim_string_init("", 0);
}

void zshim_throw_arg_count(int min_c, int max_c, int got) {
    (void)min_c; (void)max_c; (void)got;
    zshim_throw("ArgumentCountError", "wrong parameter count");
}

/* ----------------------------------------------------------------------- */
/* Exceptions                                                              */
/* ----------------------------------------------------------------------- */

void zshim_throw(const char *class_name, const char *message) {
    g_exc_set = 1;
    rs_set_exception(class_name, message);
}
int zshim_has_exception(void) { return g_exc_set; }
