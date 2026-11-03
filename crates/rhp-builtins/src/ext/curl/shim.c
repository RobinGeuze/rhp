/* What ext/curl's Rust side cannot do portably: a stdio stream over a
 * callback (CURLOPT_STDERR takes a FILE *), and the struct layouts sys.rs
 * declares, for its layout test. */

#define _GNU_SOURCE
#include <stddef.h>
#include <stdio.h>
#include <sys/types.h>
#include <curl/curl.h>

/* ext/curl/easy.rs: writes `len` bytes to the stream behind `cookie`. */
extern long rhp_curl_stderr_write(void *cookie, const char *buf, size_t len);

#if defined(__APPLE__) || defined(__FreeBSD__)
static int cookie_write(void *cookie, const char *buf, int len)
{
	return (int) rhp_curl_stderr_write(cookie, buf, (size_t) len);
}

FILE *rhp_curl_cookie_file(void *cookie)
{
	FILE *f = funopen(cookie, NULL, cookie_write, NULL, NULL);
	if (f) {
		setvbuf(f, NULL, _IONBF, 0);
	}
	return f;
}
#else
static ssize_t cookie_write(void *cookie, const char *buf, size_t len)
{
	return (ssize_t) rhp_curl_stderr_write(cookie, buf, len);
}

FILE *rhp_curl_cookie_file(void *cookie)
{
	cookie_io_functions_t io = { NULL, cookie_write, NULL, NULL };
	FILE *f = fopencookie(cookie, "w", io);
	if (f) {
		setvbuf(f, NULL, _IONBF, 0);
	}
	return f;
}
#endif

/* The C library's stderr (a macro on some systems). */
FILE *rhp_curl_stderr(void)
{
	return stderr;
}

/* sys.rs rust_layout(), in the same order. */
const size_t rhp_curl_layout[] = {
	offsetof(curl_version_info_data, ssl_version_num),
	offsetof(curl_version_info_data, protocols),
	offsetof(curl_version_info_data, iconv_ver_num),
	offsetof(curl_version_info_data, brotli_version),
	sizeof(CURLMsg),
	offsetof(CURLMsg, data),
	sizeof(struct curl_blob),
	sizeof(struct curl_certinfo),
	sizeof(struct curl_slist),
};

const unsigned long rhp_curl_layout_count = sizeof(rhp_curl_layout) / sizeof(rhp_curl_layout[0]);
