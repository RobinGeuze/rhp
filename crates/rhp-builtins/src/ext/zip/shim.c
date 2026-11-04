/*
 * ext/zip's C side: ZipArchive's class constants (consts.h) with the linked
 * libzip's values and guards, and the struct layouts the Rust side mirrors.
 * The macros php_zip.h and php's configure add around zip.h are repeated
 * here.
 */
#include <stddef.h>
#include <zip.h>

#if LIBZIP_VERSION_MAJOR < 1 || (LIBZIP_VERSION_MAJOR == 1 && LIBZIP_VERSION_MINOR < 7)
#error "rhp needs libzip 1.7.1 or later (php's configure accepts >= 0.11; rhp ports the newest API only)"
#endif
#if LIBZIP_VERSION_MAJOR == 1 && LIBZIP_VERSION_MINOR == 7 && LIBZIP_VERSION_MICRO == 0
#error "php's configure refuses libzip 1.7.0"
#endif

/* php's configure finds these functions in every libzip >= 1.7. */
#define HAVE_ENCRYPTION 1
#define HAVE_SET_MTIME 1
#define HAVE_LIBZIP_VERSION 1
#define HAVE_PROGRESS_CALLBACK 1
#define HAVE_CANCEL_CALLBACK 1
#define HAVE_METHOD_SUPPORTED 1

/* php_zip.h */
#ifndef ZIP_OVERWRITE
#define ZIP_OVERWRITE ZIP_TRUNCATE
#endif
#ifndef ZIP_LENGTH_TO_END
#define ZIP_LENGTH_TO_END 0
#endif
#define ZIP_FL_OPEN_FILE_NOW (1u << 30)
#define LIBZIP_VERSION_STR zip_libzip_version()
#define LIBZIP_ATLEAST(m, n, p) \
    (((m << 16) + (n << 8) + p) <= \
     ((LIBZIP_VERSION_MAJOR << 16) + (LIBZIP_VERSION_MINOR << 8) + LIBZIP_VERSION_MICRO))

typedef void (*rhp_zip_const_cb)(void *ctx, const char *name, long long ival, const char *sval,
                                 const char *attr);

/* Each of ZipArchive's constants that the linked zip.h defines, in the
 * stub's order: an int (sval NULL) or a string, and the PHP attribute text
 * to declare it with. */
void rhp_zip_constants(rhp_zip_const_cb cb, void *ctx)
{
#define RHP_ZIP_INT(name, value, attr) cb(ctx, name, (long long)(value), NULL, attr);
#define RHP_ZIP_STR(name, value, attr) cb(ctx, name, 0, (value), attr);
#include "consts.h"
#undef RHP_ZIP_INT
#undef RHP_ZIP_STR
}

/* zip_stream.c seeks an entry only with libzip >= 1.9.1. */
int rhp_zip_file_is_seekable(zip_file_t *zf)
{
#if LIBZIP_ATLEAST(1, 9, 1)
    return zip_file_is_seekable(zf);
#else
    (void)zf;
    return 0;
#endif
}

/* ZIP_FL_OPEN_FILE_NOW, which php adds and strips before calling libzip. */
unsigned int rhp_zip_fl_open_file_now(void) { return ZIP_FL_OPEN_FILE_NOW; }

/* Sizes and offsets of zip_stat_t and zip_error_t, for sys.rs's test. */
void rhp_zip_layout(size_t out[14])
{
    out[0] = sizeof(zip_stat_t);
    out[1] = offsetof(zip_stat_t, valid);
    out[2] = offsetof(zip_stat_t, name);
    out[3] = offsetof(zip_stat_t, index);
    out[4] = offsetof(zip_stat_t, size);
    out[5] = offsetof(zip_stat_t, comp_size);
    out[6] = offsetof(zip_stat_t, mtime);
    out[7] = offsetof(zip_stat_t, crc);
    out[8] = offsetof(zip_stat_t, comp_method);
    out[9] = offsetof(zip_stat_t, encryption_method);
    out[10] = sizeof(zip_error_t);
    out[11] = offsetof(zip_error_t, zip_err);
    out[12] = offsetof(zip_error_t, sys_err);
    out[13] = offsetof(zip_error_t, str);
}
