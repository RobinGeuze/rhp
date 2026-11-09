//! The libxml2 functions rhp calls, declared by hand (no bindgen), plus
//! `shim.c`'s accessors. build.rs links the libxml2 that php links
//! (docs/design/libraries.md) and compiles the shim against its headers.
//!
//! Most structs are opaque: their layouts differ between libxml2 releases,
//! so their fields are read through the shim. The tree structs that have
//! not changed since 2.9 are mirrored here, because DOM walks them
//! everywhere: [`XmlErrorRaw`], [`XmlNode`], [`XmlDoc`], [`XmlNs`],
//! [`XmlDtd`], [`XmlNodeSet`], [`XmlXPathObject`], and the leading fields
//! of [`XmlAttr`] and [`XmlEntity`], which grew at the end (never sized or
//! allocated in Rust). A test compares every mirror with the headers rhp
//! compiles the shim against.

#![allow(non_snake_case, non_camel_case_types, dead_code)]

use std::ffi::{c_char, c_int, c_long, c_uint, c_void};

macro_rules! opaque {
    ($($name:ident),*) => {$(
        #[repr(C)]
        pub struct $name {
            _p: [u8; 0],
            _m: std::marker::PhantomData<(*mut u8, std::marker::PhantomPinned)>,
        }
    )*};
}

opaque!(
    xmlParserCtxt,
    xmlParserInput,
    xmlParserInputBuffer,
    xmlOutputBuffer,
    xmlURI,
    xmlTextWriter,
    xmlCharEncodingHandler,
    xmlDict,
    xmlHashTable,
    xmlBuffer,
    xmlSaveCtxt,
    xmlValidCtxt,
    xmlXPathContext,
    xmlXPathParserContext,
    xmlSchemaParserCtxt,
    xmlSchema,
    xmlSchemaValidCtxt,
    xmlRelaxNGParserCtxt,
    xmlRelaxNG,
    xmlRelaxNGValidCtxt,
    xmlDOMWrapCtxt,
    xmlTextReader
);

pub type xmlChar = u8;
pub type Ctxt = *mut xmlParserCtxt;
pub type xmlDoc = XmlDoc;
pub type xmlEntity = XmlEntity;

// xmlElementType
pub const XML_ELEMENT_NODE: c_int = 1;
pub const XML_ATTRIBUTE_NODE: c_int = 2;
pub const XML_TEXT_NODE: c_int = 3;
pub const XML_CDATA_SECTION_NODE: c_int = 4;
pub const XML_ENTITY_REF_NODE: c_int = 5;
pub const XML_ENTITY_NODE: c_int = 6;
pub const XML_PI_NODE: c_int = 7;
pub const XML_COMMENT_NODE: c_int = 8;
pub const XML_DOCUMENT_NODE: c_int = 9;
pub const XML_DOCUMENT_TYPE_NODE: c_int = 10;
pub const XML_DOCUMENT_FRAG_NODE: c_int = 11;
pub const XML_NOTATION_NODE: c_int = 12;
pub const XML_HTML_DOCUMENT_NODE: c_int = 13;
pub const XML_DTD_NODE: c_int = 14;
pub const XML_ELEMENT_DECL: c_int = 15;
pub const XML_ATTRIBUTE_DECL: c_int = 16;
pub const XML_ENTITY_DECL: c_int = 17;
pub const XML_NAMESPACE_DECL: c_int = 18;
pub const XML_XINCLUDE_START: c_int = 19;
pub const XML_XINCLUDE_END: c_int = 20;

/// xmlAttributeType's XML_ATTRIBUTE_ID.
pub const XML_ATTRIBUTE_ID: c_int = 2;
/// xmlNsType's XML_LOCAL_NAMESPACE.
pub const XML_LOCAL_NAMESPACE: c_int = XML_NAMESPACE_DECL;

// xmlXPathObjectType
pub const XPATH_UNDEFINED: c_int = 0;
pub const XPATH_NODESET: c_int = 1;
pub const XPATH_BOOLEAN: c_int = 2;
pub const XPATH_NUMBER: c_int = 3;
pub const XPATH_STRING: c_int = 4;

/// `struct _xmlNode`.
#[repr(C)]
pub struct XmlNode {
    pub _private: *mut c_void,
    pub type_: c_int,
    pub name: *const xmlChar,
    pub children: *mut XmlNode,
    pub last: *mut XmlNode,
    pub parent: *mut XmlNode,
    pub next: *mut XmlNode,
    pub prev: *mut XmlNode,
    pub doc: *mut XmlDoc,
    pub ns: *mut XmlNs,
    pub content: *mut xmlChar,
    pub properties: *mut XmlAttr,
    pub nsDef: *mut XmlNs,
    pub psvi: *mut c_void,
    pub line: u16,
    pub extra: u16,
}

/// `struct _xmlAttr`, up to `psvi` (2.13 added a field after it).
#[repr(C)]
pub struct XmlAttr {
    pub _private: *mut c_void,
    pub type_: c_int,
    pub name: *const xmlChar,
    pub children: *mut XmlNode,
    pub last: *mut XmlNode,
    pub parent: *mut XmlNode,
    pub next: *mut XmlAttr,
    pub prev: *mut XmlAttr,
    pub doc: *mut XmlDoc,
    pub ns: *mut XmlNs,
    pub atype: c_int,
    pub psvi: *mut c_void,
}

/// `struct _xmlNs`.
#[repr(C)]
pub struct XmlNs {
    pub next: *mut XmlNs,
    pub type_: c_int,
    pub href: *const xmlChar,
    pub prefix: *const xmlChar,
    pub _private: *mut c_void,
    pub context: *mut XmlDoc,
}

/// `struct _xmlDoc`.
#[repr(C)]
pub struct XmlDoc {
    pub _private: *mut c_void,
    pub type_: c_int,
    pub name: *mut c_char,
    pub children: *mut XmlNode,
    pub last: *mut XmlNode,
    pub parent: *mut XmlNode,
    pub next: *mut XmlNode,
    pub prev: *mut XmlNode,
    pub doc: *mut XmlDoc,
    pub compression: c_int,
    pub standalone: c_int,
    pub intSubset: *mut XmlDtd,
    pub extSubset: *mut XmlDtd,
    pub oldNs: *mut XmlNs,
    pub version: *const xmlChar,
    pub encoding: *const xmlChar,
    pub ids: *mut c_void,
    pub refs: *mut c_void,
    pub URL: *const xmlChar,
    pub charset: c_int,
    pub dict: *mut xmlDict,
    pub psvi: *mut c_void,
    pub parseFlags: c_int,
    pub properties: c_int,
}

/// `struct _xmlDtd`.
#[repr(C)]
pub struct XmlDtd {
    pub _private: *mut c_void,
    pub type_: c_int,
    pub name: *const xmlChar,
    pub children: *mut XmlNode,
    pub last: *mut XmlNode,
    pub parent: *mut XmlDoc,
    pub next: *mut XmlNode,
    pub prev: *mut XmlNode,
    pub doc: *mut XmlDoc,
    pub notations: *mut xmlHashTable,
    pub elements: *mut xmlHashTable,
    pub attributes: *mut xmlHashTable,
    pub entities: *mut xmlHashTable,
    pub ExternalID: *const xmlChar,
    pub SystemID: *const xmlChar,
    pub pentities: *mut xmlHashTable,
}

