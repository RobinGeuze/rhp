/*
 * What php's build gives libmagic around it: php's streams, PCRE,
 * php_error_docref and the zvals php_pcre_match_impl fills (declared in
 * shim/zend_shim.h). Each goes through a hook src/fileinfo installs, which
 * rhp-builtins points at its streams and PCRE extension. Without hooks
 * (rhp-csrc's own tests) streams fail to open, patterns don't compile and
 * warnings are dropped.
 */
#include <errno.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

#include "zend_shim.h"
#include "magic.h"

/* Mirrored by src/fileinfo/sys.rs. */
struct rhp_fi_stat {
	uint32_t mode;
	int64_t size, dev, ino, nlink, uid, gid, rdev, atime, mtime, ctime, blksize, blocks;
};

struct rhp_fi_hooks {
	void *(*open)(const char *path, const char *mode, int report_errors);
	void *(*opendir)(const char *path, int report_errors);
	int (*close)(void *h);
	ssize_t (*read)(void *h, char *buf, size_t count);
	ssize_t (*write)(void *h, const char *buf, size_t count);
	/* A line of at most max bytes, or -1 at the end. */
	ssize_t (*get_line)(void *h, char *buf, size_t max);
	/* An entry's name (cut to cap - 1 bytes), or -1 at the end. */
	ssize_t (*readdir)(void *h, char *buf, size_t cap);
	int (*stat)(void *h, struct rhp_fi_stat *st);
	int (*stat_path)(const char *path, struct rhp_fi_stat *st);
	int (*fd)(void *h);
	void (*error)(int type, const char *msg, size_t len);
	void *(*pcre_compile)(const char *pattern, size_t len);
	/* php_pcre_match_impl without subpatterns: 1 with the match's bounds,
	 * 0, or -1 on a match error. */
	int (*pcre_match)(void *re, const char *subject, size_t len, size_t *start, size_t *end);
	/* pcre2_match's result (> 0 matched, < 0 no match or an error). */
	int (*pcre_test)(void *re, const char *subject, size_t len, size_t start);
	/* php_pcre_replace_impl: an emalloc'd result, or NULL. */
	char *(*pcre_replace)(void *re, const char *subject, size_t len, const char *rep, size_t rep_len,
		size_t limit, size_t *count, size_t *out_len);
};

static const struct rhp_fi_hooks *hooks;

void rhp_fi_set_hooks(const struct rhp_fi_hooks *h)
{
	__atomic_store_n(&hooks, h, __ATOMIC_RELEASE);
}

static const struct rhp_fi_hooks *H(void)
{
	return __atomic_load_n(&hooks, __ATOMIC_ACQUIRE);
}

/* A stream or directory: the hooks' handle. */
static void *handle(php_stream *s)
{
	return (void *)s;
}

php_stream *rhp_csrc_stream_open(const char *path, const char *mode, int options)
{
	if (!H()) {
		errno = ENOENT;
		return NULL;
	}
	return (php_stream *)H()->open(path, mode, (options & REPORT_ERRORS) != 0);
}

php_stream *rhp_csrc_stream_opendir(const char *path, int options)
{
	if (!H()) {
		return NULL;
	}
	return (php_stream *)H()->opendir(path, (options & REPORT_ERRORS) != 0);
}

int rhp_csrc_stream_close(php_stream *stream)
{
	return H() ? H()->close(handle(stream)) : -1;
}

ssize_t rhp_csrc_stream_read(php_stream *stream, char *buf, size_t count)
{
	return H() ? H()->read(handle(stream), buf, count) : -1;
}

ssize_t rhp_csrc_stream_write(php_stream *stream, const char *buf, size_t count)
{
	return H() ? H()->write(handle(stream), buf, count) : -1;
}

/* _php_stream_get_line with a buffer: a line of at most maxlen - 1 bytes,
 * NUL-terminated, or NULL at the end. */
char *rhp_csrc_stream_get_line(php_stream *stream, char *buf, size_t maxlen, size_t *returned_len)
{
	if (!H() || maxlen == 0) {
		return NULL;
	}
	ssize_t n = H()->get_line(handle(stream), buf, maxlen - 1);
	if (n < 0) {
		return NULL;
	}
	buf[n] = '\0';
	if (returned_len) {
		*returned_len = (size_t)n;
	}
	return buf;
}

