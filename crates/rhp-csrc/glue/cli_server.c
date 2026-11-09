/*
 * The built-in web server's C (sapi/cli): php_http_parser behind an opaque
 * parser and one event callback, and mime_type_map.h's table. The settings
 * forward every parser callback to the Rust side with its kind.
 */
#include <stddef.h>
#include "php_http_parser.h"
#include "mime_type_map.h"

/* Each event also carries the parser's method and HTTP version so far. */
typedef int (*rhp_http_event_fn)(void *ctx, int kind, const char *at, size_t len, int method, int version);

struct rhp_http_ctx {
	rhp_http_event_fn cb;
	void *ctx;
};

enum {
	EV_MESSAGE_BEGIN,
	EV_PATH,
	EV_QUERY_STRING,
	EV_URL,
	EV_FRAGMENT,
	EV_HEADER_FIELD,
	EV_HEADER_VALUE,
	EV_HEADERS_COMPLETE,
	EV_BODY,
	EV_MESSAGE_COMPLETE
};

static int fwd(php_http_parser *p, int kind, const char *at, size_t len)
{
	struct rhp_http_ctx *c = p->data;
	return c->cb(c->ctx, kind, at, len, p->method, p->http_major * 100 + p->http_minor);
}

static int on_message_begin(php_http_parser *p) { return fwd(p, EV_MESSAGE_BEGIN, NULL, 0); }
static int on_path(php_http_parser *p, const char *at, size_t len) { return fwd(p, EV_PATH, at, len); }
static int on_query_string(php_http_parser *p, const char *at, size_t len) { return fwd(p, EV_QUERY_STRING, at, len); }
static int on_url(php_http_parser *p, const char *at, size_t len) { return fwd(p, EV_URL, at, len); }
static int on_fragment(php_http_parser *p, const char *at, size_t len) { return fwd(p, EV_FRAGMENT, at, len); }
static int on_header_field(php_http_parser *p, const char *at, size_t len) { return fwd(p, EV_HEADER_FIELD, at, len); }
static int on_header_value(php_http_parser *p, const char *at, size_t len) { return fwd(p, EV_HEADER_VALUE, at, len); }
static int on_headers_complete(php_http_parser *p) { return fwd(p, EV_HEADERS_COMPLETE, NULL, 0); }
static int on_body(php_http_parser *p, const char *at, size_t len) { return fwd(p, EV_BODY, at, len); }
static int on_message_complete(php_http_parser *p) { return fwd(p, EV_MESSAGE_COMPLETE, NULL, 0); }

static const php_http_parser_settings settings = {
	on_message_begin,
	on_path,
	on_query_string,
	on_url,
	on_fragment,
	on_header_field,
	on_header_value,
	on_headers_complete,
	on_body,
	on_message_complete
};

size_t rhp_http_parser_size(void)
{
	return sizeof(php_http_parser);
}

void rhp_http_parser_init(php_http_parser *p)
{
	php_http_parser_init(p, PHP_HTTP_REQUEST);
}

size_t rhp_http_parser_execute(php_http_parser *p, rhp_http_event_fn cb, void *ctx, const char *data, size_t len)
{
	struct rhp_http_ctx c = { cb, ctx };
	size_t n;
	p->data = &c;
	n = php_http_parser_execute(p, &settings, data, len);
	p->data = NULL;
	return n;
}

int rhp_http_parser_method(const php_http_parser *p)
{
	return p->method;
}

int rhp_http_parser_version(const php_http_parser *p)
{
	return p->http_major * 100 + p->http_minor;
}

int rhp_http_parser_at_request_start(const php_http_parser *p)
{
	return p->state == s_start_req;
}

const char *rhp_http_method_name(int m)
{
	return php_http_method_str((enum php_http_method) m);
}

size_t rhp_mime_type_count(void)
{
	return sizeof(mime_type_map) / sizeof(mime_type_map[0]) - 1;
}

void rhp_mime_type_pair(size_t i, const char **ext, const char **mime)
{
	*ext = mime_type_map[i].ext;
	*mime = mime_type_map[i].mime_type;
}