/// `struct _xmlEntity`, up to `URI` (the fields after it changed).
#[repr(C)]
pub struct XmlEntity {
    pub _private: *mut c_void,
    pub type_: c_int,
    pub name: *const xmlChar,
    pub children: *mut XmlNode,
    pub last: *mut XmlNode,
    pub parent: *mut XmlDtd,
    pub next: *mut XmlNode,
    pub prev: *mut XmlNode,
    pub doc: *mut XmlDoc,
    pub orig: *mut xmlChar,
    pub content: *mut xmlChar,
    pub length: c_int,
    pub etype: c_int,
    pub ExternalID: *const xmlChar,
    pub SystemID: *const xmlChar,
    pub nexte: *mut XmlEntity,
    pub URI: *const xmlChar,
}

/// `struct _xmlNotation`.
#[repr(C)]
pub struct XmlNotation {
    pub name: *const xmlChar,
    pub PublicID: *const xmlChar,
    pub SystemID: *const xmlChar,
}

/// `struct _xmlNodeSet`.
#[repr(C)]
pub struct XmlNodeSet {
    pub nodeNr: c_int,
    pub nodeMax: c_int,
    pub nodeTab: *mut *mut XmlNode,
}

/// `struct _xmlXPathObject`.
#[repr(C)]
pub struct XmlXPathObject {
    pub type_: c_int,
    pub nodesetval: *mut XmlNodeSet,
    pub boolval: c_int,
    pub floatval: f64,
    pub stringval: *mut xmlChar,
    pub user: *mut c_void,
    pub index: c_int,
    pub user2: *mut c_void,
    pub index2: c_int,
}

pub type xmlXPathFunction = unsafe extern "C" fn(*mut xmlXPathParserContext, c_int);
pub type xmlHashScanner = unsafe extern "C" fn(*mut c_void, *mut c_void, *const xmlChar);
pub type xmlC14NIsVisibleCallback =
    unsafe extern "C" fn(*mut c_void, *mut XmlNode, *mut XmlNode) -> c_int;

/// `struct _xmlError`.
#[repr(C)]
pub struct XmlErrorRaw {
    pub domain: c_int,
    pub code: c_int,
    pub message: *mut c_char,
    pub level: c_int,
    pub file: *mut c_char,
    pub line: c_int,
    pub str1: *mut c_char,
    pub str2: *mut c_char,
    pub str3: *mut c_char,
    pub int1: c_int,
    pub int2: c_int,
    pub ctxt: *mut c_void,
    pub node: *mut c_void,
}

pub type xmlInputReadCallback = unsafe extern "C" fn(*mut c_void, *mut c_char, c_int) -> c_int;
pub type xmlInputCloseCallback = unsafe extern "C" fn(*mut c_void) -> c_int;
pub type xmlOutputWriteCallback = unsafe extern "C" fn(*mut c_void, *const c_char, c_int) -> c_int;
pub type xmlOutputCloseCallback = unsafe extern "C" fn(*mut c_void) -> c_int;
pub type xmlParserInputBufferCreateFilenameFunc =
    unsafe extern "C" fn(*const c_char, c_int) -> *mut xmlParserInputBuffer;
pub type xmlOutputBufferCreateFilenameFunc =
    unsafe extern "C" fn(*const c_char, *mut xmlCharEncodingHandler, c_int) -> *mut xmlOutputBuffer;
pub type xmlExternalEntityLoader =
    unsafe extern "C" fn(*const c_char, *const c_char, Ctxt) -> *mut xmlParserInput;
pub type xmlStructuredErrorFunc = unsafe extern "C" fn(*mut c_void, *const XmlErrorRaw);

/// shim.c `struct rhp_xml_sax`: ext/xml's SAX callbacks.
#[repr(C)]
pub struct RhpXmlSax {
    pub get_entity: unsafe extern "C" fn(*mut c_void, *const xmlChar) -> *mut xmlEntity,
    pub notation_decl:
        unsafe extern "C" fn(*mut c_void, *const xmlChar, *const xmlChar, *const xmlChar),
    pub unparsed_entity_decl: unsafe extern "C" fn(
        *mut c_void,
        *const xmlChar,
        *const xmlChar,
        *const xmlChar,
        *const xmlChar,
    ),
    pub start_element: unsafe extern "C" fn(*mut c_void, *const xmlChar, *mut *const xmlChar),
    pub end_element: unsafe extern "C" fn(*mut c_void, *const xmlChar),
    pub characters: unsafe extern "C" fn(*mut c_void, *const xmlChar, c_int),
    pub processing_instruction: unsafe extern "C" fn(*mut c_void, *const xmlChar, *const xmlChar),
    pub comment: unsafe extern "C" fn(*mut c_void, *const xmlChar),
    pub start_element_ns: unsafe extern "C" fn(
        *mut c_void,
        *const xmlChar,
        *const xmlChar,
        *const xmlChar,
        c_int,
        *mut *const xmlChar,
        c_int,
        c_int,
        *mut *const xmlChar,
    ),
    pub end_element_ns:
        unsafe extern "C" fn(*mut c_void, *const xmlChar, *const xmlChar, *const xmlChar),
}

/// shim.c `struct rhp_xml_const`.
#[repr(C)]
pub struct RhpXmlConst {
    pub name: *const c_char,
    pub value: i64,
}

// xmlParserOption
pub const XML_PARSE_NOENT: c_int = 1 << 1;
pub const XML_PARSE_OLDSAX: c_int = 1 << 20;

// xmlErrorLevel
pub const XML_ERR_WARNING: c_int = 1;
pub const XML_ERR_ERROR: c_int = 2;

// xmlEntityType
pub const XML_INTERNAL_GENERAL_ENTITY: c_int = 1;
pub const XML_EXTERNAL_GENERAL_PARSED_ENTITY: c_int = 2;
pub const XML_INTERNAL_PARAMETER_ENTITY: c_int = 4;
pub const XML_INTERNAL_PREDEFINED_ENTITY: c_int = 6;

pub const XML_ERR_INTERNAL_ERROR: c_int = 1;

