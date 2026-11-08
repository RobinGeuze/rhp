
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

/* ---- fragment parsing (inner_outer_html_mixin.c) ---- */

static lxb_dom_document_cmode_t dom_translate_quirks_mode_back(int quirks_mode)
{
	switch (quirks_mode) {
		case PHP_LIBXML_LIMITED_QUIRKS: return LXB_DOM_DOCUMENT_CMODE_LIMITED_QUIRKS;
		case PHP_LIBXML_QUIRKS: return LXB_DOM_DOCUMENT_CMODE_QUIRKS;
		default: return LXB_DOM_DOCUMENT_CMODE_NO_QUIRKS;
	}
}

/* dom_html_fragment_lexbor_parse (php's slow path, which also covers
 * valid UTF-8). */
static lxb_dom_node_t *rhp_html_fragment_lexbor_parse(lxb_html_document_t *document, lxb_dom_element_t *element, const char *input, size_t input_len)
{
	lxb_status_t status = lxb_html_document_parse_fragment_chunk_begin(document, element);
	if (status != LXB_STATUS_OK) {
		return NULL;
	}

	const lxb_encoding_data_t *encoding_data = lxb_encoding_data(LXB_ENCODING_UTF_8);
	lxb_encoding_decode_t decode;
	lxb_encoding_decode_init_single(&decode, encoding_data);

	const lxb_char_t *buf_ref = (const lxb_char_t *) input;
	const lxb_char_t *buf_end = buf_ref + input_len;
	const lxb_char_t *last_output = buf_ref;
	while (buf_ref < buf_end) {
		if (decode.u.utf_8.need == 0 && *buf_ref < 0x80) {
			buf_ref++;
			continue;
		}

		const lxb_char_t *buf_ref_backup = buf_ref;
		lxb_codepoint_t codepoint = lxb_encoding_decode_utf_8_single(&decode, &buf_ref, buf_end);
		if (UNEXPECTED(codepoint > LXB_ENCODING_MAX_CODEPOINT)) {
			status = lxb_html_document_parse_fragment_chunk(document, last_output, buf_ref_backup - last_output);
			if (UNEXPECTED(status != LXB_STATUS_OK)) {
				return NULL;
			}

			status = lxb_html_document_parse_fragment_chunk(document, LXB_ENCODING_REPLACEMENT_BYTES, LXB_ENCODING_REPLACEMENT_SIZE);
			if (UNEXPECTED(status != LXB_STATUS_OK)) {
				return NULL;
			}

			last_output = buf_ref;
		}
	}

	if (buf_ref != last_output) {
		status = lxb_html_document_parse_fragment_chunk(document, last_output, buf_ref - last_output);
		if (UNEXPECTED(status != LXB_STATUS_OK)) {
			return NULL;
		}
	}

	return lxb_html_document_parse_fragment_chunk_end(document);
}

/* dom_html_fragment_parsing_algorithm: the fragment in *out; 0, -1 when
 * lexbor failed, or a bridge status. */
int rhp_html5_parse_fragment(void *private_data, xmlNodePtr context_node, const char *input, size_t input_len, int quirks_mode, xmlNodePtr *out)
{
	lxb_html_document_t *document = lxb_html_document_create();
	document->dom_document.compat_mode = dom_translate_quirks_mode_back(quirks_mode);
	lxb_dom_element_t *element = lxb_dom_element_interface_create(&document->dom_document);

	const lxb_tag_data_t *tag_data = lxb_tag_data_by_name(document->dom_document.tags, (lxb_char_t *) context_node->name, xmlStrlen(context_node->name));
	element->node.local_name = tag_data == NULL ? LXB_TAG__UNDEF : tag_data->tag_id;

	const lxb_char_t *ns_uri;
	size_t ns_uri_len;
	if (context_node->ns == NULL || context_node->ns->href == NULL) {
		ns_uri = (lxb_char_t *) "";
		ns_uri_len = 0;
	} else {
		ns_uri = context_node->ns->href;
		ns_uri_len = xmlStrlen(ns_uri);
	}
	const lxb_ns_data_t *ns_data = lxb_ns_data_by_link(document->dom_document.ns, ns_uri, ns_uri_len);
	element->node.ns = ns_data == NULL ? LXB_NS__UNDEF : ns_data->ns_id;

	int ret;
	lxb_dom_node_t *node = rhp_html_fragment_lexbor_parse(document, element, input, input_len);
	*out = NULL;
	if (node != NULL) {
		ret = lexbor_libxml2_bridge_convert_fragment(node->last_child, context_node->doc, out, true, true, private_data);
	} else {
		ret = -1;
	}

	lxb_html_document_destroy(document);
	return ret;
}

