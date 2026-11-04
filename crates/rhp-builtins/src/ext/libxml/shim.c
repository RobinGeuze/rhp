/*
 * What rhp's libxml2 binding cannot do from Rust: header macros, the
 * printf-style error callbacks, and access to structs whose layout differs
 * between libxml2 releases (xmlParserCtxt, xmlParserInput, xmlEntity and
 * xmlAttr grew between 2.9.13 and 2.15.2). Compiled against the headers of
 * the libxml2 rhp links, so every field access matches that library. No
 * logic lives here; see sys.rs for the Rust side.
 */

#define _GNU_SOURCE /* vasprintf */
#include <stdarg.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <libxml/xmlversion.h>
#include <libxml/parser.h>
#include <libxml/parserInternals.h>
#include <libxml/tree.h>
#include <libxml/entities.h>
#include <libxml/xmlerror.h>
#include <libxml/xmlIO.h>
#include <libxml/uri.h>
#include <libxml/dict.h>
#include <libxml/globals.h>
#include <libxml/xmlwriter.h>
#include <libxml/HTMLparser.h>
#include <libxml/xmlsave.h>
#include <libxml/valid.h>
#include <libxml/xpath.h>
#include <libxml/xpathInternals.h>
#if defined(LIBXML_SCHEMAS_ENABLED) || defined(LIBXML_RELAXNG_ENABLED)
#include <libxml/relaxng.h>
#endif
#ifdef LIBXML_SCHEMAS_ENABLED
#include <libxml/xmlschemas.h>
#include <libxml/xmlschemastypes.h>
#endif

/* The node header every tree struct shares with xmlNode. Code that walks a
 * tree reads these fields through xmlNode, whatever the node's real type. */
