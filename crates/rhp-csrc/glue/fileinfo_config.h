/*
 * The php_config.h macros libmagic reads, as php's configure sets them on
 * macOS and glibc (Homebrew's and Debian's builds). Force-included before
 * prefix_fileinfo.h. php compiles libmagic without PHP_FILEINFO_UNCOMPRESS,
 * so there is no decompression and nothing is spawned.
 */
#ifndef RHP_FILEINFO_CONFIG_H
#define RHP_FILEINFO_CONFIG_H

#define HAVE_UNISTD_H 1
#define HAVE_INTTYPES_H 1
#define HAVE_STDINT_H 1
#define HAVE_DIRENT_H 1
#define HAVE_SYS_TIME_H 1
#define HAVE_SYS_MMAN_H 1
#define HAVE_SYS_IOCTL_H 1
#define HAVE_SYS_WAIT_H 1
#define HAVE_UTIME_H 1
#define HAVE_WCHAR_H 1
#define HAVE_ASCTIME_R 1
#define HAVE_CTIME_R 1
#define HAVE_FORK 1
#define HAVE_MEMMEM 1
#define HAVE_MKSTEMP 1
#define HAVE_MMAP 1
#define HAVE_STRCASESTR 1
#define HAVE_STRNDUP 1
#define HAVE_UTIME 1
#define HAVE_UTIMES 1
#define HAVE_STRUCT_TM_TM_GMTOFF 1
#define HAVE_STRUCT_TM_TM_ZONE 1
#define HAVE_VASPRINTF 1
#define HAVE_ASPRINTF 1
#define SIZEOF_LONG_LONG 8
#define HAVE_STRLCPY 1
#define HAVE_STRLCAT 1
#if !defined(__APPLE__)
# define HAVE_SYS_SYSMACROS_H 1
/* glibc has strlcpy from 2.38; before that php brings its own (main/strlcpy.c,
 * main/strlcat.c), which glue/fileinfo.c stands in for. */
# include <features.h>
# if !__GLIBC_PREREQ(2, 38)
#  include <stddef.h>
size_t rhp_fi_strlcpy(char *dst, const char *src, size_t siz);
size_t rhp_fi_strlcat(char *dst, const char *src, size_t siz);
#  define strlcpy rhp_fi_strlcpy
#  define strlcat rhp_fi_strlcat
#  define RHP_FI_STRLCPY 1
# endif
#endif

#endif