unsafe extern "C" {
    // parser
    pub fn xmlParseChunk(ctxt: Ctxt, chunk: *const c_char, size: c_int, terminate: c_int) -> c_int;
    pub fn xmlCtxtUseOptions(ctxt: Ctxt, options: c_int) -> c_int;
    pub fn xmlFreeParserCtxt(ctxt: Ctxt);
    pub fn xmlStopParser(ctxt: Ctxt);
    pub fn xmlCtxtGetLastError(ctx: *mut c_void) -> *const XmlErrorRaw;
    pub fn xmlGetPredefinedEntity(name: *const xmlChar) -> *mut xmlEntity;
    pub fn xmlGetDocEntity(doc: *const xmlDoc, name: *const xmlChar) -> *mut xmlEntity;

    // errors
    pub fn xmlGetLastError() -> *const XmlErrorRaw;
    pub fn xmlResetLastError();
    pub fn xmlSetStructuredErrorFunc(ctx: *mut c_void, handler: Option<xmlStructuredErrorFunc>);

    // I/O and entity loading
    pub fn xmlGetExternalEntityLoader() -> Option<xmlExternalEntityLoader>;
    pub fn xmlSetExternalEntityLoader(f: Option<xmlExternalEntityLoader>);
    pub fn xmlNewInputFromFile(ctxt: Ctxt, filename: *const c_char) -> *mut xmlParserInput;
    pub fn xmlParserInputBufferCreateFilenameDefault(
        f: Option<xmlParserInputBufferCreateFilenameFunc>,
    ) -> Option<xmlParserInputBufferCreateFilenameFunc>;
    pub fn xmlOutputBufferCreateFilenameDefault(
        f: Option<xmlOutputBufferCreateFilenameFunc>,
    ) -> Option<xmlOutputBufferCreateFilenameFunc>;
    pub fn xmlParseCharEncoding(name: *const c_char) -> c_int;
    pub fn xmlParserInputBufferCreateIO(
        ioread: Option<xmlInputReadCallback>,
        ioclose: Option<xmlInputCloseCallback>,
        ioctx: *mut c_void,
        enc: c_int,
    ) -> *mut xmlParserInputBuffer;
    pub fn xmlOutputBufferCreateIO(
        iowrite: Option<xmlOutputWriteCallback>,
        ioclose: Option<xmlOutputCloseCallback>,
        ioctx: *mut c_void,
        encoder: *mut xmlCharEncodingHandler,
    ) -> *mut xmlOutputBuffer;
    pub fn xmlOutputBufferClose(out: *mut xmlOutputBuffer) -> c_int;

    // URIs
    pub fn xmlParseURI(s: *const c_char) -> *mut xmlURI;
    pub fn xmlCreateURI() -> *mut xmlURI;
    pub fn xmlParseURIReference(uri: *mut xmlURI, s: *const c_char) -> c_int;
    pub fn xmlFreeURI(uri: *mut xmlURI);
    pub fn xmlURIUnescapeString(s: *const c_char, len: c_int, target: *mut c_char) -> *mut c_char;
    pub fn xmlURIEscapeStr(s: *const xmlChar, list: *const xmlChar) -> *mut xmlChar;

    // tree
    pub fn xmlValidateName(value: *const xmlChar, space: c_int) -> c_int;
    pub fn xmlValidateNCName(value: *const xmlChar, space: c_int) -> c_int;
    pub fn xmlValidateQName(value: *const xmlChar, space: c_int) -> c_int;
    pub fn xmlSplitQName2(name: *const xmlChar, prefix: *mut *mut xmlChar) -> *mut xmlChar;
    pub fn xmlNewDoc(version: *const xmlChar) -> *mut XmlDoc;
    pub fn xmlFreeDoc(doc: *mut XmlDoc);
    pub fn xmlNewDocNode(
        doc: *mut XmlDoc,
        ns: *mut XmlNs,
        name: *const xmlChar,
        content: *const xmlChar,
    ) -> *mut XmlNode;
    pub fn xmlNewDocRawNode(
        doc: *mut XmlDoc,
        ns: *mut XmlNs,
        name: *const xmlChar,
        content: *const xmlChar,
    ) -> *mut XmlNode;
    pub fn xmlNewNode(ns: *mut XmlNs, name: *const xmlChar) -> *mut XmlNode;
    pub fn xmlNewDocText(doc: *const XmlDoc, content: *const xmlChar) -> *mut XmlNode;
    pub fn xmlNewDocTextLen(doc: *mut XmlDoc, content: *const xmlChar, len: c_int) -> *mut XmlNode;
    pub fn xmlNewText(content: *const xmlChar) -> *mut XmlNode;
    pub fn xmlNewTextLen(content: *const xmlChar, len: c_int) -> *mut XmlNode;
    pub fn xmlNewDocComment(doc: *mut XmlDoc, content: *const xmlChar) -> *mut XmlNode;
    pub fn xmlNewComment(content: *const xmlChar) -> *mut XmlNode;
    pub fn xmlNewCDataBlock(doc: *mut XmlDoc, content: *const xmlChar, len: c_int) -> *mut XmlNode;
    pub fn xmlNewDocPI(
        doc: *mut XmlDoc,
        name: *const xmlChar,
        content: *const xmlChar,
    ) -> *mut XmlNode;
    pub fn xmlNewPI(name: *const xmlChar, content: *const xmlChar) -> *mut XmlNode;
    pub fn xmlNewReference(doc: *const XmlDoc, name: *const xmlChar) -> *mut XmlNode;
    pub fn xmlNewDocFragment(doc: *mut XmlDoc) -> *mut XmlNode;
    pub fn xmlNewDocProp(
        doc: *mut XmlDoc,
        name: *const xmlChar,
        value: *const xmlChar,
    ) -> *mut XmlAttr;
    pub fn xmlNewProp(
        node: *mut XmlNode,
        name: *const xmlChar,
        value: *const xmlChar,
    ) -> *mut XmlAttr;
    pub fn xmlAddChildList(parent: *mut XmlNode, cur: *mut XmlNode) -> *mut XmlNode;
    pub fn xmlNewNs(node: *mut XmlNode, href: *const xmlChar, prefix: *const xmlChar)
        -> *mut XmlNs;
    pub fn xmlFreeNs(ns: *mut XmlNs);
    pub fn xmlFreeNode(node: *mut XmlNode);
    pub fn xmlFreeNodeList(node: *mut XmlNode);
    pub fn xmlFreeProp(attr: *mut XmlAttr);
    pub fn xmlFreeDtd(dtd: *mut XmlDtd);
    pub fn xmlUnlinkNode(node: *mut XmlNode);
    pub fn xmlAddChild(parent: *mut XmlNode, cur: *mut XmlNode) -> *mut XmlNode;
    pub fn xmlAddPrevSibling(cur: *mut XmlNode, elem: *mut XmlNode) -> *mut XmlNode;
    pub fn xmlAddNextSibling(cur: *mut XmlNode, elem: *mut XmlNode) -> *mut XmlNode;
    pub fn xmlReplaceNode(old: *mut XmlNode, cur: *mut XmlNode) -> *mut XmlNode;
    pub fn xmlDocGetRootElement(doc: *const XmlDoc) -> *mut XmlNode;
    pub fn xmlDocSetRootElement(doc: *mut XmlDoc, root: *mut XmlNode) -> *mut XmlNode;
    pub fn xmlSetTreeDoc(tree: *mut XmlNode, doc: *mut XmlDoc);
    pub fn xmlSearchNs(doc: *mut XmlDoc, node: *mut XmlNode, prefix: *const xmlChar) -> *mut XmlNs;
    pub fn xmlSearchNsByHref(
        doc: *mut XmlDoc,
        node: *mut XmlNode,
        href: *const xmlChar,
    ) -> *mut XmlNs;
    pub fn xmlGetNsList(doc: *const XmlDoc, node: *const XmlNode) -> *mut *mut XmlNs;
    pub fn xmlReconciliateNs(doc: *mut XmlDoc, tree: *mut XmlNode) -> c_int;
    pub fn xmlSetNs(node: *mut XmlNode, ns: *mut XmlNs);
    pub fn xmlHasNsProp(
        node: *const XmlNode,
        name: *const xmlChar,
        ns: *const xmlChar,
    ) -> *mut XmlAttr;
    pub fn xmlHasProp(node: *const XmlNode, name: *const xmlChar) -> *mut XmlAttr;
    pub fn xmlSetProp(
        node: *mut XmlNode,
        name: *const xmlChar,
        value: *const xmlChar,
    ) -> *mut XmlAttr;
    pub fn xmlSetNsProp(
        node: *mut XmlNode,
        ns: *mut XmlNs,
        name: *const xmlChar,
        value: *const xmlChar,
    ) -> *mut XmlAttr;
    pub fn xmlGetNsProp(
        node: *const XmlNode,
        name: *const xmlChar,
        ns: *const xmlChar,
    ) -> *mut xmlChar;
    pub fn xmlGetNoNsProp(node: *const XmlNode, name: *const xmlChar) -> *mut xmlChar;
    pub fn xmlNodeGetContent(node: *const XmlNode) -> *mut xmlChar;
    pub fn xmlNodeSetContent(node: *mut XmlNode, content: *const xmlChar);
    pub fn xmlNodeSetContentLen(node: *mut XmlNode, content: *const xmlChar, len: c_int);
    pub fn xmlNodeAddContent(node: *mut XmlNode, content: *const xmlChar);
    pub fn xmlNodeAddContentLen(node: *mut XmlNode, content: *const xmlChar, len: c_int);
    pub fn xmlNodeListGetString(
        doc: *mut XmlDoc,
        list: *const XmlNode,
        inline: c_int,
    ) -> *mut xmlChar;
    pub fn xmlNodeGetBase(doc: *const XmlDoc, node: *const XmlNode) -> *mut xmlChar;
    pub fn xmlGetNodePath(node: *const XmlNode) -> *mut xmlChar;
    pub fn xmlGetLineNo(node: *const XmlNode) -> c_long;
    pub fn xmlDocCopyNode(node: *mut XmlNode, doc: *mut XmlDoc, extended: c_int) -> *mut XmlNode;
    pub fn xmlCopyDtd(dtd: *mut XmlDtd) -> *mut XmlDtd;
    pub fn xmlGetIntSubset(doc: *const XmlDoc) -> *mut XmlDtd;
    pub fn xmlCreateIntSubset(
        doc: *mut XmlDoc,
        name: *const xmlChar,
        external_id: *const xmlChar,
        system_id: *const xmlChar,
    ) -> *mut XmlDtd;
    pub fn xmlTextConcat(node: *mut XmlNode, content: *const xmlChar, len: c_int) -> c_int;
    pub fn xmlIsBlankNode(node: *const XmlNode) -> c_int;
    pub fn xmlDOMWrapAdoptNode(
        ctxt: *mut xmlDOMWrapCtxt,
        source_doc: *mut XmlDoc,
        node: *mut XmlNode,
        dest_doc: *mut XmlDoc,
        dest_parent: *mut XmlNode,
        options: c_int,
    ) -> c_int;
    pub fn xmlGetID(doc: *mut XmlDoc, id: *const xmlChar) -> *mut XmlAttr;
    pub fn xmlRemoveID(doc: *mut XmlDoc, attr: *mut XmlAttr) -> c_int;
    pub fn xmlAddID(
        ctxt: *mut xmlValidCtxt,
        doc: *mut XmlDoc,
        value: *const xmlChar,
        attr: *mut XmlAttr,
    ) -> *mut c_void;
    pub fn xmlIsID(doc: *mut XmlDoc, elem: *mut XmlNode, attr: *mut XmlAttr) -> c_int;

    // strings
    pub fn xmlStrdup(s: *const xmlChar) -> *mut xmlChar;
    pub fn xmlStrndup(s: *const xmlChar, len: c_int) -> *mut xmlChar;
    pub fn xmlStrEqual(a: *const xmlChar, b: *const xmlChar) -> c_int;
    pub fn xmlStrlen(s: *const xmlChar) -> c_int;
    pub fn xmlUTF8Strlen(s: *const xmlChar) -> c_int;
    pub fn xmlUTF8Strsize(s: *const xmlChar, len: c_int) -> c_int;
    pub fn xmlUTF8Strpos(s: *const xmlChar, pos: c_int) -> *const xmlChar;
    pub fn xmlUTF8Strsub(s: *const xmlChar, start: c_int, len: c_int) -> *mut xmlChar;
    pub fn xmlUTF8Strndup(s: *const xmlChar, len: c_int) -> *mut xmlChar;
    pub fn xmlStrcat(cur: *mut xmlChar, add: *const xmlChar) -> *mut xmlChar;

    // dict and hash tables
    pub fn xmlDictOwns(dict: *mut xmlDict, s: *const xmlChar) -> c_int;
    pub fn xmlDictLookup(dict: *mut xmlDict, name: *const xmlChar, len: c_int) -> *const xmlChar;
    pub fn xmlDictExists(dict: *mut xmlDict, name: *const xmlChar, len: c_int) -> *const xmlChar;
    pub fn xmlDictReference(dict: *mut xmlDict) -> c_int;
    pub fn xmlDictFree(dict: *mut xmlDict);
    pub fn xmlHashLookup(table: *mut xmlHashTable, name: *const xmlChar) -> *mut c_void;
    pub fn xmlHashSize(table: *mut xmlHashTable) -> c_int;
    pub fn xmlHashScan(table: *mut xmlHashTable, f: xmlHashScanner, data: *mut c_void);
    pub fn xmlHashRemoveEntry(
        table: *mut xmlHashTable,
        name: *const xmlChar,
        dealloc: *mut c_void,
    ) -> c_int;

    // output
    pub fn xmlAllocOutputBuffer(encoder: *mut xmlCharEncodingHandler) -> *mut xmlOutputBuffer;
    pub fn xmlOutputBufferCreateFilename(
        uri: *const c_char,
        encoder: *mut xmlCharEncodingHandler,
        compression: c_int,
    ) -> *mut xmlOutputBuffer;
    pub fn xmlOutputBufferCreateBuffer(
        buffer: *mut xmlBuffer,
        encoder: *mut xmlCharEncodingHandler,
    ) -> *mut xmlOutputBuffer;
    pub fn xmlOutputBufferFlush(out: *mut xmlOutputBuffer) -> c_int;
    pub fn xmlOutputBufferGetContent(out: *mut xmlOutputBuffer) -> *const xmlChar;
    pub fn xmlOutputBufferGetSize(out: *mut xmlOutputBuffer) -> usize;
    pub fn xmlBufferCreate() -> *mut xmlBuffer;
    pub fn xmlBufferFree(buf: *mut xmlBuffer);
    pub fn xmlBufferContent(buf: *const xmlBuffer) -> *const xmlChar;
    pub fn xmlBufferLength(buf: *const xmlBuffer) -> c_int;
    pub fn xmlNodeDumpOutput(
        buf: *mut xmlOutputBuffer,
        doc: *mut XmlDoc,
        cur: *mut XmlNode,
        level: c_int,
        format: c_int,
        encoding: *const c_char,
    );
    pub fn xmlSaveToIO(
        iowrite: Option<xmlOutputWriteCallback>,
        ioclose: Option<xmlOutputCloseCallback>,
        ioctx: *mut c_void,
        encoding: *const c_char,
        options: c_int,
    ) -> *mut xmlSaveCtxt;
    pub fn xmlSaveDoc(ctxt: *mut xmlSaveCtxt, doc: *mut XmlDoc) -> c_long;
    pub fn xmlDocDumpMemory(doc: *mut XmlDoc, mem: *mut *mut xmlChar, size: *mut c_int);
    pub fn xmlSaveClose(ctxt: *mut xmlSaveCtxt) -> c_int;
    pub fn xmlSaveTree(ctxt: *mut xmlSaveCtxt, node: *mut XmlNode) -> c_long;
    pub fn xmlSaveFlush(ctxt: *mut xmlSaveCtxt) -> c_int;
    pub fn xmlOutputBufferWrite(out: *mut xmlOutputBuffer, len: c_int, buf: *const c_char)
        -> c_int;
    pub fn xmlOutputBufferWriteString(out: *mut xmlOutputBuffer, s: *const c_char) -> c_int;
    pub fn xmlGetUTF8Char(utf: *const xmlChar, len: *mut c_int) -> c_int;
    pub fn htmlNewDocNoDtD(uri: *const xmlChar, external_id: *const xmlChar) -> *mut XmlDoc;
    pub fn xmlDictCreate() -> *mut xmlDict;
    pub fn xmlPathToURI(path: *const xmlChar) -> *mut xmlChar;
    pub fn xmlSaveFormatFileEnc(
        filename: *const c_char,
        doc: *mut XmlDoc,
        encoding: *const c_char,
        format: c_int,
    ) -> c_int;
    pub fn xmlFindCharEncodingHandler(name: *const c_char) -> *mut xmlCharEncodingHandler;
    pub fn xmlCharEncCloseFunc(handler: *mut xmlCharEncodingHandler) -> c_int;

    // parsing documents
    pub fn xmlCreateMemoryParserCtxt(buffer: *const c_char, size: c_int) -> Ctxt;
    pub fn xmlCreateFileParserCtxt(filename: *const c_char) -> Ctxt;
    pub fn xmlParseDocument(ctxt: Ctxt) -> c_int;
    pub fn xmlSwitchToEncoding(ctxt: Ctxt, handler: *mut xmlCharEncodingHandler) -> c_int;
    pub fn xmlCanonicPath(path: *const xmlChar) -> *mut xmlChar;
    pub fn xmlParseBalancedChunkMemory(
        doc: *mut XmlDoc,
        sax: *mut c_void,
        user_data: *mut c_void,
        depth: c_int,
        string: *const xmlChar,
        lst: *mut *mut XmlNode,
    ) -> c_int;
    pub fn xmlInitParser();

    // HTML
    pub fn htmlCreateMemoryParserCtxt(buffer: *const c_char, size: c_int) -> Ctxt;
    pub fn htmlCreateFileParserCtxt(filename: *const c_char, encoding: *const c_char) -> Ctxt;
    pub fn htmlCtxtUseOptions(ctxt: Ctxt, options: c_int) -> c_int;
    pub fn htmlParseDocument(ctxt: Ctxt) -> c_int;
    pub fn htmlFreeParserCtxt(ctxt: Ctxt);
    pub fn htmlDocDumpMemoryFormat(
        doc: *mut XmlDoc,
        mem: *mut *mut xmlChar,
        size: *mut c_int,
        format: c_int,
    );
    pub fn htmlNodeDumpFormatOutput(
        buf: *mut xmlOutputBuffer,
        doc: *mut XmlDoc,
        cur: *mut XmlNode,
        encoding: *const c_char,
        format: c_int,
    );
    pub fn htmlSaveFileFormat(
        filename: *const c_char,
        doc: *mut XmlDoc,
        encoding: *const c_char,
        format: c_int,
    ) -> c_int;
    pub fn htmlGetMetaEncoding(doc: *mut XmlDoc) -> *const xmlChar;

    // validation
    pub fn xmlNewValidCtxt() -> *mut xmlValidCtxt;
    pub fn xmlFreeValidCtxt(ctxt: *mut xmlValidCtxt);
    pub fn xmlValidateDocument(ctxt: *mut xmlValidCtxt, doc: *mut XmlDoc) -> c_int;
    pub fn xmlSchemaNewParserCtxt(url: *const c_char) -> *mut xmlSchemaParserCtxt;
    pub fn xmlSchemaNewMemParserCtxt(
        buffer: *const c_char,
        size: c_int,
    ) -> *mut xmlSchemaParserCtxt;
    pub fn xmlSchemaParse(ctxt: *mut xmlSchemaParserCtxt) -> *mut xmlSchema;
    pub fn xmlSchemaFreeParserCtxt(ctxt: *mut xmlSchemaParserCtxt);
    pub fn xmlSchemaNewValidCtxt(schema: *mut xmlSchema) -> *mut xmlSchemaValidCtxt;
    pub fn xmlSchemaSetValidOptions(ctxt: *mut xmlSchemaValidCtxt, options: c_int) -> c_int;
    pub fn xmlSchemaValidateDoc(ctxt: *mut xmlSchemaValidCtxt, doc: *mut XmlDoc) -> c_int;
    pub fn xmlSchemaFree(schema: *mut xmlSchema);
    pub fn xmlSchemaFreeValidCtxt(ctxt: *mut xmlSchemaValidCtxt);
    pub fn xmlRelaxNGNewParserCtxt(url: *const c_char) -> *mut xmlRelaxNGParserCtxt;
    pub fn xmlRelaxNGNewMemParserCtxt(
        buffer: *const c_char,
        size: c_int,
    ) -> *mut xmlRelaxNGParserCtxt;
    pub fn xmlRelaxNGParse(ctxt: *mut xmlRelaxNGParserCtxt) -> *mut xmlRelaxNG;
    pub fn xmlRelaxNGFreeParserCtxt(ctxt: *mut xmlRelaxNGParserCtxt);
    pub fn xmlRelaxNGNewValidCtxt(schema: *mut xmlRelaxNG) -> *mut xmlRelaxNGValidCtxt;
    pub fn xmlRelaxNGValidateDoc(ctxt: *mut xmlRelaxNGValidCtxt, doc: *mut XmlDoc) -> c_int;
    pub fn xmlRelaxNGFree(schema: *mut xmlRelaxNG);
    pub fn xmlRelaxNGFreeValidCtxt(ctxt: *mut xmlRelaxNGValidCtxt);
    pub fn xmlXIncludeProcessFlags(doc: *mut XmlDoc, flags: c_int) -> c_int;
    pub fn xmlC14NExecute(
        doc: *mut XmlDoc,
        is_visible: Option<xmlC14NIsVisibleCallback>,
        user_data: *mut c_void,
        mode: c_int,
        inclusive_ns_prefixes: *mut *mut xmlChar,
        with_comments: c_int,
        buf: *mut xmlOutputBuffer,
    ) -> c_int;
    pub fn xmlC14NDocSaveTo(
        doc: *mut XmlDoc,
        nodes: *mut XmlNodeSet,
        mode: c_int,
        inclusive_ns_prefixes: *mut *mut xmlChar,
        with_comments: c_int,
        buf: *mut xmlOutputBuffer,
    ) -> c_int;

    // XPath
    pub fn xmlXPathNewContext(doc: *mut XmlDoc) -> *mut xmlXPathContext;
    pub fn xmlXPathFreeContext(ctxt: *mut xmlXPathContext);
    pub fn xmlXPathRegisterNs(
        ctxt: *mut xmlXPathContext,
        prefix: *const xmlChar,
        ns_uri: *const xmlChar,
    ) -> c_int;
    pub fn xmlXPathRegisterFuncNS(
        ctxt: *mut xmlXPathContext,
        name: *const xmlChar,
        ns_uri: *const xmlChar,
        f: Option<xmlXPathFunction>,
    ) -> c_int;
    pub fn xmlXPathEvalExpression(
        s: *const xmlChar,
        ctxt: *mut xmlXPathContext,
    ) -> *mut XmlXPathObject;
    pub fn xmlXPathFreeObject(obj: *mut XmlXPathObject);
    pub fn xmlXPathNewString(s: *const xmlChar) -> *mut XmlXPathObject;
    pub fn xmlXPathNewBoolean(v: c_int) -> *mut XmlXPathObject;
    pub fn xmlXPathNewNodeSet(node: *mut XmlNode) -> *mut XmlXPathObject;
    pub fn xmlXPathCastToString(obj: *mut XmlXPathObject) -> *mut xmlChar;
    pub fn xmlXPathEval(s: *const xmlChar, ctxt: *mut xmlXPathContext) -> *mut XmlXPathObject;

    // SimpleXML
    pub fn xmlReadMemory(
        buffer: *const c_char,
        size: c_int,
        url: *const c_char,
        encoding: *const c_char,
        options: c_int,
    ) -> *mut XmlDoc;
    pub fn xmlReadFile(url: *const c_char, encoding: *const c_char, options: c_int) -> *mut XmlDoc;
    pub fn xmlCopyDoc(doc: *mut XmlDoc, recursive: c_int) -> *mut XmlDoc;
    pub fn xmlNewChild(
        parent: *mut XmlNode,
        ns: *mut XmlNs,
        name: *const xmlChar,
        content: *const xmlChar,
    ) -> *mut XmlNode;
    pub fn xmlNewTextChild(
        parent: *mut XmlNode,
        ns: *mut XmlNs,
        name: *const xmlChar,
        content: *const xmlChar,
    ) -> *mut XmlNode;
    pub fn xmlNewNsProp(
        node: *mut XmlNode,
        ns: *mut XmlNs,
        name: *const xmlChar,
        value: *const xmlChar,
    ) -> *mut XmlAttr;
    pub fn xmlEncodeEntitiesReentrant(doc: *mut XmlDoc, input: *const xmlChar) -> *mut xmlChar;

    // XMLReader
    pub fn xmlParserInputBufferCreateMem(
        mem: *const c_char,
        size: c_int,
        enc: c_int,
    ) -> *mut xmlParserInputBuffer;
    pub fn xmlFreeParserInputBuffer(input: *mut xmlParserInputBuffer);
    pub fn xmlNewTextReader(
        input: *mut xmlParserInputBuffer,
        uri: *const c_char,
    ) -> *mut xmlTextReader;
    pub fn xmlTextReaderSetup(
        reader: *mut xmlTextReader,
        input: *mut xmlParserInputBuffer,
        url: *const c_char,
        encoding: *const c_char,
        options: c_int,
    ) -> c_int;
    pub fn xmlReaderForFile(
        filename: *const c_char,
        encoding: *const c_char,
        options: c_int,
    ) -> *mut xmlTextReader;
    pub fn xmlReaderForIO(
        ioread: Option<xmlInputReadCallback>,
        ioclose: Option<xmlInputCloseCallback>,
        ioctx: *mut c_void,
        url: *const c_char,
        encoding: *const c_char,
        options: c_int,
    ) -> *mut xmlTextReader;
    pub fn xmlFreeTextReader(reader: *mut xmlTextReader);
    pub fn xmlTextReaderRead(reader: *mut xmlTextReader) -> c_int;
    pub fn xmlTextReaderNext(reader: *mut xmlTextReader) -> c_int;
    pub fn xmlTextReaderExpand(reader: *mut xmlTextReader) -> *mut XmlNode;
    pub fn xmlTextReaderReadInnerXml(reader: *mut xmlTextReader) -> *mut xmlChar;
    pub fn xmlTextReaderReadOuterXml(reader: *mut xmlTextReader) -> *mut xmlChar;
    pub fn xmlTextReaderReadString(reader: *mut xmlTextReader) -> *mut xmlChar;
    pub fn xmlTextReaderGetAttribute(
        reader: *mut xmlTextReader,
        name: *const xmlChar,
    ) -> *mut xmlChar;
    pub fn xmlTextReaderGetAttributeNo(reader: *mut xmlTextReader, no: c_int) -> *mut xmlChar;
    pub fn xmlTextReaderGetAttributeNs(
        reader: *mut xmlTextReader,
        local: *const xmlChar,
        ns: *const xmlChar,
    ) -> *mut xmlChar;
    pub fn xmlTextReaderLookupNamespace(
        reader: *mut xmlTextReader,
        prefix: *const xmlChar,
    ) -> *mut xmlChar;
    pub fn xmlTextReaderMoveToAttribute(reader: *mut xmlTextReader, name: *const xmlChar) -> c_int;
    pub fn xmlTextReaderMoveToAttributeNo(reader: *mut xmlTextReader, no: c_int) -> c_int;
    pub fn xmlTextReaderMoveToAttributeNs(
        reader: *mut xmlTextReader,
        local: *const xmlChar,
        ns: *const xmlChar,
    ) -> c_int;
    pub fn xmlTextReaderMoveToElement(reader: *mut xmlTextReader) -> c_int;
    pub fn xmlTextReaderMoveToFirstAttribute(reader: *mut xmlTextReader) -> c_int;
    pub fn xmlTextReaderMoveToNextAttribute(reader: *mut xmlTextReader) -> c_int;
    pub fn xmlTextReaderGetParserProp(reader: *mut xmlTextReader, prop: c_int) -> c_int;
    pub fn xmlTextReaderSetParserProp(
        reader: *mut xmlTextReader,
        prop: c_int,
        value: c_int,
    ) -> c_int;
    pub fn xmlTextReaderIsValid(reader: *mut xmlTextReader) -> c_int;
    pub fn xmlTextReaderSchemaValidate(reader: *mut xmlTextReader, xsd: *const c_char) -> c_int;
    pub fn xmlTextReaderRelaxNGSetSchema(
        reader: *mut xmlTextReader,
        schema: *mut xmlRelaxNG,
    ) -> c_int;
    pub fn xmlTextReaderAttributeCount(reader: *mut xmlTextReader) -> c_int;
    pub fn xmlTextReaderDepth(reader: *mut xmlTextReader) -> c_int;
    pub fn xmlTextReaderHasAttributes(reader: *mut xmlTextReader) -> c_int;
    pub fn xmlTextReaderHasValue(reader: *mut xmlTextReader) -> c_int;
    pub fn xmlTextReaderIsDefault(reader: *mut xmlTextReader) -> c_int;
    pub fn xmlTextReaderIsEmptyElement(reader: *mut xmlTextReader) -> c_int;
    pub fn xmlTextReaderNodeType(reader: *mut xmlTextReader) -> c_int;
    pub fn xmlTextReaderConstBaseUri(reader: *mut xmlTextReader) -> *const xmlChar;
    pub fn xmlTextReaderConstLocalName(reader: *mut xmlTextReader) -> *const xmlChar;
    pub fn xmlTextReaderConstName(reader: *mut xmlTextReader) -> *const xmlChar;
    pub fn xmlTextReaderConstNamespaceUri(reader: *mut xmlTextReader) -> *const xmlChar;
    pub fn xmlTextReaderConstPrefix(reader: *mut xmlTextReader) -> *const xmlChar;
    pub fn xmlTextReaderConstValue(reader: *mut xmlTextReader) -> *const xmlChar;
    pub fn xmlTextReaderConstXmlLang(reader: *mut xmlTextReader) -> *const xmlChar;

    // xmlwriter
    pub fn xmlNewTextWriter(out: *mut xmlOutputBuffer) -> *mut xmlTextWriter;
    pub fn xmlNewTextWriterFilename(uri: *const c_char, compression: c_int) -> *mut xmlTextWriter;
    pub fn xmlFreeTextWriter(writer: *mut xmlTextWriter);
    pub fn xmlTextWriterFlush(writer: *mut xmlTextWriter) -> c_int;
    pub fn xmlTextWriterSetIndent(writer: *mut xmlTextWriter, indent: c_int) -> c_int;
    pub fn xmlTextWriterSetIndentString(writer: *mut xmlTextWriter, s: *const xmlChar) -> c_int;
    pub fn xmlTextWriterStartDocument(
        writer: *mut xmlTextWriter,
        version: *const c_char,
        encoding: *const c_char,
        standalone: *const c_char,
    ) -> c_int;
    pub fn xmlTextWriterEndDocument(writer: *mut xmlTextWriter) -> c_int;
    pub fn xmlTextWriterStartComment(writer: *mut xmlTextWriter) -> c_int;
    pub fn xmlTextWriterEndComment(writer: *mut xmlTextWriter) -> c_int;
    pub fn xmlTextWriterWriteComment(writer: *mut xmlTextWriter, content: *const xmlChar) -> c_int;
    pub fn xmlTextWriterStartElement(writer: *mut xmlTextWriter, name: *const xmlChar) -> c_int;
    pub fn xmlTextWriterStartElementNS(
        writer: *mut xmlTextWriter,
        prefix: *const xmlChar,
        name: *const xmlChar,
        uri: *const xmlChar,
    ) -> c_int;
    pub fn xmlTextWriterEndElement(writer: *mut xmlTextWriter) -> c_int;
    pub fn xmlTextWriterFullEndElement(writer: *mut xmlTextWriter) -> c_int;
    pub fn xmlTextWriterWriteElement(
        writer: *mut xmlTextWriter,
        name: *const xmlChar,
        content: *const xmlChar,
    ) -> c_int;
    pub fn xmlTextWriterWriteElementNS(
        writer: *mut xmlTextWriter,
        prefix: *const xmlChar,
        name: *const xmlChar,
        uri: *const xmlChar,
        content: *const xmlChar,
    ) -> c_int;
    pub fn xmlTextWriterWriteRaw(writer: *mut xmlTextWriter, content: *const xmlChar) -> c_int;
    pub fn xmlTextWriterWriteString(writer: *mut xmlTextWriter, content: *const xmlChar) -> c_int;
    pub fn xmlTextWriterStartAttribute(writer: *mut xmlTextWriter, name: *const xmlChar) -> c_int;
    pub fn xmlTextWriterStartAttributeNS(
        writer: *mut xmlTextWriter,
        prefix: *const xmlChar,
        name: *const xmlChar,
        uri: *const xmlChar,
    ) -> c_int;
    pub fn xmlTextWriterEndAttribute(writer: *mut xmlTextWriter) -> c_int;
    pub fn xmlTextWriterWriteAttribute(
        writer: *mut xmlTextWriter,
        name: *const xmlChar,
        content: *const xmlChar,
    ) -> c_int;
    pub fn xmlTextWriterWriteAttributeNS(
        writer: *mut xmlTextWriter,
        prefix: *const xmlChar,
        name: *const xmlChar,
        uri: *const xmlChar,
        content: *const xmlChar,
    ) -> c_int;
    pub fn xmlTextWriterStartPI(writer: *mut xmlTextWriter, target: *const xmlChar) -> c_int;
    pub fn xmlTextWriterEndPI(writer: *mut xmlTextWriter) -> c_int;
    pub fn xmlTextWriterWritePI(
        writer: *mut xmlTextWriter,
        target: *const xmlChar,
        content: *const xmlChar,
    ) -> c_int;
    pub fn xmlTextWriterStartCDATA(writer: *mut xmlTextWriter) -> c_int;
    pub fn xmlTextWriterEndCDATA(writer: *mut xmlTextWriter) -> c_int;
    pub fn xmlTextWriterWriteCDATA(writer: *mut xmlTextWriter, content: *const xmlChar) -> c_int;
    pub fn xmlTextWriterStartDTD(
        writer: *mut xmlTextWriter,
        name: *const xmlChar,
        pubid: *const xmlChar,
        sysid: *const xmlChar,
    ) -> c_int;
    pub fn xmlTextWriterEndDTD(writer: *mut xmlTextWriter) -> c_int;
    pub fn xmlTextWriterWriteDTD(
        writer: *mut xmlTextWriter,
        name: *const xmlChar,
        pubid: *const xmlChar,
        sysid: *const xmlChar,
        subset: *const xmlChar,
    ) -> c_int;
    pub fn xmlTextWriterStartDTDElement(writer: *mut xmlTextWriter, name: *const xmlChar) -> c_int;
    pub fn xmlTextWriterEndDTDElement(writer: *mut xmlTextWriter) -> c_int;
    pub fn xmlTextWriterWriteDTDElement(
        writer: *mut xmlTextWriter,
        name: *const xmlChar,
        content: *const xmlChar,
    ) -> c_int;
    pub fn xmlTextWriterStartDTDAttlist(writer: *mut xmlTextWriter, name: *const xmlChar) -> c_int;
    pub fn xmlTextWriterEndDTDAttlist(writer: *mut xmlTextWriter) -> c_int;
    pub fn xmlTextWriterWriteDTDAttlist(
        writer: *mut xmlTextWriter,
        name: *const xmlChar,
        content: *const xmlChar,
    ) -> c_int;
    pub fn xmlTextWriterStartDTDEntity(
        writer: *mut xmlTextWriter,
        pe: c_int,
        name: *const xmlChar,
    ) -> c_int;
    pub fn xmlTextWriterEndDTDEntity(writer: *mut xmlTextWriter) -> c_int;
    pub fn xmlTextWriterWriteDTDEntity(
        writer: *mut xmlTextWriter,
        pe: c_int,
        name: *const xmlChar,
        pubid: *const xmlChar,
        sysid: *const xmlChar,
        ndataid: *const xmlChar,
        content: *const xmlChar,
    ) -> c_int;

    // shim.c
    pub static rhp_xml_error_layout: [usize; 14];
    pub fn rhp_xml_header_version() -> c_int;
    pub fn rhp_xml_header_dotted_version() -> *const c_char;
    pub fn rhp_xml_loaded_version() -> *const c_char;
    pub fn rhp_xml_constants() -> *const RhpXmlConst;
    pub fn rhp_xml_process_init();
    pub fn rhp_xml_set_msg_sink(
        sink: Option<unsafe extern "C" fn(c_int, *mut c_void, *const c_char, usize)>,
    );
    pub fn rhp_xml_set_generic_error(on: c_int);
    pub fn rhp_xml_ctxt_use_php_errors(ctxt: Ctxt);
    pub fn rhp_xml_ctxt_input_pos(
        ctxt: Ctxt,
        filename: *mut *const c_char,
        line: *mut c_int,
        col: *mut c_int,
    ) -> c_int;
    pub fn rhp_xml_ctxt_byte_index(ctxt: Ctxt) -> c_long;
    pub fn rhp_xml_ctxt_input_cur(ctxt: Ctxt, base: *mut *const xmlChar) -> *const xmlChar;
    pub fn rhp_xml_ctxt_errno(ctxt: Ctxt) -> c_int;
    pub fn rhp_xml_ctxt_set_errno(ctxt: Ctxt, err: c_int);
    pub fn rhp_xml_ctxt_in_subset(ctxt: Ctxt) -> c_int;
    pub fn rhp_xml_ctxt_in_content(ctxt: Ctxt) -> c_int;
    pub fn rhp_xml_ctxt_doc(ctxt: Ctxt) -> *mut xmlDoc;
    pub fn rhp_xml_ctxt_set_well_formed(ctxt: Ctxt, v: c_int);
    pub fn rhp_xml_ctxt_set_sax_initialized(ctxt: Ctxt, v: c_uint);
    pub fn rhp_xml_ctxt_loader_info(
        ctxt: Ctxt,
        directory: *mut *const c_char,
        int_sub_name: *mut *const xmlChar,
        ext_sub_uri: *mut *const xmlChar,
        ext_sub_system: *mut *const xmlChar,
    );
    pub fn rhp_xml_ctxt_set_huge(ctxt: Ctxt, huge: c_int);
    pub fn rhp_xml_sanitize_parse_ctxt_options(ctxt: Ctxt);
    pub fn rhp_xml_ctxt_free_doc(ctxt: Ctxt);
    pub fn rhp_xml_compat_push_parser(fns: *const RhpXmlSax, user: *mut c_void) -> Ctxt;
    pub fn rhp_xml_entity_fields(
        ent: *mut xmlEntity,
        name: *mut *const xmlChar,
        content: *mut *const xmlChar,
        system_id: *mut *const xmlChar,
        external_id: *mut *const xmlChar,
    ) -> c_int;
    pub fn rhp_xml_input_from_io(
        ctxt: Ctxt,
        read: xmlInputReadCallback,
        close: xmlInputCloseCallback,
        context: *mut c_void,
    ) -> *mut xmlParserInput;
    pub fn rhp_xml_output_hook_failed(encoder: *mut xmlCharEncodingHandler);
    pub fn rhp_xml_free(ptr: *mut c_void);
    pub fn rhp_xml_malloc(size: usize) -> *mut c_void;
    pub fn rhp_xml_uri_scheme(uri: *mut xmlURI) -> *const c_char;
    pub fn rhp_xml_uri_opaque(uri: *mut xmlURI) -> *const c_char;

    pub static rhp_xml_tree_layout: [usize; 64];
    pub fn rhp_xml_free_entity(entity: *mut XmlEntity);
    pub fn rhp_xml_set_save_no_empty_tags(v: c_int) -> c_int;
    pub fn rhp_xml_sanitize_globals(saved: *mut c_int);
    pub fn rhp_xml_restore_globals(saved: *const c_int);
    pub fn rhp_xml_output_buffer_error(out: *mut xmlOutputBuffer) -> c_int;
    pub fn rhp_xml_valid_ctxt_use_php_errors(ctxt: *mut xmlValidCtxt);
    pub fn rhp_xml_schema_parser_use_php_errors(ctxt: *mut xmlSchemaParserCtxt);
    pub fn rhp_xml_schema_valid_use_php_errors(ctxt: *mut xmlSchemaValidCtxt);
    pub fn rhp_xml_relaxng_parser_use_php_errors(ctxt: *mut xmlRelaxNGParserCtxt);
    pub fn rhp_xml_relaxng_valid_use_php_errors(ctxt: *mut xmlRelaxNGValidCtxt);
    pub fn rhp_xml_ctxt_directory(ctxt: Ctxt) -> *const c_char;
    pub fn rhp_xml_ctxt_set_directory(ctxt: Ctxt, dir: *mut c_char);
    pub fn rhp_xml_ctxt_well_formed(ctxt: Ctxt) -> c_int;
    pub fn rhp_xml_ctxt_take_doc(ctxt: Ctxt) -> *mut XmlDoc;
    pub fn rhp_xml_xpath_node(ctxt: *mut xmlXPathContext) -> *mut XmlNode;
    pub fn rhp_xml_xpath_set_node(ctxt: *mut xmlXPathContext, node: *mut XmlNode);
    pub fn rhp_xml_xpath_doc(ctxt: *mut xmlXPathContext) -> *mut XmlDoc;
    pub fn rhp_xml_xpath_namespaces(ctxt: *mut xmlXPathContext, nr: *mut c_int) -> *mut *mut XmlNs;
    pub fn rhp_xml_xpath_set_namespaces(
        ctxt: *mut xmlXPathContext,
        list: *mut *mut XmlNs,
        nr: c_int,
    );
    pub fn rhp_xml_xpath_user_data(ctxt: *mut xmlXPathContext) -> *mut c_void;
    pub fn rhp_xml_xpath_set_user_data(ctxt: *mut xmlXPathContext, data: *mut c_void);
    pub fn rhp_xml_xpath_parser_context(ctxt: *mut xmlXPathParserContext) -> *mut xmlXPathContext;
    pub fn rhp_xml_xpath_function(
        ctxt: *mut xmlXPathContext,
        uri: *mut *const xmlChar,
    ) -> *const xmlChar;
    pub fn rhp_xml_xpath_value_pop(ctxt: *mut xmlXPathParserContext) -> *mut XmlXPathObject;
    pub fn rhp_xml_xpath_value_push(ctxt: *mut xmlXPathParserContext, obj: *mut XmlXPathObject);
    pub fn rhp_xml_generic_error_msg(msg: *const c_char);
    pub fn rhp_xml_attribute_decl_default(attr: *mut c_void) -> *const xmlChar;
}