#define RHP_SAME_HEAD(T)                                                  \
	_Static_assert(offsetof(T, _private) == offsetof(xmlNode, _private), #T); \
	_Static_assert(offsetof(T, type) == offsetof(xmlNode, type), #T);         \
	_Static_assert(offsetof(T, name) == offsetof(xmlNode, name), #T);         \
	_Static_assert(offsetof(T, children) == offsetof(xmlNode, children), #T); \
	_Static_assert(offsetof(T, last) == offsetof(xmlNode, last), #T);         \
	_Static_assert(offsetof(T, parent) == offsetof(xmlNode, parent), #T);     \
	_Static_assert(offsetof(T, next) == offsetof(xmlNode, next), #T);         \
	_Static_assert(offsetof(T, prev) == offsetof(xmlNode, prev), #T);         \
	_Static_assert(offsetof(T, doc) == offsetof(xmlNode, doc), #T)
RHP_SAME_HEAD(xmlAttr);
RHP_SAME_HEAD(xmlEntity);
RHP_SAME_HEAD(xmlDtd);
RHP_SAME_HEAD(xmlDoc);

/* sys.rs mirrors xmlError (unchanged from 2.9 to 2.15) as XmlErrorRaw;
 * its test compares the mirror with this table. */
const size_t rhp_xml_error_layout[] = {
	sizeof(xmlError),
	offsetof(xmlError, domain),
	offsetof(xmlError, code),
	offsetof(xmlError, message),
	offsetof(xmlError, level),
	offsetof(xmlError, file),
	offsetof(xmlError, line),
	offsetof(xmlError, str1),
	offsetof(xmlError, str2),
	offsetof(xmlError, str3),
	offsetof(xmlError, int1),
	offsetof(xmlError, int2),
	offsetof(xmlError, ctxt),
	offsetof(xmlError, node),
};

/* ---- versions (php: header macros, and xmlParserVersion at run time) ---- */

int rhp_xml_header_version(void) { return LIBXML_VERSION; }
const char *rhp_xml_header_dotted_version(void) { return LIBXML_DOTTED_VERSION; }
const char *rhp_xml_loaded_version(void) { return xmlParserVersion; }

/* ---- the LIBXML_* constants, as libxml.stub.php's @cvalue names them ---- */

struct rhp_xml_const {
	const char *name;
	long long value;
};

static const struct rhp_xml_const rhp_xml_consts[] = {
	{"LIBXML_RECOVER", XML_PARSE_RECOVER},
	{"LIBXML_NOENT", XML_PARSE_NOENT},
#if LIBXML_VERSION >= 21300
	{"LIBXML_NO_XXE", XML_PARSE_NO_XXE},
#endif
	{"LIBXML_DTDLOAD", XML_PARSE_DTDLOAD},
	{"LIBXML_DTDATTR", XML_PARSE_DTDATTR},
	{"LIBXML_DTDVALID", XML_PARSE_DTDVALID},
	{"LIBXML_NOERROR", XML_PARSE_NOERROR},
	{"LIBXML_NOWARNING", XML_PARSE_NOWARNING},
	{"LIBXML_NOBLANKS", XML_PARSE_NOBLANKS},
	{"LIBXML_XINCLUDE", XML_PARSE_XINCLUDE},
	{"LIBXML_NSCLEAN", XML_PARSE_NSCLEAN},
	{"LIBXML_NOCDATA", XML_PARSE_NOCDATA},
	{"LIBXML_NONET", XML_PARSE_NONET},
	{"LIBXML_PEDANTIC", XML_PARSE_PEDANTIC},
	{"LIBXML_COMPACT", XML_PARSE_COMPACT},
	{"LIBXML_NOXMLDECL", XML_SAVE_NO_DECL},
	{"LIBXML_PARSEHUGE", XML_PARSE_HUGE},
	{"LIBXML_BIGLINES", XML_PARSE_BIG_LINES},
	{"LIBXML_NOEMPTYTAG", 1 << 2}, /* php_libxml.h LIBXML_SAVE_NOEMPTYTAG */
#ifdef LIBXML_SCHEMAS_ENABLED
	{"LIBXML_SCHEMA_CREATE", XML_SCHEMA_VAL_VC_I_CREATE},
#endif
	{"LIBXML_HTML_NOIMPLIED", HTML_PARSE_NOIMPLIED},
	{"LIBXML_HTML_NODEFDTD", HTML_PARSE_NODEFDTD},
	{"LIBXML_ERR_NONE", XML_ERR_NONE},
	{"LIBXML_ERR_WARNING", XML_ERR_WARNING},
	{"LIBXML_ERR_ERROR", XML_ERR_ERROR},
	{"LIBXML_ERR_FATAL", XML_ERR_FATAL},
	{NULL, 0},
};

const struct rhp_xml_const *rhp_xml_constants(void) { return rhp_xml_consts; }

/* ---- initialisation (php_libxml_initialize) ---- */

void rhp_xml_process_init(void)
{
	xmlInitParser();
	/* php does these under ZTS; rhp runs one VM per thread. */
#ifdef LIBXML_SCHEMAS_ENABLED
	xmlSchemaInitTypes();
#endif
#ifdef LIBXML_RELAXNG_ENABLED
	xmlRelaxNGInitTypes();
#endif
}

/* ---- printf-style callbacks: format without a size limit (php uses
 * vspprintf) and hand Rust the finished message ---- */

/* php_libxml_error_level */
#define RHP_LIBXML_ERROR 0
#define RHP_LIBXML_CTX_ERROR 1
#define RHP_LIBXML_CTX_WARNING 2

typedef void (*rhp_xml_msg_sink)(int kind, void *ctx, const char *msg, size_t len);
static rhp_xml_msg_sink rhp_msg_sink;

void rhp_xml_set_msg_sink(rhp_xml_msg_sink sink) { rhp_msg_sink = sink; }

static void rhp_deliver(int kind, void *ctx, const char *fmt, va_list ap)
{
	char *buf = NULL;
	int len = vasprintf(&buf, fmt, ap);
	if (len < 0 || buf == NULL) {
		return;
	}
	if (rhp_msg_sink != NULL) {
		rhp_msg_sink(kind, ctx, buf, (size_t) len);
	}
	free(buf);
}

/* php_libxml_error_handler */
static void rhp_xml_generic_error(void *ctx, const char *msg, ...)
{
	va_list ap;
	va_start(ap, msg);
	rhp_deliver(RHP_LIBXML_ERROR, ctx, msg, ap);
	va_end(ap);
}

/* php_libxml_ctx_error */
static void rhp_xml_ctx_error(void *ctx, const char *msg, ...)
{
	va_list ap;
	va_start(ap, msg);
	rhp_deliver(RHP_LIBXML_CTX_ERROR, ctx, msg, ap);
	va_end(ap);
}

/* php_libxml_ctx_warning */
static void rhp_xml_ctx_warning(void *ctx, const char *msg, ...)
{
	va_list ap;
	va_start(ap, msg);
	rhp_deliver(RHP_LIBXML_CTX_WARNING, ctx, msg, ap);
	va_end(ap);
}

/* The per-thread generic error channel, as php's RINIT/post_deactivate set
 * it: on (php's handler) or off (libxml2's default, stderr). */
void rhp_xml_set_generic_error(int on)
{
	xmlSetGenericErrorFunc(NULL, on ? rhp_xml_generic_error : NULL);
}

/* Route a parser context's error and warning channels to php's ctx handlers
 * (what ext/dom and ext/simplexml do before parsing). */
void rhp_xml_ctxt_use_php_errors(xmlParserCtxtPtr ctxt)
{
	ctxt->vctxt.error = rhp_xml_ctx_error;
	ctxt->vctxt.warning = rhp_xml_ctx_warning;
	if (ctxt->sax != NULL) {
		ctxt->sax->error = rhp_xml_ctx_error;
		ctxt->sax->warning = rhp_xml_ctx_warning;
	}
}

/* ---- parser context fields ---- */

/* php_libxml_ctx_error_level reads parser->input->filename, and
 * php_libxml_error_handler_va its line and col. 0 when there is no input. */
int rhp_xml_ctxt_input_pos(xmlParserCtxtPtr ctxt, const char **filename, int *line, int *col)
{
	if (ctxt == NULL || ctxt->input == NULL) {
		return 0;
	}
	*filename = ctxt->input->filename;
	*line = ctxt->input->line;
	*col = ctxt->input->col;
	return 1;
}

/* compat.c XML_GetCurrentByteIndex */
long rhp_xml_ctxt_byte_index(xmlParserCtxtPtr ctxt)
{
	return (long) (ctxt->input->consumed + (ctxt->input->cur - ctxt->input->base));
}

const xmlChar *rhp_xml_ctxt_input_cur(xmlParserCtxtPtr ctxt, const xmlChar **base)
{
	*base = ctxt->input->base;
	return ctxt->input->cur;
}

int rhp_xml_ctxt_errno(xmlParserCtxtPtr ctxt) { return ctxt->errNo; }
void rhp_xml_ctxt_set_errno(xmlParserCtxtPtr ctxt, int err) { ctxt->errNo = err; }
int rhp_xml_ctxt_in_subset(xmlParserCtxtPtr ctxt) { return ctxt->inSubset; }
int rhp_xml_ctxt_in_content(xmlParserCtxtPtr ctxt) { return ctxt->instate == XML_PARSER_CONTENT; }
xmlDocPtr rhp_xml_ctxt_doc(xmlParserCtxtPtr ctxt) { return ctxt->myDoc; }
void rhp_xml_ctxt_set_well_formed(xmlParserCtxtPtr ctxt, int v) { ctxt->wellFormed = v; }
void rhp_xml_ctxt_set_sax_initialized(xmlParserCtxtPtr ctxt, unsigned int v) { ctxt->sax->initialized = v; }

/* The ctxt fields php's external entity loader passes to the user resolver. */
void rhp_xml_ctxt_loader_info(xmlParserCtxtPtr ctxt, const char **directory, const xmlChar **int_sub_name,
		const xmlChar **ext_sub_uri, const xmlChar **ext_sub_system)
{
	*directory = ctxt->directory;
	*int_sub_name = ctxt->intSubName;
	*ext_sub_uri = ctxt->extSubURI;
	*ext_sub_system = ctxt->extSubSystem;
}

/* xml.c xml_parse_helper: XML_PARSE_HUGE and the dictionary limit. */
void rhp_xml_ctxt_set_huge(xmlParserCtxtPtr ctxt, int huge)
{
	if (huge) {
		ctxt->options |= XML_PARSE_HUGE;
		xmlDictSetLimit(ctxt->dict, 0);
	} else {
		ctxt->options &= ~XML_PARSE_HUGE;
		xmlDictSetLimit(ctxt->dict, XML_MAX_DICTIONARY_LIMIT);
	}
}

/* php_libxml.h php_libxml_sanitize_parse_ctxt_options */
void rhp_xml_sanitize_parse_ctxt_options(xmlParserCtxtPtr ctxt)
{
	ctxt->loadsubset = 0;
	ctxt->validate = 0;
	ctxt->pedantic = 0;
	ctxt->replaceEntities = 0;
	ctxt->linenumbers = 0;
	ctxt->keepBlanks = 1;
	ctxt->options = 0;
}

/* compat.c XML_ParserFree: the document a push parser may have built. */
void rhp_xml_ctxt_free_doc(xmlParserCtxtPtr ctxt)
{
	if (ctxt->myDoc) {
		xmlFreeDoc(ctxt->myDoc);
		ctxt->myDoc = NULL;
	}
}

/* ---- ext/xml's SAX handlers (compat.c php_xml_compat_handlers) ---- */

struct rhp_xml_sax {
	getEntitySAXFunc get_entity;
	notationDeclSAXFunc notation_decl;
	unparsedEntityDeclSAXFunc unparsed_entity_decl;
	startElementSAXFunc start_element;
	endElementSAXFunc end_element;
	charactersSAXFunc characters;
	processingInstructionSAXFunc processing_instruction;
	commentSAXFunc comment;
	startElementNsSAX2Func start_element_ns;
	endElementNsSAX2Func end_element_ns;
};

/* xmlCreatePushParserCtxt copies the handler table. */
xmlParserCtxtPtr rhp_xml_compat_push_parser(const struct rhp_xml_sax *fns, void *user)
{
	xmlSAXHandler sax;
	memset(&sax, 0, sizeof(sax));
	sax.getEntity = fns->get_entity;
	sax.notationDecl = fns->notation_decl;
	sax.unparsedEntityDecl = fns->unparsed_entity_decl;
	sax.startElement = fns->start_element;
	sax.endElement = fns->end_element;
	sax.characters = fns->characters;
	sax.processingInstruction = fns->processing_instruction;
	sax.comment = fns->comment;
	sax.cdataBlock = fns->characters;
	sax.initialized = XML_SAX2_MAGIC;
	sax.startElementNs = fns->start_element_ns;
	sax.endElementNs = fns->end_element_ns;
	return xmlCreatePushParserCtxt(&sax, user, NULL, 0, NULL);
}

/* ---- entities ---- */

int rhp_xml_entity_fields(xmlEntityPtr ent, const xmlChar **name, const xmlChar **content,
		const xmlChar **system_id, const xmlChar **external_id)
{
	*name = ent->name;
	*content = ent->content;
	*system_id = ent->SystemID;
	*external_id = ent->ExternalID;
	return (int) ent->etype;
}

/* ---- I/O ---- */

/* An input stream over read/close callbacks, for the entity loader
 * (libxml.c php_libxml_external_entity_loader). On failure the close
 * callback has run. */
xmlParserInputPtr rhp_xml_input_from_io(xmlParserCtxtPtr ctxt, xmlInputReadCallback read_cb,
		xmlInputCloseCallback close_cb, void *context)
{
#if LIBXML_VERSION < 21400
	xmlParserInputBufferPtr pib = xmlAllocParserInputBuffer(XML_CHAR_ENCODING_NONE);
	if (pib == NULL) {
		close_cb(context);
		return NULL;
	}
	pib->context = context;
	pib->readcallback = read_cb;
	pib->closecallback = close_cb;
	xmlParserInputPtr ret = xmlNewIOInputStream(ctxt, pib, XML_CHAR_ENCODING_NONE);
	if (ret == NULL) {
		xmlFreeParserInputBuffer(pib);
	}
	return ret;
#else
	(void) ctxt;
	return xmlNewInputFromIO(NULL, read_cb, close_cb, context, 0);
#endif
}

/* php_libxml_output_buffer_create_filename's error path: before 2.14.4
 * the hook owns the encoder it was given. */
void rhp_xml_output_hook_failed(xmlCharEncodingHandlerPtr encoder)
{
#if LIBXML_VERSION < 21404
	xmlCharEncCloseFunc(encoder);
#else
	(void) encoder;
#endif
}

/* ---- misc ---- */

void rhp_xml_free(void *ptr) { xmlFree(ptr); }
void *rhp_xml_malloc(size_t size) { return xmlMalloc(size); }

const char *rhp_xml_uri_scheme(xmlURIPtr uri) { return uri->scheme; }
const char *rhp_xml_uri_opaque(xmlURIPtr uri) { return uri->opaque; }

/* ---- the tree structs sys.rs mirrors: its test compares this table ---- */

const size_t rhp_xml_tree_layout[64] = {
	sizeof(xmlNode),
	offsetof(xmlNode, _private),
	offsetof(xmlNode, type),
	offsetof(xmlNode, name),
	offsetof(xmlNode, children),
	offsetof(xmlNode, last),
	offsetof(xmlNode, parent),
	offsetof(xmlNode, next),
	offsetof(xmlNode, prev),
	offsetof(xmlNode, doc),
	offsetof(xmlNode, ns),
	offsetof(xmlNode, content),
	offsetof(xmlNode, properties),
	offsetof(xmlNode, nsDef),
	offsetof(xmlNode, psvi),
	offsetof(xmlNode, line),
	offsetof(xmlNode, extra),
	offsetof(xmlAttr, next),
	offsetof(xmlAttr, doc),
	offsetof(xmlAttr, ns),
	offsetof(xmlAttr, atype),
	offsetof(xmlAttr, psvi),
	sizeof(xmlNs),
	offsetof(xmlNs, next),
	offsetof(xmlNs, type),
	offsetof(xmlNs, href),
	offsetof(xmlNs, prefix),
	offsetof(xmlNs, _private),
	offsetof(xmlNs, context),
	sizeof(xmlDoc),
	offsetof(xmlDoc, compression),
	offsetof(xmlDoc, standalone),
	offsetof(xmlDoc, intSubset),
	offsetof(xmlDoc, extSubset),
	offsetof(xmlDoc, oldNs),
	offsetof(xmlDoc, version),
	offsetof(xmlDoc, encoding),
	offsetof(xmlDoc, ids),
	offsetof(xmlDoc, URL),
	offsetof(xmlDoc, dict),
	offsetof(xmlDoc, parseFlags),
	offsetof(xmlDoc, properties),
	sizeof(xmlDtd),
	offsetof(xmlDtd, notations),
	offsetof(xmlDtd, entities),
	offsetof(xmlDtd, ExternalID),
	offsetof(xmlDtd, SystemID),
	offsetof(xmlDtd, pentities),
	offsetof(xmlEntity, orig),
	offsetof(xmlEntity, content),
	offsetof(xmlEntity, length),
	offsetof(xmlEntity, etype),
	offsetof(xmlEntity, ExternalID),
	offsetof(xmlEntity, SystemID),
	offsetof(xmlEntity, URI),
	sizeof(xmlNotation),
	sizeof(xmlNodeSet),
	offsetof(xmlNodeSet, nodeTab),
	sizeof(xmlXPathObject),
	offsetof(xmlXPathObject, nodesetval),
	offsetof(xmlXPathObject, boolval),
	offsetof(xmlXPathObject, floatval),
	offsetof(xmlXPathObject, stringval),
	offsetof(xmlXPathObject, index2),
};

/* ---- tree helpers with version differences ---- */

/* php_libxml_node_free's XML_ENTITY_DECL case: 2.12 frees an entity with
 * xmlFreeEntity, older releases by hand. */
void rhp_xml_free_entity(xmlEntityPtr entity)
{
#if LIBXML_VERSION >= 21200
	xmlFreeEntity(entity);
#else
	if (entity->children != NULL && entity->owner && entity == (xmlEntityPtr) entity->children->parent) {
		xmlFreeNodeList(entity->children);
	}
	xmlDictPtr dict = entity->doc != NULL ? entity->doc->dict : NULL;
	if (dict == NULL || !xmlDictOwns(dict, entity->name)) {
		xmlFree((xmlChar *) entity->name);
	}
	if (dict == NULL || !xmlDictOwns(dict, entity->ExternalID)) {
		xmlFree((xmlChar *) entity->ExternalID);
	}
	if (dict == NULL || !xmlDictOwns(dict, entity->SystemID)) {
		xmlFree((xmlChar *) entity->SystemID);
	}
	if (dict == NULL || !xmlDictOwns(dict, entity->URI)) {
		xmlFree((xmlChar *) entity->URI);
	}
	if (dict == NULL || !xmlDictOwns(dict, entity->content)) {
		xmlFree(entity->content);
	}
	if (dict == NULL || !xmlDictOwns(dict, entity->orig)) {
		xmlFree(entity->orig);
	}
	xmlFree(entity);
#endif
}

/* The per-thread xmlSaveNoEmptyTags; returns the old value. */
int rhp_xml_set_save_no_empty_tags(int v)
{
	int old = xmlSaveNoEmptyTags;
	xmlSaveNoEmptyTags = v;
	return old;
}

/* PHP_LIBXML_SANITIZE_GLOBALS / PHP_LIBXML_RESTORE_GLOBALS */
void rhp_xml_sanitize_globals(int *saved)
{
	saved[0] = xmlLoadExtDtdDefaultValue;
	xmlLoadExtDtdDefaultValue = 0;
	saved[1] = xmlDoValidityCheckingDefaultValue;
	xmlDoValidityCheckingDefaultValue = 0;
	saved[2] = xmlPedanticParserDefault(0);
	saved[3] = xmlSubstituteEntitiesDefault(0);
	saved[4] = xmlLineNumbersDefault(0);
	saved[5] = xmlKeepBlanksDefault(1);
}

void rhp_xml_restore_globals(const int *saved)
{
	xmlLoadExtDtdDefaultValue = saved[0];
	xmlDoValidityCheckingDefaultValue = saved[1];
	(void) xmlPedanticParserDefault(saved[2]);
	(void) xmlSubstituteEntitiesDefault(saved[3]);
	(void) xmlLineNumbersDefault(saved[4]);
	(void) xmlKeepBlanksDefault(saved[5]);
}

int rhp_xml_output_buffer_error(xmlOutputBufferPtr out) { return out->error; }

/* ---- php's error handlers on validation contexts (ext/dom/document.c) ---- */

void rhp_xml_valid_ctxt_use_php_errors(xmlValidCtxtPtr cvp)
{
	cvp->userData = NULL;
	cvp->error = (xmlValidityErrorFunc) rhp_xml_generic_error;
	cvp->warning = (xmlValidityWarningFunc) rhp_xml_generic_error;
}

#ifdef LIBXML_SCHEMAS_ENABLED
void rhp_xml_schema_parser_use_php_errors(xmlSchemaParserCtxtPtr parser)
{
	xmlSchemaSetParserErrors(parser, (xmlSchemaValidityErrorFunc) rhp_xml_generic_error,
		(xmlSchemaValidityWarningFunc) rhp_xml_generic_error, parser);
}

void rhp_xml_schema_valid_use_php_errors(xmlSchemaValidCtxtPtr vptr)
{
	xmlSchemaSetValidErrors(vptr, (xmlSchemaValidityErrorFunc) rhp_xml_generic_error,
		(xmlSchemaValidityWarningFunc) rhp_xml_generic_error, vptr);
}

void rhp_xml_relaxng_parser_use_php_errors(xmlRelaxNGParserCtxtPtr parser)
{
	xmlRelaxNGSetParserErrors(parser, (xmlRelaxNGValidityErrorFunc) rhp_xml_generic_error,
		(xmlRelaxNGValidityWarningFunc) rhp_xml_generic_error, parser);
}

void rhp_xml_relaxng_valid_use_php_errors(xmlRelaxNGValidCtxtPtr vptr)
{
	xmlRelaxNGSetValidErrors(vptr, (xmlRelaxNGValidityErrorFunc) rhp_xml_generic_error,
		(xmlRelaxNGValidityWarningFunc) rhp_xml_generic_error, vptr);
}
#endif

/* An attribute declaration's default value (xmlAttribute grew in 2.13). */
const xmlChar *rhp_xml_attribute_decl_default(xmlAttributePtr attr) { return attr->defaultValue; }

/* xmlGenericError(xmlGenericErrorContext, msg) */
void rhp_xml_generic_error_msg(const char *msg)
{
	xmlGenericError(xmlGenericErrorContext, "%s", msg);
}

/* ---- parser context fields for ext/dom's document parser ---- */

const char *rhp_xml_ctxt_directory(xmlParserCtxtPtr ctxt) { return ctxt->directory; }

/* Replaces ctxt->directory; takes ownership of dir (libxml2-allocated). */
void rhp_xml_ctxt_set_directory(xmlParserCtxtPtr ctxt, char *dir)
{
	if (ctxt->directory != NULL) {
		xmlFree((char *) ctxt->directory);
	}
	ctxt->directory = dir;
}

int rhp_xml_ctxt_well_formed(xmlParserCtxtPtr ctxt) { return ctxt->wellFormed; }

xmlDocPtr rhp_xml_ctxt_take_doc(xmlParserCtxtPtr ctxt)
{
	xmlDocPtr doc = ctxt->myDoc;
	ctxt->myDoc = NULL;
	return doc;
}

/* ---- XPath contexts ---- */

xmlNodePtr rhp_xml_xpath_node(xmlXPathContextPtr ctxt) { return ctxt->node; }
void rhp_xml_xpath_set_node(xmlXPathContextPtr ctxt, xmlNodePtr node) { ctxt->node = node; }
xmlDocPtr rhp_xml_xpath_doc(xmlXPathContextPtr ctxt) { return ctxt->doc; }

xmlNsPtr *rhp_xml_xpath_namespaces(xmlXPathContextPtr ctxt, int *nr)
{
	*nr = ctxt->nsNr;
	return ctxt->namespaces;
}

void rhp_xml_xpath_set_namespaces(xmlXPathContextPtr ctxt, xmlNsPtr *list, int nr)
{
	ctxt->namespaces = list;
	ctxt->nsNr = nr;
}

void *rhp_xml_xpath_user_data(xmlXPathContextPtr ctxt) { return ctxt->userData; }
void rhp_xml_xpath_set_user_data(xmlXPathContextPtr ctxt, void *data) { ctxt->userData = data; }
xmlXPathContextPtr rhp_xml_xpath_parser_context(xmlXPathParserContextPtr ctxt) { return ctxt->context; }

const xmlChar *rhp_xml_xpath_function(xmlXPathContextPtr ctxt, const xmlChar **uri)
{
	*uri = ctxt->functionURI;
	return ctxt->function;
}

/* valuePop and valuePush are macros in 2.15. */
xmlXPathObjectPtr rhp_xml_xpath_value_pop(xmlXPathParserContextPtr ctxt) { return valuePop(ctxt); }
void rhp_xml_xpath_value_push(xmlXPathParserContextPtr ctxt, xmlXPathObjectPtr obj) { valuePush(ctxt, obj); }
