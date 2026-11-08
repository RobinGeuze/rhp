
/* ---- rhp's API (tail.c): php's createFromString/createFromFile bodies,
 * split so ext/dom/html5.rs can drive them. ---- */

typedef struct rhp_html5 {
	dom_lexbor_libxml2_bridge_application_data app;
	lexbor_libxml2_bridge_parse_context ctx;
	dom_decoding_encoding_ctx dec;
	lxb_html_document_t *document;
	lxb_html_parser_t *parser;
	size_t tokenizer_error_offset;
	size_t tree_error_offset;
	bool implicit;
	char *input_name;
	void *rust;
	char filebuf[4096];
} rhp_html5;

const char *rhp_html5_status_string(int status)
{
	return dom_lexbor_libxml2_bridge_status_code_to_string((lexbor_libxml2_bridge_status) status);
}

/* lxb_encoding_data_by_name: the encoding's canonical name, or NULL. */
const char *rhp_html5_encoding_name(const char *name, size_t len)
{
	const lxb_encoding_data_t *d = lxb_encoding_data_by_name((const lxb_char_t *) name, len);
	return d == NULL ? NULL : (const char *) d->name;
}

rhp_html5 *rhp_html5_new(const char *input_name, int no_implied, int report_errors, void *rust)
{
	rhp_html5 *h = calloc(1, sizeof(*h));
	if (h == NULL) {
		return NULL;
	}
	h->input_name = strdup(input_name);
	h->rust = rust;
	h->app.input_name = h->input_name;
	h->app.current_total_offset = 0;
	h->app.html_no_implied = no_implied;
	dom_reset_line_column_cache(&h->app.cache_tokenizer);
	lexbor_libxml2_bridge_parse_context_init(&h->ctx);
	if (report_errors) {
		lexbor_libxml2_bridge_parse_set_error_callbacks(
			&h->ctx,
			dom_lexbor_libxml2_bridge_tokenizer_error_reporter,
			dom_lexbor_libxml2_bridge_tree_error_reporter
		);
	}
	h->ctx.application_data = &h->app;
	dom_decoding_encoding_ctx_init(&h->dec);
	h->implicit = true;
	return h;
}

/* An override or sniffed encoding: false when lexbor doesn't know it. */
int rhp_html5_set_encoding(rhp_html5 *h, const char *buf_start, const char *name, size_t len)
{
	const lxb_encoding_data_t *encoding_data = lxb_encoding_data_by_name((const lxb_char_t *) name, len);
	if (!encoding_data) {
		return 0;
	}
	h->implicit = false;
	dom_setup_parser_encoding_manually((const lxb_char_t *) (buf_start ? buf_start : h->filebuf), encoding_data, &h->dec, &h->app);
	return 1;
}

int rhp_html5_begin(rhp_html5 *h)
{
	h->document = lxb_html_document_create();
	if (UNEXPECTED(h->document == NULL)) {
		return -1;
	}
	if (lxb_html_document_parse_chunk_begin(h->document) != LXB_STATUS_OK) {
		return -1;
	}
	h->parser = h->document->dom_document.parser;
	return 0;
}

/* createFromString's loop. */
int rhp_html5_parse_string(rhp_html5 *h, const char *source, size_t source_len)
{
	const lxb_char_t *buf_ref = (const lxb_char_t *) source;
	if (h->implicit) {
		dom_setup_parser_encoding_implicitly(&buf_ref, &source_len, &h->dec, &h->app);
		h->implicit = false;
	}
	if (rhp_html5_begin(h) != 0) {
		return -1;
	}
	rhp_html5_rust = h->rust;
	while (source_len > 0) {
		size_t chunk_size = source_len;
		const size_t MAX_CHUNK_SIZE = sizeof(h->dec.encoding_output) / sizeof(*h->dec.encoding_output);
		if (chunk_size > MAX_CHUNK_SIZE) {
			chunk_size = MAX_CHUNK_SIZE;
		}
		source_len -= chunk_size;
		const lxb_char_t *buf_end = buf_ref + chunk_size;
		if (!dom_parse_decode_encode_step(&h->ctx, h->document, h->parser, &buf_ref, buf_end, &h->dec, &h->tokenizer_error_offset, &h->tree_error_offset)) {
			return -1;
		}
		if (h->app.current_input_characters) {
			h->app.current_input_characters += chunk_size;
		}
	}
	return 0;
}

/* createFromFile reads into this buffer. */
char *rhp_html5_file_buf(rhp_html5 *h)
{
	return h->filebuf;
}

/* One step of createFromFile's read loop over `read` bytes of filebuf. */
int rhp_html5_parse_file_chunk(rhp_html5 *h, size_t read)
{
	if (h->document == NULL && rhp_html5_begin(h) != 0) {
		return -1;
	}
	rhp_html5_rust = h->rust;
	const lxb_char_t *buf_ref = (const lxb_char_t *) h->filebuf;
	if (h->implicit) {
		h->implicit = false;
		dom_setup_parser_encoding_implicitly(&buf_ref, &read, &h->dec, &h->app);
	}
	const lxb_char_t *buf_end = buf_ref + read;
	if (!dom_parse_decode_encode_step(&h->ctx, h->document, h->parser, &buf_ref, buf_end, &h->dec, &h->tokenizer_error_offset, &h->tree_error_offset)) {
		return -1;
	}
	return 0;
}