extern void rhp_xml_sanitize_parse_ctxt_options(xmlParserCtxtPtr ctxt);

static void rhp_xml_parser_tag_name(const xmlNode *context_node, xmlParserCtxtPtr parser)
{
	if (context_node->ns != NULL && context_node->ns->prefix != NULL) {
		xmlParseChunk(parser, (const char *) context_node->ns->prefix, xmlStrlen(context_node->ns->prefix), 0);
		xmlParseChunk(parser, ":", 1, 0);
	}

	xmlParseChunk(parser, (const char *) context_node->name, xmlStrlen(context_node->name), 0);
}

/* dom_xml_fragment_parsing_algorithm's parse: `decls` are the in-scope
 * namespace declarations as attribute text (" xmlns:p=\"uri\"..."). The
 * parsed document (sharing the context document's dict) in *out; returns
 * whether it is well-formed. */
int rhp_dom_xml_parse_fragment(xmlNodePtr context_node, const char *decls, size_t decls_len, const char *input, size_t input_len, xmlDocPtr *out)
{
	*out = NULL;
	xmlParserCtxtPtr parser = xmlCreatePushParserCtxt(NULL, NULL, NULL, 0, NULL);
	if (UNEXPECTED(parser == NULL)) {
		return -1;
	}

	xmlDictFree(parser->dict);
	if (context_node->doc->dict == NULL) {
		context_node->doc->dict = xmlDictCreate();
		xmlDictSetLimit(context_node->doc->dict, XML_MAX_DICTIONARY_LIMIT);
	}
	parser->dict = context_node->doc->dict;

	rhp_xml_sanitize_parse_ctxt_options(parser);
	xmlCtxtUseOptions(parser, XML_PARSE_IGNORE_ENC | XML_PARSE_NOERROR | XML_PARSE_NOWARNING);

	xmlCharEncodingHandlerPtr encoding = xmlFindCharEncodingHandler("UTF-8");
	(void) xmlSwitchToEncoding(parser, encoding);

	xmlParseChunk(parser, "<", 1, 0);
	rhp_xml_parser_tag_name(context_node, parser);
	xmlParseChunk(parser, decls, (int) decls_len, 0);
	xmlParseChunk(parser, ">", 1, 0);
	xmlParseChunk(parser, input, (int) input_len, 0);
	xmlParseChunk(parser, "</", 2, 0);
	rhp_xml_parser_tag_name(context_node, parser);
	xmlParseChunk(parser, ">", 1, 1);

	if (!parser->wellFormed || !parser->nsWellFormed) {
		parser->dict = NULL;
		xmlFreeDoc(parser->myDoc);
		xmlFreeParserCtxt(parser);
		return 0;
	}

	*out = parser->myDoc;
	xmlFreeParserCtxt(parser);
	return 1;
}

/* ---- CSS selectors (parentnode/css_selectors.c) ---- */

typedef void (*rhp_css_sink)(void *ctx, const xmlNode *node);

typedef struct {
	rhp_css_sink sink;
	void *ctx;
	const xmlNode *first;
	const xmlNode *reference;
	bool result;
} rhp_css_ctx;

static lxb_status_t rhp_css_single_cb(const xmlNode *node, lxb_css_selector_specificity_t spec, void *ctx)
{
	((rhp_css_ctx *) ctx)->first = node;
	return LXB_STATUS_STOP;
}

