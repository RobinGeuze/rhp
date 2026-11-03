//! A port of ext/xml/compat.c: libxml2 SAX handlers that imitate expat's
//! callbacks, calling the handlers in `super` that xml.c installed.

use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use super::Parser;
use crate::ext::libxml::ffi::guard;
use crate::ext::libxml::sys::{self, cstr, xmlChar};

/// The parser a SAX callback's user data points at.
///
/// # Safety
/// `user` is the `RefCell<Parser>` [`parser_create`] registered, kept alive
/// for the length of the parse by the caller.
unsafe fn parser<'a>(user: *mut c_void) -> &'a RefCell<Parser> {
    unsafe { &*(user as *const RefCell<Parser>) }
}

fn opt<'a>(p: *const xmlChar) -> Option<&'a [u8]> {
    (!p.is_null()).then(|| unsafe { cstr(p) })
}

/// qualify_namespace: `URI<sep>name`, or `name` without a URI. The
/// separator is the first byte of php's separator string, none if empty.
fn qualify(p: &Parser, name: &[u8], uri: Option<&[u8]>) -> Vec<u8> {
    match uri {
        Some(u) => {
            let mut q = u.to_vec();
            q.extend(p.ns_separator.first());
            q.extend_from_slice(name);
            q
        }
        None => name.to_vec(),
    }
}

/// start_element_emit_default: the start tag's source text, from the
/// parser's input (php's parse mode keeps all of it).
fn start_element_emit_default(p: &RefCell<Parser>) {
    let (has_default, ctxt) = {
        let s = p.borrow();
        (s.c.default, s.ctxt)
    };
    if !has_default {
        return;
    }
    let text = unsafe {
        let mut base = ptr::null();
        let end = sys::rhp_xml_ctxt_input_cur(ctxt, &mut base);
        let mut cur = end;
        while cur > base && *cur != b'<' {
            cur = cur.sub(1);
        }
        let len = end.offset_from(cur) as usize + 1;
        let mut t = std::slice::from_raw_parts(cur, len).to_vec();
        if *end == b'/' {
            // Keep start and end of an empty element split (gh20439_2).
            t[len - 1] = b'>';
        }
        t
    };
    super::default_handler(p, &text);
}