int rhp_html5_finish(rhp_html5 *h)
{
	if (h->document == NULL && rhp_html5_begin(h) != 0) {
		return -1;
	}
	rhp_html5_rust = h->rust;
	if (!dom_parse_decode_encode_finish(&h->ctx, h->document, h->parser, &h->dec, &h->tokenizer_error_offset, &h->tree_error_offset)) {
		return -1;
	}
	if (lxb_html_document_parse_chunk_end(h->document) != LXB_STATUS_OK) {
		return -1;
	}
	return 0;
}

/* Converts the parsed document; on success *out is the libxml2 document
 * with php's post-processing and encoding. Returns a bridge status. */
int rhp_html5_convert(rhp_html5 *h, long options, int compact, int create_default_ns, void *private_data, xmlDocPtr *out)
{
	xmlDocPtr lxml_doc;
	lexbor_libxml2_bridge_status status = lexbor_libxml2_bridge_convert_document(
		h->document, &lxml_doc, compact, create_default_ns, private_data);
	lexbor_libxml2_bridge_copy_observations(h->parser->tree, &h->ctx.observations);
	if (status != LEXBOR_LIBXML2_BRIDGE_STATUS_OK) {
		return status;
	}
	dom_post_process_html5_loading(lxml_doc, options, &h->ctx.observations);
	if (h->dec.decode_data) {
		lxml_doc->encoding = xmlStrdup((const xmlChar *) h->dec.decode_data->name);
	} else {
		lxml_doc->encoding = xmlStrdup((const xmlChar *) "UTF-8");
	}
	*out = lxml_doc;
	return 0;
}

int rhp_html5_quirks(rhp_html5 *h)
{
	return (int) h->ctx.observations.quirks_mode;
}

void rhp_html5_free(rhp_html5 *h)
{
	if (h == NULL) {
		return;
	}
	if (h->document != NULL) {
		lxb_html_document_destroy(h->document);
	}
	free(h->input_name);
	free(h);
}

typedef void (*rhp_html5_sink)(void *ctx, const char *buf, size_t len);

static int rhp_html5_sink_write(void *ctx, const char *buf, size_t len)
{
	void **pair = ctx;
	((rhp_html5_sink) pair[0])(pair[1], buf, len);
	return 0;
}

/* dom_common_save's encoding half: `utf8` (the serializer's output) in the
 * document's encoding, written to `sink`. */
void rhp_html5_encode_output(const char *encoding, const char *utf8, size_t len, rhp_html5_sink sink, void *sink_ctx)
{
	void *pair[2] = { (void *) sink, sink_ctx };
	const lxb_encoding_data_t *decoding_data = lxb_encoding_data(LXB_ENCODING_UTF_8);
	const lxb_encoding_data_t *encoding_data = lxb_encoding_data_by_name((const lxb_char_t *) encoding, strlen(encoding));
	if (encoding_data == NULL) {
		encoding_data = decoding_data;
	}
	lxb_encoding_encode_t encode;
	lxb_encoding_decode_t decode;
	lxb_char_t *encoding_output = malloc(4096);
	lxb_codepoint_t *codepoints = malloc(4096 * sizeof(lxb_codepoint_t));
	(void) lxb_encoding_encode_init(&encode, encoding_data, encoding_output, 4096);
	(void) lxb_encoding_decode_init(&decode, decoding_data, codepoints, 4096);
	if (encoding_data->encoding == LXB_ENCODING_UTF_8) {
		lxb_encoding_encode_replace_set(&encode, LXB_ENCODING_REPLACEMENT_BYTES, LXB_ENCODING_REPLACEMENT_SIZE);
	} else {
		lxb_encoding_encode_replace_set(&encode, (const lxb_char_t *) "?", 1);
	}
	lxb_encoding_decode_replace_set(&decode, LXB_ENCODING_REPLACEMENT_BUFFER, LXB_ENCODING_REPLACEMENT_BUFFER_LEN);

	dom_output_ctx output_ctx;
	output_ctx.encoding_data = encoding_data;
	output_ctx.decoding_data = decoding_data;
	output_ctx.encode = &encode;
	output_ctx.decode = &decode;
	output_ctx.codepoints = codepoints;
	output_ctx.encoding_output = encoding_output;
	output_ctx.output_data = pair;
	output_ctx.write_output = rhp_html5_sink_write;

	if (encoding_data->encoding == LXB_ENCODING_UTF_8) {
		dom_saveHTML_write_string_len_utf8_output(&output_ctx, utf8, len);
	} else {
		dom_saveHTML_write_string_len(&output_ctx, utf8, len);
	}

	(void) lxb_encoding_decode_finish(&decode);
	if (lxb_encoding_decode_buf_used(&decode)) {
		const lxb_codepoint_t *codepoints_ref = (const lxb_codepoint_t *) codepoints;
		(void) encoding_data->encode(&encode, &codepoints_ref, codepoints_ref + lxb_encoding_decode_buf_used(&decode));
		sink(sink_ctx, (const char *) encoding_output, lxb_encoding_encode_buf_used(&encode));
	}
	(void) lxb_encoding_encode_finish(&encode);
	if (lxb_encoding_encode_buf_used(&encode)) {
		sink(sink_ctx, (const char *) encoding_output, lxb_encoding_encode_buf_used(&encode));
	}
	free(encoding_output);
	free(codepoints);
}