static void fill_stat(zend_stat_t *sb, const struct rhp_fi_stat *st)
{
	memset(sb, 0, sizeof(*sb));
	sb->st_mode = (mode_t)st->mode;
	sb->st_size = (off_t)st->size;
	sb->st_dev = (dev_t)st->dev;
	sb->st_ino = (ino_t)st->ino;
	sb->st_nlink = (nlink_t)st->nlink;
	sb->st_uid = (uid_t)st->uid;
	sb->st_gid = (gid_t)st->gid;
	sb->st_rdev = (dev_t)st->rdev;
#if defined(__APPLE__)
	sb->st_atimespec.tv_sec = st->atime;
	sb->st_mtimespec.tv_sec = st->mtime;
	sb->st_ctimespec.tv_sec = st->ctime;
#else
	sb->st_atim.tv_sec = st->atime;
	sb->st_mtim.tv_sec = st->mtime;
	sb->st_ctim.tv_sec = st->ctime;
#endif
	sb->st_blksize = (blksize_t)st->blksize;
	sb->st_blocks = (blkcnt_t)st->blocks;
}

int rhp_csrc_stream_stat(php_stream *stream, php_stream_statbuf *ssb)
{
	struct rhp_fi_stat st;
	if (!H() || H()->stat(handle(stream), &st) != 0) {
		return -1;
	}
	fill_stat(&ssb->sb, &st);
	return 0;
}

int rhp_csrc_stream_stat_path(const char *path, php_stream_statbuf *ssb)
{
	struct rhp_fi_stat st;
	if (!H() || H()->stat_path(path, &st) != 0) {
		return FAILURE;
	}
	fill_stat(&ssb->sb, &st);
	return SUCCESS;
}

int rhp_csrc_stream_cast(php_stream *stream, int castas, void **ret)
{
	if (!H() || castas != PHP_STREAM_AS_FD) {
		return FAILURE;
	}
	int fd = H()->fd(handle(stream));
	if (fd < 0) {
		return FAILURE;
	}
	if (ret) {
		*(int *)ret = fd;
	}
	return SUCCESS;
}

php_stream_dirent *rhp_csrc_stream_readdir(php_stream *dirstream, php_stream_dirent *ent)
{
	if (!H()) {
		return NULL;
	}
	ssize_t n = H()->readdir(handle(dirstream), ent->d_name, sizeof(ent->d_name));
	if (n < 0) {
		return NULL;
	}
	ent->d_name[n] = '\0';
	ent->d_type = 0;
	return ent;
}

void rhp_csrc_error_docref(const char *docref, int type, const char *format, ...)
{
	(void)docref;
	char *msg;
	va_list ap;
	va_start(ap, format);
	size_t len = rhp_csrc_vspprintf(&msg, 0, format, ap);
	va_end(ap);
	if (H()) {
		H()->error(type, msg, len);
	}
	efree(msg);
}

_Noreturn void rhp_csrc_error_noreturn(int type, const char *format, ...)
{
	(void)type;
	(void)format;
	abort();
}

/* The zvals php_pcre_match_impl returns. */
zval *rhp_csrc_hash_index_find(const HashTable *ht, zend_ulong h)
{
	if (ht == NULL || h >= ht->nNumUsed) {
		return NULL;
	}
	return &ht->arPacked[h];
}

void rhp_csrc_zval_ptr_dtor(zval *zv)
{
	switch (zv->type) {
		case IS_STRING:
			zend_string_release(zv->value.str);
			break;
		case IS_ARRAY: {
			HashTable *ht = zv->value.arr;
			for (uint32_t i = 0; i < ht->nNumUsed; i++) {
				rhp_csrc_zval_ptr_dtor(&ht->arPacked[i]);
			}
			efree(ht->arPacked);
			efree(ht);
			break;
		}
		default:
			break;
	}
	zv->type = IS_NULL;
}