unsafe extern "C" fn start_element_handler(
    user: *mut c_void,
    name: *const xmlChar,
    attributes: *mut *const xmlChar,
) {
    guard((), || {
        let p = unsafe { parser(user) };
        if !p.borrow().c.element {
            start_element_emit_default(p);
            return;
        }
        let mut attrs = Vec::new();
        if !attributes.is_null() {
            let mut a = attributes;
            unsafe {
                while !(*a).is_null() {
                    let k = cstr(*a).to_vec();
                    let v = cstr(*a.add(1)).to_vec();
                    attrs.push((k, v));
                    a = a.add(2);
                }
            }
        }
        super::start_element_handler(p, unsafe { cstr(name) }, &attrs);
    })
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn start_element_handler_ns(
    user: *mut c_void,
    name: *const xmlChar,
    _prefix: *const xmlChar,
    uri: *const xmlChar,
    nb_namespaces: c_int,
    namespaces: *mut *const xmlChar,
    nb_attributes: c_int,
    _nb_defaulted: c_int,
    attributes: *mut *const xmlChar,
) {
    guard((), || {
        let p = unsafe { parser(user) };
        if nb_namespaces > 0 && p.borrow().c.start_ns {
            for i in 0..nb_namespaces as usize {
                let (prefix, href) =
                    unsafe { (*namespaces.add(2 * i), *namespaces.add(2 * i + 1)) };
                super::start_namespace_decl_handler(p, opt(prefix), opt(href));
            }
        }
        if !p.borrow().c.element {
            start_element_emit_default(p);
            return;
        }
        let (qualified, attrs) = {
            let s = p.borrow();
            let qualified = qualify(&s, unsafe { cstr(name) }, opt(uri));
            let mut attrs = Vec::new();
            if !attributes.is_null() {
                for i in 0..nb_attributes.max(0) as usize {
                    let a = unsafe { std::slice::from_raw_parts(attributes.add(5 * i), 5) };
                    let local = unsafe { cstr(a[0]) };
                    let k = if a[1].is_null() {
                        local.to_vec()
                    } else {
                        qualify(&s, local, opt(a[2]))
                    };
                    let v = unsafe {
                        std::slice::from_raw_parts(a[3], a[4].offset_from(a[3]) as usize).to_vec()
                    };
                    attrs.push((k, v));
                }
            }
            (qualified, attrs)
        };
        super::start_element_handler(p, &qualified, &attrs);
    })
}

unsafe extern "C" fn end_element_handler(user: *mut c_void, name: *const xmlChar) {
    guard((), || {
        let p = unsafe { parser(user) };
        let name = unsafe { cstr(name) };
        if !p.borrow().c.element {
            if p.borrow().c.default {
                super::default_handler(p, &[b"</".as_slice(), name, b">"].concat());
            }
            return;
        }
        super::end_element_handler(p, name);
    })
}

unsafe extern "C" fn end_element_handler_ns(
    user: *mut c_void,
    name: *const xmlChar,
    prefix: *const xmlChar,
    uri: *const xmlChar,
) {
    guard((), || {
        let p = unsafe { parser(user) };
        let name = unsafe { cstr(name) };
        if !p.borrow().c.element {
            if p.borrow().c.default {
                let text = match opt(prefix) {
                    Some(pre) => [b"</".as_slice(), pre, b":", name, b">"].concat(),
                    None => [b"</".as_slice(), name, b">"].concat(),
                };
                super::default_handler(p, &text);
            }
            return;
        }
        let qualified = qualify(&p.borrow(), name, opt(uri));
        super::end_element_handler(p, &qualified);
    })
}

unsafe extern "C" fn cdata_handler(user: *mut c_void, cdata: *const xmlChar, len: c_int) {
    guard((), || {
        let p = unsafe { parser(user) };
        let data = unsafe { std::slice::from_raw_parts(cdata, len.max(0) as usize) };
        if !p.borrow().c.cdata {
            if p.borrow().c.default {
                super::default_handler(p, data);
            }
            return;
        }
        super::character_data_handler(p, data);
    })
}

unsafe extern "C" fn pi_handler(user: *mut c_void, target: *const xmlChar, data: *const xmlChar) {
    guard((), || {
        let p = unsafe { parser(user) };
        if !p.borrow().c.pi {
            if p.borrow().c.default {
                // spprintf("%s") prints "(null)" for NULL.
                let d = opt(data).unwrap_or(b"(null)");
                let t = opt(target).unwrap_or(b"(null)");
                super::default_handler(p, &[b"<?".as_slice(), t, b" ", d, b"?>"].concat());
            }
            return;
        }
        super::processing_instruction_handler(p, opt(target), opt(data));
    })
}

unsafe extern "C" fn unparsed_entity_decl_handler(
    user: *mut c_void,
    name: *const xmlChar,
    pub_id: *const xmlChar,
    sys_id: *const xmlChar,
    notation: *const xmlChar,
) {
    guard((), || {
        let p = unsafe { parser(user) };
        if !p.borrow().c.unparsed {
            return;
        }
        super::unparsed_entity_decl_handler(
            p,
            [opt(name), None, opt(sys_id), opt(pub_id), opt(notation)],
        );
    })
}

unsafe extern "C" fn notation_decl_handler(
    user: *mut c_void,
    notation: *const xmlChar,
    pub_id: *const xmlChar,
    sys_id: *const xmlChar,
) {
    guard((), || {
        let p = unsafe { parser(user) };
        if !p.borrow().c.notation {
            return;
        }
        super::notation_decl_handler(p, [opt(notation), None, opt(sys_id), opt(pub_id)]);
    })
}

unsafe extern "C" fn comment_handler(user: *mut c_void, comment: *const xmlChar) {
    guard((), || {
        let p = unsafe { parser(user) };
        if p.borrow().c.default {
            let text = [b"<!--".as_slice(), unsafe { cstr(comment) }, b"-->"].concat();
            super::default_handler(p, &text);
        }
    })
}

fn external_entity_ref(
    p: &RefCell<Parser>,
    names: Option<&[u8]>,
    sys_id: Option<&[u8]>,
    pub_id: Option<&[u8]>,
) {
    if !p.borrow().c.extent {
        return;
    }
    if super::external_entity_ref_handler(p, [names, Some(b""), sys_id, pub_id]) == 0 {
        let ctxt = p.borrow().ctxt;
        unsafe {
            sys::xmlStopParser(ctxt);
            sys::rhp_xml_ctxt_set_errno(ctxt, super::XML_ERROR_EXTERNAL_ENTITY_HANDLING);
        }
    }
}

unsafe extern "C" fn get_entity(user: *mut c_void, name: *const xmlChar) -> *mut sys::xmlEntity {
    guard(ptr::null_mut(), || {
        let p = unsafe { parser(user) };
        let ctxt = p.borrow().ctxt;
        let mut ret = ptr::null_mut();
        if unsafe { sys::rhp_xml_ctxt_in_subset(ctxt) } != 0 {
            return ret;
        }
        unsafe {
            ret = sys::xmlGetPredefinedEntity(name);
            if ret.is_null() {
                ret = sys::xmlGetDocEntity(sys::rhp_xml_ctxt_doc(ctxt), name);
            }
        }
        let (mut ename, mut content, mut system_id, mut external_id) =
            (ptr::null(), ptr::null(), ptr::null(), ptr::null());
        let etype = if ret.is_null() {
            0
        } else {
            unsafe {
                sys::rhp_xml_entity_fields(
                    ret,
                    &mut ename,
                    &mut content,
                    &mut system_id,
                    &mut external_id,
                )
            }
        };
        if !(ret.is_null() || unsafe { sys::rhp_xml_ctxt_in_content(ctxt) } != 0) {
            return ret;
        }
        let internal = matches!(
            etype,
            sys::XML_INTERNAL_GENERAL_ENTITY
                | sys::XML_INTERNAL_PARAMETER_ENTITY
                | sys::XML_INTERNAL_PREDEFINED_ENTITY
        );
        if ret.is_null() || internal {
            let (has_default, has_cdata) = {
                let s = p.borrow();
                (s.c.default, s.c.cdata)
            };
            // Predefined entities expand unless there is no cdata handler.
            if has_default
                && !(!ret.is_null() && etype == sys::XML_INTERNAL_PREDEFINED_ENTITY && has_cdata)
            {
                let text = [b"&".as_slice(), unsafe { cstr(name) }, b";"].concat();
                super::default_handler(p, &text);
            } else if has_cdata && !ret.is_null() {
                // expat expands internal entities for the cdata handler when
                // there is no default handler.
                super::character_data_handler(p, unsafe { cstr(content) });
            }
        } else if etype == sys::XML_EXTERNAL_GENERAL_PARSED_ENTITY {
            external_entity_ref(p, opt(ename), opt(system_id), opt(external_id));
        }
        ret
    })
}

/// XML_ParserCreate_MM, after php allocated the struct: a push parser with
/// the compat handlers. `sep` is php's separator string for a namespace
/// parser.
pub(super) fn parser_create(p: &RefCell<Parser>, sep: Option<Vec<u8>>) {
    let fns = sys::RhpXmlSax {
        get_entity,
        notation_decl: notation_decl_handler,
        unparsed_entity_decl: unparsed_entity_decl_handler,
        start_element: start_element_handler,
        end_element: end_element_handler,
        characters: cdata_handler,
        processing_instruction: pi_handler,
        comment: comment_handler,
        start_element_ns: start_element_handler_ns,
        end_element_ns: end_element_handler_ns,
    };
    let user = p as *const RefCell<Parser> as *mut c_void;
    let ctxt = unsafe { sys::rhp_xml_compat_push_parser(&fns, user) };
    if ctxt.is_null() {
        return;
    }
    unsafe {
        sys::rhp_xml_sanitize_parse_ctxt_options(ctxt);
        sys::xmlCtxtUseOptions(ctxt, sys::XML_PARSE_OLDSAX | sys::XML_PARSE_NOENT);
        sys::rhp_xml_ctxt_set_well_formed(ctxt, 0);
    }
    let mut s = p.borrow_mut();
    s.ctxt = ctxt;
    match sep {
        Some(sep) => {
            // The SAX2 flag stays set by XML_SAX2_MAGIC.
            s.use_namespace = true;
            s.ns_separator = sep;
        }
        // libxml2 needs XML_SAX2_MAGIC to create the context; reset it.
        None => unsafe { sys::rhp_xml_ctxt_set_sax_initialized(ctxt, 1) },
    }
}

/// XML_Parse
pub(super) fn xml_parse(ctxt: sys::Ctxt, data: &[u8], is_final: bool) -> c_int {
    let error = unsafe {
        sys::xmlParseChunk(
            ctxt,
            data.as_ptr() as *const c_char,
            data.len() as c_int,
            is_final as c_int,
        )
    };
    if error == 0 {
        let e = unsafe { sys::xmlCtxtGetLastError(ctxt.cast()) };
        return (e.is_null() || unsafe { (*e).level } <= sys::XML_ERR_WARNING) as c_int;
    }
    0
}

/// compat.c error_mapping, indexed by libxml2 error code.
const ERROR_MAPPING: &[&str] = &[
    "No error",
    "No memory",
    "Invalid document start",
    "Empty document",
    "Not well-formed (invalid token)",
    "Invalid document end",
    "Invalid hexadecimal character reference",
    "Invalid decimal character reference",
    "Invalid character reference",
    "Invalid character",
    "XML_ERR_CHARREF_AT_EOF",
    "XML_ERR_CHARREF_IN_PROLOG",
    "XML_ERR_CHARREF_IN_EPILOG",
    "XML_ERR_CHARREF_IN_DTD",
    "XML_ERR_ENTITYREF_AT_EOF",
    "XML_ERR_ENTITYREF_IN_PROLOG",
    "XML_ERR_ENTITYREF_IN_EPILOG",
    "XML_ERR_ENTITYREF_IN_DTD",
    "PEReference at end of document",
    "PEReference in prolog",
    "PEReference in epilog",
    "PEReference: forbidden within markup decl in internal subset",
    "XML_ERR_ENTITYREF_NO_NAME",
    "EntityRef: expecting ';'",
    "PEReference: no name",
    "PEReference: expecting ';'",
    "Undeclared entity error",
    "Undeclared entity warning",
    "Unparsed Entity",
    "XML_ERR_ENTITY_IS_EXTERNAL",
    "XML_ERR_ENTITY_IS_PARAMETER",
    "Unknown encoding",
    "Unsupported encoding",
    "String not started expecting ' or \"",
    "String not closed expecting \" or '",
    "Namespace declaration error",
    "EntityValue: \" or ' expected",
    "EntityValue: \" or ' expected",
    "< in attribute",
    "Attribute not started",
    "Attribute not finished",
    "Attribute without value",
    "Attribute redefined",
    "SystemLiteral \" or ' expected",
    "SystemLiteral \" or ' expected",
    // "XML_ERR_COMMENT_NOT_STARTED" is left out on purpose, as in php.
    "Comment not finished",
    "Processing Instruction not started",
    "Processing Instruction not finished",
    "NOTATION: Name expected here",
    "'>' required to close NOTATION declaration",
    "'(' required to start ATTLIST enumeration",
    "'(' required to start ATTLIST enumeration",
    "MixedContentDecl : '|' or ')*' expected",
    "XML_ERR_MIXED_NOT_FINISHED",
    "ELEMENT in DTD not started",
    "ELEMENT in DTD not finished",
    "XML declaration not started",
    "XML declaration not finished",
    "XML_ERR_CONDSEC_NOT_STARTED",
    "XML conditional section not closed",
    "Content error in the external subset",
    "DOCTYPE not finished",
    "Sequence ']]>' not allowed in content",
    "CDATA not finished",
    "Reserved XML Name",
    "Space required",
    "XML_ERR_SEPARATOR_REQUIRED",
    "NmToken expected in ATTLIST enumeration",
    "XML_ERR_NAME_REQUIRED",
    "MixedContentDecl : '#PCDATA' expected",
    "SYSTEM or PUBLIC, the URI is missing",
    "PUBLIC, the Public Identifier is missing",
    "< required",
    "> required",
    "</ required",
    "= required",
    "Mismatched tag",
    "Tag not finished",
    "standalone accepts only 'yes' or 'no'",
    "Invalid XML encoding name",
    "Comment must not contain '--' (double-hyphen)",
    "Invalid encoding",
    "external parsed entities cannot be standalone",
    "XML conditional section '[' expected",
    "Entity value required",
    "chunk is not well balanced",
    "extra content at the end of well balanced chunk",
    "XML_ERR_ENTITY_CHAR_ERROR",
    "PEReferences forbidden in internal subset",
    "Detected an entity reference loop",
    "XML_ERR_ENTITY_BOUNDARY",
    "Invalid URI",
    "Fragment not allowed",
    "XML_WAR_CATALOG_PI",
    "XML_ERR_NO_DTD",
    "conditional section INCLUDE or IGNORE keyword expected",
    "Version in XML Declaration missing",
    "XML_WAR_UNKNOWN_VERSION",
    "XML_WAR_LANG_VALUE",
    "XML_WAR_NS_URI",
    "XML_WAR_NS_URI_RELATIVE",
    "Missing encoding in text declaration",
];

/// XML_ErrorString
pub(super) fn error_string(code: i64) -> &'static str {
    // xml_error_string passes the code as a C int.
    usize::try_from(code as c_int)
        .ok()
        .and_then(|i| ERROR_MAPPING.get(i))
        .copied()
        .unwrap_or("Unknown")
}