static lxb_status_t rhp_css_array_cb(const xmlNode *node, lxb_css_selector_specificity_t spec, void *ctx)
{
	rhp_css_ctx *c = ctx;
	c->sink(c->ctx, node);
	return LXB_STATUS_OK;
}

static lxb_status_t rhp_css_matches_cb(const xmlNode *node, lxb_css_selector_specificity_t spec, void *ctx)
{
	rhp_css_ctx *c = ctx;
	if (node == c->reference) {
		c->result = true;
		return LXB_STATUS_STOP;
	}
	return LXB_STATUS_OK;
}

/* Results of rhp_dom_css. */
#define RHP_CSS_OK 0
#define RHP_CSS_SYNTAX 1
#define RHP_CSS_UNSUPPORTED 2
#define RHP_CSS_BLANK 3

/* mode 0 querySelector, 1 querySelectorAll (nodes to sink), 2 matches,
 * 3 closest. *out is the node found (0, 3) or non-NULL for a match (2).
 * A syntax error's message (malloc'd) in *err. */
int rhp_dom_css(const xmlNode *root, const char *sel, size_t sel_len, int mode, int quirks,
	rhp_css_sink sink, void *sink_ctx, const xmlNode **out, char **err)
{
	lxb_css_parser_t parser;
	lxb_selectors_t selectors;
	lxb_status_t status;
	int ret = RHP_CSS_OK;
	*out = NULL;
	*err = NULL;
	rhp_css_blank = 0;

	memset(&parser, 0, sizeof(lxb_css_parser_t));
	(void) lxb_css_parser_init(&parser, NULL);
	memset(&selectors, 0, sizeof(lxb_selectors_t));
	(void) lxb_selectors_init(&selectors);
	lxb_selectors_opt_t options = mode == 1 ? LXB_SELECTORS_OPT_DEFAULT : LXB_SELECTORS_OPT_MATCH_FIRST;
	if (quirks) {
		options |= LXB_SELECTORS_OPT_QUIRKS_MODE;
	}
	lxb_selectors_opt_set(&selectors, options);

	lxb_css_selector_list_t *list = lxb_css_selectors_parse(&parser, (const lxb_char_t *) sel, sel_len);
	if (UNEXPECTED(list == NULL)) {
		size_t nr_of_messages = lexbor_array_obj_length(&parser.log->messages);
		if (nr_of_messages > 0) {
			lxb_css_log_message_t *msg = lexbor_array_obj_get(&parser.log->messages, 0);
			if (asprintf(err, "Invalid selector (%.*s)", (int) msg->text.length, msg->text.data) < 0) {
				*err = NULL;
			}
		}
		ret = RHP_CSS_SYNTAX;
	} else {
		rhp_css_ctx c = { sink, sink_ctx, NULL, NULL, false };
		switch (mode) {
			case 0:
				status = lxb_selectors_find(&selectors, root, list, rhp_css_single_cb, &c);
				*out = c.first;
				break;
			case 1:
				status = lxb_selectors_find(&selectors, root, list, rhp_css_array_cb, &c);
				break;
			case 2:
				c.reference = root;
				status = lxb_selectors_match_node(&selectors, root, list, rhp_css_matches_cb, &c);
				*out = c.result ? root : NULL;
				break;
			default: {
				status = LXB_STATUS_OK;
				const xmlNode *current = root;
				while (current != NULL) {
					c.reference = current;
					c.result = false;
					status = lxb_selectors_match_node(&selectors, current, list, rhp_css_matches_cb, &c);
					if (status != LXB_STATUS_OK && status != LXB_STATUS_STOP) {
						break;
					}
					status = LXB_STATUS_OK;
					if (c.result) {
						*out = current;
						break;
					}
					current = current->parent;
				}
				break;
			}
		}
		if (rhp_css_blank) {
			ret = RHP_CSS_BLANK;
		} else if (status != LXB_STATUS_OK && status != LXB_STATUS_STOP) {
			ret = RHP_CSS_UNSUPPORTED;
		}
	}

	lxb_css_selector_list_destroy_memory(list);
	lxb_selectors_destroy(&selectors);
	(void) lxb_css_parser_destroy(&parser, false);
	return ret;
}