static HashTable *packed(uint32_t n)
{
	HashTable *ht = emalloc(sizeof(*ht));
	ht->nNumUsed = n;
	ht->arPacked = ecalloc(n ? n : 1, sizeof(zval));
	return ht;
}

pcre_cache_entry *rhp_csrc_pcre_get_compiled_regex_cache(zend_string *regex)
{
	return H() ? (pcre_cache_entry *)H()->pcre_compile(ZSTR_VAL(regex), ZSTR_LEN(regex)) : NULL;
}

int rhp_csrc_pcre2_match(const pcre2_code *code, PCRE2_SPTR subject, size_t length,
	size_t start_offset, uint32_t options, pcre2_match_data *match_data, pcre2_match_context *mcontext)
{
	(void)options;
	(void)match_data;
	(void)mcontext;
	return H()->pcre_test((void *)code, (const char *)subject, length, start_offset);
}

/* php_pcre_match_impl(pce, subject, return_value, subpats, 0,
 * PREG_OFFSET_CAPTURE, 0) as softmagic.c calls it: the match count, and
 * subpats[0] = [match, offset]. On an error php returns false, leaving the
 * long unset; here it reads as 0 (no match). libmagic reads no other group. */
void rhp_csrc_pcre_match_impl(pcre_cache_entry *pce, zend_string *subject_str, zval *return_value,
	zval *subpats, int global, zend_long flags, zend_long start_offset)
{
	(void)global;
	(void)flags;
	(void)start_offset;
	size_t start = 0, end = 0;
	int rc = H()->pcre_match(pce, ZSTR_VAL(subject_str), ZSTR_LEN(subject_str), &start, &end);
	subpats->type = IS_ARRAY;
	if (rc < 0) {
		return_value->type = IS_FALSE;
		return_value->value.lval = 0;
		subpats->value.arr = packed(0);
		return;
	}
	return_value->type = IS_LONG;
	return_value->value.lval = rc;
	if (rc == 0) {
		subpats->value.arr = packed(0);
		return;
	}
	HashTable *outer = packed(1);
	HashTable *pair = packed(2);
	pair->arPacked[0].type = IS_STRING;
	pair->arPacked[0].value.str = zend_string_init(ZSTR_VAL(subject_str) + start, end - start, 0);
	pair->arPacked[1].type = IS_LONG;
	pair->arPacked[1].value.lval = (zend_long)start;
	outer->arPacked[0].type = IS_ARRAY;
	outer->arPacked[0].value.arr = pair;
	subpats->value.arr = outer;
}

zend_string *rhp_csrc_pcre_replace_impl(pcre_cache_entry *pce, zend_string *subject_str, const char *subject,
	size_t subject_len, zend_string *replace_str, size_t limit, size_t *replace_count)
{
	(void)subject_str;
	size_t out_len = 0;
	char *out = H()->pcre_replace(pce, subject, subject_len, ZSTR_VAL(replace_str), ZSTR_LEN(replace_str),
		limit, replace_count, &out_len);
	if (out == NULL) {
		return NULL;
	}
	zend_string *res = zend_string_init(out, out_len, 0);
	efree(out);
	return res;
}

#ifdef RHP_FI_STRLCPY
size_t rhp_fi_strlcpy(char *dst, const char *src, size_t siz)
{
	size_t n = strlen(src);
	if (siz) {
		size_t c = n < siz - 1 ? n : siz - 1;
		memcpy(dst, src, c);
		dst[c] = '\0';
	}
	return n;
}

size_t rhp_fi_strlcat(char *dst, const char *src, size_t siz)
{
	size_t d = strnlen(dst, siz);
	if (d == siz) {
		return siz + strlen(src);
	}
	return d + rhp_fi_strlcpy(dst + d, src, siz - d);
}
#endif

/* Layouts and constants src/fileinfo mirrors. */
_Static_assert(MAGIC_MIME_TYPE == 0x10 && MAGIC_MIME_ENCODING == 0x400, "magic.h flags");
_Static_assert(MAGIC_VERSION == 546, "libmagic version");
_Static_assert(sizeof(struct rhp_fi_stat) == 4 + 4 + 12 * 8, "rhp_fi_stat layout");