/// libxml2's `xmlFree` (a global function pointer, hence the shim).
///
/// # Safety
/// `p` was allocated by libxml2 (or is NULL).
pub unsafe fn xml_free<T>(p: *mut T) {
    if !p.is_null() {
        unsafe { rhp_xml_free(p.cast()) }
    }
}

/// Bytes of a libxml2-allocated string, which is then freed.
///
/// # Safety
/// `p` is NULL or a NUL-terminated string allocated by libxml2.
pub unsafe fn take_string(p: *mut xmlChar) -> Option<Vec<u8>> {
    if p.is_null() {
        return None;
    }
    let v = unsafe { cstr(p) }.to_vec();
    unsafe { rhp_xml_free(p.cast()) };
    Some(v)
}

/// The bytes of a NUL-terminated C string (empty for NULL).
///
/// # Safety
/// `p` is NULL or points to a NUL-terminated string that outlives the
/// returned slice.
pub unsafe fn cstr<'a, T>(p: *const T) -> &'a [u8] {
    if p.is_null() {
        return &[];
    }
    unsafe { std::ffi::CStr::from_ptr(p.cast()) }.to_bytes()
}

/// `b`'s address for a call that also takes its length, without copying:
/// never an empty slice's dangling pointer, which libxml2 2.13+ reads.
pub fn mem_ptr(b: &[u8]) -> *const c_char {
    if b.is_empty() {
        c"".as_ptr()
    } else {
        b.as_ptr().cast()
    }
}

/// `s` with a NUL terminator, for passing to C. A string with an embedded
/// NUL is cut there, as C would read it.
pub fn cbytes(s: &[u8]) -> Vec<u8> {
    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    let mut v = Vec::with_capacity(end + 1);
    v.extend_from_slice(&s[..end]);
    v.push(0);
    v
}

#[cfg(test)]
mod tests;
