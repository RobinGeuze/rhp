//! The libxml2 functions rhp calls, declared by hand (no bindgen), plus
//! `shim.c`'s accessors. build.rs links the libxml2 that php links
//! (docs/design/libraries.md) and compiles the shim against its headers.
//!
//! Structs are opaque: their layouts differ between libxml2 releases, so
//! fields are read through the shim. The one exception is [`XmlErrorRaw`],
//! which has not changed since 2.9 (a test checks it against the headers).

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
    xmlDoc,
    xmlEntity,
    xmlURI,
    xmlTextWriter,
    xmlCharEncodingHandler
);

pub type xmlChar = u8;
pub type Ctxt = *mut xmlParserCtxt;

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
    pub fn rhp_xml_uri_scheme(uri: *mut xmlURI) -> *const c_char;
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
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn error_mirror_matches_the_headers() {
        let c = unsafe { rhp_xml_error_layout };
        let rust = [
            size_of::<XmlErrorRaw>(),
            offset_of!(XmlErrorRaw, domain),
            offset_of!(XmlErrorRaw, code),
            offset_of!(XmlErrorRaw, message),
            offset_of!(XmlErrorRaw, level),
            offset_of!(XmlErrorRaw, file),
            offset_of!(XmlErrorRaw, line),
            offset_of!(XmlErrorRaw, str1),
            offset_of!(XmlErrorRaw, str2),
            offset_of!(XmlErrorRaw, str3),
            offset_of!(XmlErrorRaw, int1),
            offset_of!(XmlErrorRaw, int2),
            offset_of!(XmlErrorRaw, ctxt),
            offset_of!(XmlErrorRaw, node),
        ];
        assert_eq!(c, rust);
    }
}
