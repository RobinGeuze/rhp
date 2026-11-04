//! DOMDocument's methods (php-src ext/dom/document.c): node factories,
//! import and adoption, parsing and serialising (XML and libxml2's HTML4
//! parser), and DTD, XML Schema, RelaxNG and XInclude processing.

use std::ffi::c_int;
use std::ptr;

use rhp_values::{ArrayKey, Value};

use super::util::*;
use super::{
    arg, bytes, create_object, ensure_intern, get_intern, get_obj, int, opt_bytes, throw, Cx,
    DomObject, Kind, INVALID_CHARACTER_ERR, INVALID_STATE_ERR, NAMESPACE_ERR, NOT_SUPPORTED_ERR,
    WRONG_DOCUMENT_ERR,
};
use crate::ext::libxml::node::{self as lxn, ClassType};
use crate::ext::libxml::sys::{self, XmlDoc, XmlNode};
use crate::Thrown;

const INT_MAX: i64 = i32::MAX as i64;

// xmlParserOption / xmlSaveOption / XML_SCHEMA_VAL_VC_I_CREATE
const XML_PARSE_RECOVER: i64 = 1 << 0;
const XML_PARSE_NOENT: i64 = 1 << 1;
const XML_PARSE_DTDATTR: i64 = 1 << 3;
const XML_PARSE_DTDVALID: i64 = 1 << 4;
const XML_PARSE_NOBLANKS: i64 = 1 << 8;
const XML_PARSE_NOXINCNODE: i64 = 1 << 15;
const XML_SAVE_FORMAT: c_int = 1 << 0;
const XML_SAVE_NO_DECL: i64 = 1 << 1;
const XML_SAVE_AS_XML: c_int = 1 << 5;
const LIBXML_SAVE_NOEMPTYTAG: i64 = 1 << 2;
const XML_SCHEMA_VAL_VC_I_CREATE: i64 = 1 << 0;

fn value_error(cx: &Cx, n: usize, pname: &str, msg: &str) -> Thrown {
    crate::ext::zpp::value_error(&cx.fname, n, pname, msg)
}

/// The document node of `this`.
fn this_doc(cx: &mut Cx, this: &Value) -> Result<(std::rc::Rc<DomObject>, *mut XmlDoc), Thrown> {
    let (d, n) = get_obj(cx, this)?;
    Ok((d, n.cast()))
}

pub fn construct(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let version = opt_bytes(args, 0).map(|v| c(&v));
    let encoding = bytes(args, 1);
    let docp = unsafe { sys::xmlNewDoc(version.as_ref().map_or(ptr::null(), |v| v.as_ptr())) };
    if docp.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    if !encoding.is_empty() {
        let ce = c(&encoding);
        unsafe { (*docp).encoding = sys::xmlStrdup(ce.as_ptr()) };
    }
    let intern = ensure_intern(this, || Kind::Node).expect("a DOMDocument");
    replace_document(&intern, docp, false);
    Ok(Value::Null)
}

/// The old document's settings a load carries over: props, class type and
/// modification counter.
type Kept = (Option<Box<lxn::DocProps>>, ClassType, usize);

/// Swap the document an object wraps (DOMDocument::__construct and
/// php_dom_finish_loading_document); a load takes the old props along.
fn replace_document(intern: &DomObject, newdoc: *mut XmlDoc, load: bool) -> Option<Kept> {
    let olddoc = intern.node();
    let mut kept = None;
    if !olddoc.is_null() {
        let doc_ptr = intern.document();
        let (class_type, modnr, props) = unsafe {
            (
                (*doc_ptr).class_type,
                (*doc_ptr).cache_tag,
                if load {
                    (*doc_ptr).doc_props.take()
                } else {
                    None
                },
            )
        };
        lxn::decrement_node_ptr(&intern.base);
        let rc = lxn::decrement_doc_ref(&intern.base);
        if rc != 0 {
            unsafe { (*olddoc)._private = ptr::null_mut() };
        }
        kept = Some((props, class_type, modnr));
    }
    intern.base.document.set(ptr::null_mut());
    lxn::increment_doc_ref(&intern.base, newdoc);
    let me: *const lxn::NodeObject = &intern.base;
    lxn::increment_node_ptr(&intern.base, newdoc.cast(), me);
    kept
}

pub fn create_element(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let value = opt_bytes(args, 1).map(|v| c(&v));
    let (intern, docp) = this_doc(cx, this)?;
    if !valid_name(&name) {
        return throw(cx, INVALID_CHARACTER_ERR, intern.strict());
    }
    let cn = c(&name);
    let vp = value.as_ref().map_or(ptr::null(), |v| v.as_ptr());
    let node = cx.with_libxml_owned(
        || unsafe { sys::xmlNewDocNode(docp, ptr::null_mut(), cn.as_ptr(), vp) },
        free_node,
    )?;
    if node.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    create_object(cx, node, Some(&intern))
}

pub fn create_document_fragment(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let (intern, docp) = this_doc(cx, this)?;
    let node = unsafe { sys::xmlNewDocFragment(docp) };
    if node.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    create_object(cx, node, Some(&intern))
}

pub fn create_text_node(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let value = c(&bytes(args, 0));
    let (intern, docp) = this_doc(cx, this)?;
    let node = unsafe { sys::xmlNewDocText(docp, value.as_ptr()) };
    if node.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    create_object(cx, node, Some(&intern))
}

pub fn create_comment(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let value = c(&bytes(args, 0));
    let (intern, docp) = this_doc(cx, this)?;
    let node = unsafe { sys::xmlNewDocComment(docp, value.as_ptr()) };
    if node.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    create_object(cx, node, Some(&intern))
}

pub fn create_cdata_section(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let value = bytes(args, 0);
    let (intern, docp) = this_doc(cx, this)?;
    let node = unsafe { sys::xmlNewCDataBlock(docp, nul(&value).as_ptr(), value.len() as c_int) };
    if node.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    create_object(cx, node, Some(&intern))
}

pub fn create_processing_instruction(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let value = opt_bytes(args, 1).map(|v| c(&v));
    let (intern, docp) = this_doc(cx, this)?;
    if !valid_name(&name) {
        return throw(cx, INVALID_CHARACTER_ERR, intern.strict());
    }
    let cn = c(&name);
    let vp = value.as_ref().map_or(ptr::null(), |v| v.as_ptr());
    let node = unsafe { sys::xmlNewDocPI(docp, cn.as_ptr(), vp) };
    if node.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    create_object(cx, node, Some(&intern))
}

pub fn create_attribute(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    if name.contains(&0) {
        return Err(value_error(
            cx,
            1,
            "localName",
            "must not contain any null bytes",
        ));
    }
    let (intern, docp) = this_doc(cx, this)?;
    if !valid_name(&name) {
        return throw(cx, INVALID_CHARACTER_ERR, intern.strict());
    }
    let cn = c(&name);
    let node = unsafe { sys::xmlNewDocProp(docp, cn.as_ptr(), ptr::null()) };
    if node.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    create_object(cx, node.cast(), Some(&intern))
}

pub fn create_entity_reference(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let (intern, docp) = this_doc(cx, this)?;
    if !valid_name(&name) {
        return throw(cx, INVALID_CHARACTER_ERR, intern.strict());
    }
    let cn = c(&name);
    let node = unsafe { sys::xmlNewReference(docp, cn.as_ptr()) };
    if node.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    create_object(cx, node, Some(&intern))
}

pub fn import_node(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, docp) = this_doc(cx, this)?;
    let (_, nodep) = get_obj(cx, &arg(args, 0))?;
    let recursive = super::boolean(args, 1);
    unsafe {
        let t = (*nodep).type_;
        if t == sys::XML_HTML_DOCUMENT_NODE || t == sys::XML_DOCUMENT_NODE {
            cx.warn("Cannot import: Node Type Not Supported")?;
            return Ok(Value::Bool(false));
        }
        let retnodep;
        if (*nodep).doc == docp {
            retnodep = nodep;
        } else {
            retnodep = super::util::clone_node(nodep, docp, recursive);
            if retnodep.is_null() {
                return Ok(Value::Bool(false));
            }
            if (*retnodep).type_ == sys::XML_ATTRIBUTE_NODE
                && !(*nodep).ns.is_null()
                && (*retnodep).ns.is_null()
            {
                let root = sys::xmlDocGetRootElement(docp);
                let mut nsptr = sys::xmlSearchNsByHref(docp, root, (*(*nodep).ns).href);
                if nsptr.is_null() || (*nsptr).prefix.is_null() {
                    let href = b((*(*nodep).ns).href).to_vec();
                    let prefix = if (*(*nodep).ns).prefix.is_null() {
                        None
                    } else {
                        Some(b((*(*nodep).ns).prefix).to_vec())
                    };
                    nsptr = get_ns(root, &href, prefix.as_deref()).unwrap_or(ptr::null_mut());
                    if !nsptr.is_null() && root.is_null() {
                        lxn::set_old_ns(docp, nsptr);
                    }
                }
                (*retnodep).ns = nsptr;
            }
        }
        create_object(cx, retnodep, Some(&intern))
    }
}

pub fn create_element_ns(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let uri = opt_bytes(args, 0);
    let name = bytes(args, 1);
    let value = opt_bytes(args, 2).map(|v| c(&v));
    let vp = value.as_ref().map_or(ptr::null(), |v| v.as_ptr());
    let (intern, docp) = this_doc(cx, this)?;
    let mut nodep: *mut XmlNode = ptr::null_mut();
    let errorcode = match check_qname(&name, uri.as_ref().map_or(0, |u| u.len())) {
        Err(code) => code,
        Ok((localname, prefix)) => {
            if valid_name(&localname) {
                let cl = c(&localname);
                unsafe {
                    nodep = cx.with_libxml_owned(
                        || sys::xmlNewDocNode(docp, ptr::null_mut(), cl.as_ptr(), vp),
                        free_node,
                    )?;
                    if nodep.is_null() {
                        throw(cx, INVALID_STATE_ERR, true)?;
                    }
                    let mut code = 0;
                    if let Some(u) = &uri {
                        let cu = c(u);
                        let mut nsptr = sys::xmlSearchNsByHref((*nodep).doc, nodep, cu.as_ptr());
                        if nsptr.is_null() {
                            match get_ns(nodep, u, prefix.as_deref()) {
                                Ok(ns) => nsptr = ns,
                                Err(e) => code = e,
                            }
                        }
                        (*nodep).ns = nsptr;
                    }
                    code
                }
            } else {
                INVALID_CHARACTER_ERR
            }
        }
    };
    if errorcode != 0 {
        if !nodep.is_null() {
            unsafe { sys::xmlFreeNode(nodep) };
        }
        return throw(cx, errorcode, intern.strict());
    }
    create_object(cx, nodep, Some(&intern))
}

pub fn create_attribute_ns(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let uri = opt_bytes(args, 0);
    let name = bytes(args, 1);
    let (intern, docp) = this_doc(cx, this)?;
    let root = unsafe { sys::xmlDocGetRootElement(docp) };
    if root.is_null() {
        cx.warn("Document Missing Root Element")?;
        return Ok(Value::Bool(false));
    }
    let (localname, prefix) = match validate_and_extract(uri.as_deref(), &name) {
        Ok(p) => p,
        Err(_) => return throw(cx, NAMESPACE_ERR, intern.strict()),
    };
    let cl = c(&localname);
    let nodep: *mut XmlNode = unsafe { sys::xmlNewDocProp(docp, cl.as_ptr(), ptr::null()).cast() };
    if nodep.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    let mut errorcode = 0;
    if let Some(u) = uri.as_ref().filter(|u| !u.is_empty()) {
        let cu = c(u);
        unsafe {
            let mut nsptr = sys::xmlSearchNsByHref(docp, root, cu.as_ptr());
            if name == b"xmlns" || prefix.as_deref() == Some(b"xml") {
                if nsptr.is_null() {
                    let cp = prefix.as_deref().map(c);
                    nsptr = sys::xmlNewNs(
                        ptr::null_mut(),
                        cu.as_ptr(),
                        cp.as_ref().map_or(ptr::null(), |p| p.as_ptr()),
                    );
                    lxn::set_old_ns(docp, nsptr);
                }
            } else if nsptr.is_null() || (*nsptr).prefix.is_null() {
                nsptr = get_ns_unchecked(root, u, Some(prefix.as_deref().unwrap_or(b"default")));
                if nsptr.is_null() {
                    errorcode = NAMESPACE_ERR;
                }
            }
            (*nodep).ns = nsptr;
        }
    }
    if errorcode != 0 {
        unsafe { sys::xmlFreeProp(nodep.cast()) };
        return throw(cx, errorcode, intern.strict());
    }
    create_object(cx, nodep, Some(&intern))
}

pub fn get_element_by_id(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let idname = bytes(args, 0);
    let (intern, docp) = this_doc(cx, this)?;
    unsafe {
        if !ids_modified_since_parsing(intern.document()) {
            let ci = c(&idname);
            let attrp = sys::xmlGetID(docp, ci.as_ptr());
            if !attrp.is_null() && !(*attrp).parent.is_null() {
                return create_object(cx, (*attrp).parent, Some(&intern));
            }
        } else {
            let base = docp as *const XmlNode;
            let mut node = (*docp).children;
            while !node.is_null() {
                if (*node).type_ == sys::XML_ELEMENT_NODE {
                    let mut a = (*node).properties;
                    while !a.is_null() {
                        if (*a).atype == sys::XML_ATTRIBUTE_ID && attr_value_equals(a, &idname) {
                            return create_object(cx, node, Some(&intern));
                        }
                        a = (*a).next;
                    }
                }
                node = next_in_tree_order(node, base);
            }
        }
    }
    Ok(Value::Null)
}

fn transfer_document_ref_single(node: *mut XmlNode, new_document: *mut lxn::RefObj) {
    if let Some(obj) = unsafe { lxn::object_of_node(node) } {
        unsafe { (*new_document).refcount += 1 };
        lxn::decrement_doc_ref(obj);
        obj.document.set(new_document);
    }
}

fn transfer_document_ref_aux(node: *mut XmlNode, new_document: *mut lxn::RefObj) {
    transfer_document_ref_single(node, new_document);
    unsafe {
        if (*node).type_ == sys::XML_ELEMENT_NODE {
            let mut a = (*node).properties;
            while !a.is_null() {
                transfer_document_ref_single(a.cast(), new_document);
                let mut ch = (*a).children;
                while !ch.is_null() {
                    transfer_document_ref_single(ch, new_document);
                    ch = (*ch).next;
                }
                a = (*a).next;
            }
        }
    }
}

/// php_dom_transfer_document_ref
fn transfer_document_ref(node: *mut XmlNode, new_document: *mut lxn::RefObj) {
    if new_document.is_null() {
        return;
    }
    let base = node;
    transfer_document_ref_aux(base, new_document);
    let mut n = unsafe { (*node).children };
    while !n.is_null() {
        transfer_document_ref_aux(n, new_document);
        n = next_in_tree_order(n, base);
    }
}

/// Before 2.13, xmlDOMWrapAdoptNode leaves names and content in the old
/// document's dictionary (libxml_fixup_name_and_content).
fn fixup_name_and_content(src_doc: *mut XmlDoc, dst_doc: *mut XmlDoc, node: *mut XmlNode) {
    unsafe fn copy(src: *mut sys::xmlDict, dst: *mut sys::xmlDict, s: *const u8) -> *const u8 {
        unsafe {
            if s.is_null() {
                return ptr::null();
            }
            if sys::xmlDictOwns(src, s) == 1 {
                if dst.is_null() {
                    return sys::xmlStrdup(s);
                }
                return sys::xmlDictLookup(dst, s, -1);
            }
            s
        }
    }
    unsafe {
        if (*node).type_ == sys::XML_ENTITY_REF_NODE {
            (*node).children = ptr::null_mut();
        }
        if !src_doc.is_null() && !(*src_doc).dict.is_null() {
            (*node).name = copy((*src_doc).dict, (*dst_doc).dict, (*node).name);
            (*node).content = copy((*src_doc).dict, ptr::null_mut(), (*node).content) as *mut u8;
        }
    }
}

fn fixup_name_and_content_outer(src_doc: *mut XmlDoc, dst_doc: *mut XmlDoc, node: *mut XmlNode) {
    fixup_name_and_content(src_doc, dst_doc, node);
    unsafe {
        if (*node).type_ == sys::XML_ELEMENT_NODE {
            let mut a = (*node).properties;
            while !a.is_null() {
                fixup_name_and_content(src_doc, dst_doc, a.cast());
                let mut ch = (*a).children;
                while !ch.is_null() {
                    fixup_name_and_content(src_doc, dst_doc, ch);
                    ch = (*ch).next;
                }
                a = (*a).next;
            }
        }
        let t = (*node).type_;
        if t == sys::XML_ELEMENT_NODE || t == sys::XML_ATTRIBUTE_NODE {
            let mut ch = (*node).children;
            while !ch.is_null() {
                fixup_name_and_content_outer(src_doc, dst_doc, ch);
                ch = (*ch).next;
            }
        }
    }
}

/// php_dom_adopt_node
pub fn adopt_node_impl(
    nodep: *mut XmlNode,
    new_doc_obj: &DomObject,
    new_document: *mut XmlDoc,
) -> bool {
    unsafe {
        let old_doc = (*nodep).doc;
        lxn::invalidate_node_list_cache_from_doc(old_doc);
        if old_doc != new_document {
            lxn::invalidate_node_list_cache(new_doc_obj.document());
            let ret = sys::xmlDOMWrapAdoptNode(
                ptr::null_mut(),
                old_doc,
                nodep,
                new_document,
                ptr::null_mut(),
                0,
            );
            if ret != 0 {
                return false;
            }
            if crate::ext::libxml::header_version() < 21300 {
                fixup_name_and_content_outer(old_doc, new_document, nodep);
            }
            transfer_document_ref(nodep, new_doc_obj.document());
        } else {
            sys::xmlUnlinkNode(nodep);
        }
    }
    true
}

pub fn adopt_node(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let nodev = arg(args, 0);
    let (nobj, nodep) = get_obj(cx, &nodev)?;
    unsafe {
        let t = (*nodep).type_;
        if matches!(
            t,
            sys::XML_DOCUMENT_NODE
                | sys::XML_HTML_DOCUMENT_NODE
                | sys::XML_DOCUMENT_TYPE_NODE
                | sys::XML_DTD_NODE
                | sys::XML_ENTITY_NODE
                | sys::XML_NOTATION_NODE
        ) {
            return throw(cx, NOT_SUPPORTED_ERR, nobj.strict());
        }
    }
    let (dobj, new_document) = this_doc(cx, this)?;
    if !adopt_node_impl(nodep, &dobj, new_document) {
        return Ok(Value::Bool(false));
    }
    Ok(nodev)
}

pub fn normalize_document(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let (_, docp) = this_doc(cx, this)?;
    normalize_legacy(docp.cast());
    Ok(Value::Null)
}

/// `expand_filepath`: an absolute path, dots resolved, symlinks kept.
fn expand_path(path: &[u8]) -> Vec<u8> {
    use std::os::unix::ffi::OsStringExt;
    let full = if path.starts_with(b"/") {
        path.to_vec()
    } else {
        let mut cwd = std::env::current_dir()
            .map(|d| d.into_os_string().into_vec())
            .unwrap_or_default();
        cwd.push(b'/');
        cwd.extend_from_slice(path);
        cwd
    };
    let mut parts: Vec<&[u8]> = Vec::new();
    for seg in full.split(|&b| b == b'/') {
        match seg {
            b"" | b"." => {}
            b".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let mut out = Vec::new();
    for p in parts {
        out.push(b'/');
        out.extend_from_slice(p);
    }
    if out.is_empty() {
        out.push(b'/');
    }
    out
}

/// dom_get_valid_file_path: a local path (file:// URLs included) made
/// absolute (`realpath`, else `expand_filepath`); other URLs unchanged.
pub fn valid_file_path(source: &[u8]) -> Option<Vec<u8>> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let mut src = source;
    let has_scheme = unsafe {
        let uri = sys::xmlCreateURI();
        if uri.is_null() {
            return None;
        }
        let cs = c(source);
        let esc = sys::xmlURIEscapeStr(cs.as_ptr(), c":".as_ptr().cast());
        sys::xmlParseURIReference(uri, esc.cast());
        sys::xml_free(esc);
        let has = !sys::rhp_xml_uri_scheme(uri).is_null();
        sys::xmlFreeURI(uri);
        has
    };
    let mut is_file_uri = false;
    if has_scheme {
        if source.len() >= 8 && source[..8].eq_ignore_ascii_case(b"file:///") {
            is_file_uri = true;
            src = &source[7..];
        } else if source.len() >= 17 && source[..17].eq_ignore_ascii_case(b"file://localhost/") {
            is_file_uri = true;
            src = &source[16..];
        }
    }
    if !has_scheme || is_file_uri {
        let os = std::ffi::OsStr::from_bytes(src);
        let p = match std::fs::canonicalize(os) {
            Ok(p) => p.into_os_string().into_vec(),
            Err(_) => expand_path(src),
        };
        // expand_filepath fails past MAXPATHLEN.
        if p.len() >= libc::PATH_MAX as usize {
            return None;
        }
        return Some(p);
    }
    Some(src.to_vec())
}

/// `EG(error_reporting) |= E_WARNING` for the duration of a recovering
/// parse.
struct ShowWarnings<'a> {
    old: Option<Vec<u8>>,
    host: *mut (dyn crate::Host + 'a),
}

impl<'a> ShowWarnings<'a> {
    fn new(host: &mut (dyn crate::Host + 'a)) -> Self {
        let old = host.ini_get(b"error_reporting");
        if let Some(o) = &old {
            let v = rhp_values::conv::to_int(&Value::str(o.clone()));
            host.ini_set(
                b"error_reporting",
                (v | crate::E_WARNING).to_string().into_bytes(),
            );
        }
        ShowWarnings { old, host }
    }

    fn restore(self) {
        if let Some(o) = self.old {
            unsafe { (*self.host).ini_set(b"error_reporting", o) };
        }
    }
}

/// Load mode for the document parser.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    String,
    File,
}

/// What dom_document_parser produced.
enum Parsed {
    Doc(*mut XmlDoc),
    Malformed,
    Failed,
}

/// dom_document_parser
fn document_parser(
    cx: &mut Cx,
    props: &lxn::DocProps,
    mode: Mode,
    source: &[u8],
    mut options: i64,
) -> Result<Parsed, Thrown> {
    let recover = props.recover || options & XML_PARSE_RECOVER != 0;
    let ctxt = if mode == Mode::File {
        if source.contains(&0) {
            return Err(value_error(
                cx,
                1,
                "filename",
                "must not contain any null bytes",
            ));
        }
        match valid_file_path(source) {
            Some(dest) => {
                let cd = c(&dest);
                cx.with_libxml_owned(
                    || unsafe { sys::xmlCreateFileParserCtxt(cd.as_ptr().cast()) },
                    |p| unsafe {
                        if !p.is_null() {
                            sys::xmlFreeParserCtxt(p)
                        }
                    },
                )?
            }
            None => ptr::null_mut(),
        }
    } else {
        unsafe { sys::xmlCreateMemoryParserCtxt(source.as_ptr().cast(), source.len() as c_int) }
    };
    if ctxt.is_null() {
        return Ok(Parsed::Failed);
    }
    unsafe {
        if mode != Mode::File {
            if let Ok(cwd) = std::env::current_dir() {
                use std::os::unix::ffi::OsStringExt;
                let mut d = cwd.into_os_string().into_vec();
                if d.last() != Some(&b'/') {
                    d.push(b'/');
                }
                let cd = c(&d);
                sys::rhp_xml_ctxt_set_directory(ctxt, sys::xmlCanonicPath(cd.as_ptr()).cast());
            }
        }
        sys::rhp_xml_ctxt_use_php_errors(ctxt);
    }
    if props.validateonparse && options & XML_PARSE_DTDVALID == 0 {
        options |= XML_PARSE_DTDVALID;
    }
    if props.resolveexternals && options & XML_PARSE_DTDATTR == 0 {
        options |= XML_PARSE_DTDATTR;
    }
    if props.substituteentities && options & XML_PARSE_NOENT == 0 {
        options |= XML_PARSE_NOENT;
    }
    if !props.preservewhitespace && options & XML_PARSE_NOBLANKS == 0 {
        options |= XML_PARSE_NOBLANKS;
    }
    if recover {
        options |= XML_PARSE_RECOVER;
    }
    unsafe {
        sys::rhp_xml_sanitize_parse_ctxt_options(ctxt);
        sys::xmlCtxtUseOptions(ctxt, options as c_int);
    }
    let shown = recover.then(|| ShowWarnings::new(cx.host));
    let parsed = cx.with_libxml(|| unsafe { sys::xmlParseDocument(ctxt) });
    if let Some(s) = shown {
        s.restore();
    }
    let ret = unsafe {
        if sys::rhp_xml_ctxt_well_formed(ctxt) != 0 || recover {
            let doc = sys::rhp_xml_ctxt_take_doc(ctxt);
            let dir = sys::rhp_xml_ctxt_directory(ctxt);
            if !doc.is_null() && (*doc).URL.is_null() && !dir.is_null() {
                (*doc).URL = sys::xmlStrdup(dir.cast());
            }
            Parsed::Doc(doc)
        } else {
            let doc = sys::rhp_xml_ctxt_take_doc(ctxt);
            if !doc.is_null() {
                sys::xmlFreeDoc(doc);
            }
            Parsed::Malformed
        }
    };
    unsafe { sys::xmlFreeParserCtxt(ctxt) };
    if let Err(t) = parsed {
        if let Parsed::Doc(doc) = ret {
            if !doc.is_null() {
                unsafe { sys::xmlFreeDoc(doc) };
            }
        }
        return Err(t);
    }
    Ok(ret)
}

/// php_dom_finish_loading_document
fn finish_loading(this: &Value, newdoc: *mut XmlDoc) -> Result<Value, Thrown> {
    if newdoc.is_null() {
        return Ok(Value::Bool(false));
    }
    let intern = ensure_intern(this, || Kind::Node).expect("a DOMDocument");
    let kept = replace_document(&intern, newdoc, true);
    let doc = intern.document();
    if let Some((props, class_type, old_modnr)) = kept {
        unsafe {
            (*doc).doc_props = props;
            (*doc).class_type = class_type;
            if old_modnr != 0 {
                (*doc).cache_tag = old_modnr;
                lxn::invalidate_node_list_cache(doc);
            }
        }
    } else {
        unsafe { (*doc).class_type = ClassType::Legacy };
    }
    Ok(Value::Bool(true))
}

fn parse_document(cx: &mut Cx, this: &Value, args: &[Value], mode: Mode) -> Result<Value, Thrown> {
    let source = bytes(args, 0);
    let options = int(args, 1);
    if source.is_empty() {
        let p = if mode == Mode::File {
            "filename"
        } else {
            "source"
        };
        return Err(value_error(cx, 1, p, "must not be empty"));
    }
    if source.len() > INT_MAX as usize {
        cx.warn("Input string is too long")?;
        return Ok(Value::Bool(false));
    }
    if options > INT_MAX || options < i32::MIN as i64 {
        cx.warn("Invalid options")?;
        return Ok(Value::Bool(false));
    }
    let props = super::intern_of(this)
        .map(|d| lxn::doc_props_read_only(d.document()))
        .unwrap_or(lxn::DocProps::DEFAULT);
    let newdoc = match document_parser(cx, &props, mode, &source, options)? {
        Parsed::Doc(d) => d,
        Parsed::Malformed | Parsed::Failed => ptr::null_mut(),
    };
    finish_loading(this, newdoc)
}

pub fn load(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    parse_document(cx, this, args, Mode::File)
}

pub fn load_xml(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    parse_document(cx, this, args, Mode::String)
}

pub fn save(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let file = bytes(args, 0);
    let options = int(args, 1);
    if file.contains(&0) {
        return Err(value_error(
            cx,
            1,
            "filename",
            "must not contain any null bytes",
        ));
    }
    if file.is_empty() {
        return Err(value_error(cx, 1, "filename", "must not be empty"));
    }
    let (intern, docp) = this_doc(cx, this)?;
    let format = lxn::doc_props_read_only(intern.document()).formatoutput;
    let cf = c(&file);
    let bytes = cx.with_libxml(|| unsafe {
        let saved =
            (options & LIBXML_SAVE_NOEMPTYTAG != 0).then(|| sys::rhp_xml_set_save_no_empty_tags(1));
        let n = sys::xmlSaveFormatFileEnc(
            cf.as_ptr().cast(),
            docp,
            (*docp).encoding.cast(),
            format as c_int,
        );
        if let Some(old) = saved {
            sys::rhp_xml_set_save_no_empty_tags(old);
        }
        n
    })?;
    if bytes == -1 {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Int(bytes as i64))
}

pub fn save_xml(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let nodev = arg(args, 0);
    let options = int(args, 1);
    let (intern, docp) = this_doc(cx, this)?;
    let format = lxn::doc_props_read_only(intern.document()).formatoutput;
    let encoding = unsafe { (*docp).encoding };
    let no_empty = (options & LIBXML_SAVE_NOEMPTYTAG != 0) as c_int;
    let res = if !matches!(nodev, Value::Null) {
        let (_, node) = get_obj(cx, &nodev)?;
        if unsafe { (*node).doc } != docp {
            return throw(cx, WRONG_DOCUMENT_ERR, intern.strict());
        }
        cx.with_libxml(|| unsafe {
            let old = sys::rhp_xml_set_save_no_empty_tags(no_empty);
            let r = lxn::dump_node_to_str(docp, node, format, encoding);
            sys::rhp_xml_set_save_no_empty_tags(old);
            r
        })?
    } else {
        let mut opts = XML_SAVE_AS_XML;
        if options & XML_SAVE_NO_DECL != 0 {
            opts |= XML_SAVE_NO_DECL as c_int;
        }
        if format {
            opts |= XML_SAVE_FORMAT;
        }
        cx.with_libxml(|| unsafe {
            let old = sys::rhp_xml_set_save_no_empty_tags(no_empty);
            let r = lxn::dump_doc_to_str(docp, opts, encoding);
            sys::rhp_xml_set_save_no_empty_tags(old);
            r
        })?
    };
    match res {
        Some(s) => Ok(Value::str(s)),
        None => {
            cx.warn("Could not save document")?;
            Ok(Value::Bool(false))
        }
    }
}

fn xinclude_strip_attrs(basep: *mut XmlNode) {
    unsafe {
        let mut prop = (*basep).properties;
        while !prop.is_null() {
            lxn::node_free_resource(prop.cast());
            let mut ch = (*prop).children;
            while !ch.is_null() {
                lxn::node_free_resource(ch);
                ch = (*ch).next;
            }
            prop = (*prop).next;
        }
    }
}

/// dom_xinclude_strip_references: libxml2 replaces xi:include elements by
/// copies, so their objects must let go first.
fn xinclude_strip_references(basep: *mut XmlNode) {
    unsafe {
        lxn::node_free_resource(basep);
        xinclude_strip_attrs(basep);
        let mut cur = (*basep).children;
        while !cur.is_null() {
            lxn::node_free_resource(cur);
            if (*cur).type_ == sys::XML_ELEMENT_NODE {
                xinclude_strip_attrs(cur);
            }
            cur = next_in_tree_order(cur, basep);
        }
    }
}

const XINCLUDE_NS: &[u8] = b"http://www.w3.org/2003/XInclude";
const XINCLUDE_OLD_NS: &[u8] = b"http://www.w3.org/2001/XInclude";

fn xinclude_strip_fallback_references(basep: *const XmlNode) {
    unsafe {
        let mut cur = (*basep).children;
        while !cur.is_null() {
            if (*cur).type_ == sys::XML_ELEMENT_NODE
                && !(*cur).ns.is_null()
                && b((*cur).name) == b"include"
                && matches!(b((*(*cur).ns).href), h if h == XINCLUDE_NS || h == XINCLUDE_OLD_NS)
            {
                xinclude_strip_references(cur);
            }
            cur = next_in_tree_order(cur, basep);
        }
    }
}

pub fn xinclude(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let mut flags = int(args, 0);
    if !(i32::MIN as i64..=INT_MAX).contains(&flags) {
        cx.warn("Invalid flags")?;
        return Ok(Value::Bool(false));
    }
    let (intern, docp) = this_doc(cx, this)?;
    xinclude_strip_fallback_references(docp.cast());
    flags |= XML_PARSE_NOXINCNODE;
    let err = cx.with_libxml(|| unsafe {
        let mut saved = [0i32; 6];
        sys::rhp_xml_sanitize_globals(saved.as_mut_ptr());
        let e = sys::xmlXIncludeProcessFlags(docp, flags as c_int);
        sys::rhp_xml_restore_globals(saved.as_ptr());
        e
    })?;
    lxn::invalidate_node_list_cache(intern.document());
    Ok(if err != 0 {
        Value::Int(err as i64)
    } else {
        Value::Bool(false)
    })
}

pub fn validate(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let (_, docp) = this_doc(cx, this)?;
    let valid = cx.with_libxml(|| unsafe {
        let mut saved = [0i32; 6];
        sys::rhp_xml_sanitize_globals(saved.as_mut_ptr());
        let cvp = sys::xmlNewValidCtxt();
        sys::rhp_xml_valid_ctxt_use_php_errors(cvp);
        let v = sys::xmlValidateDocument(cvp, docp);
        sys::rhp_xml_restore_globals(saved.as_ptr());
        sys::xmlFreeValidCtxt(cvp);
        v
    })?;
    Ok(Value::Bool(valid != 0))
}

fn schema_validate_common(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
    mode: Mode,
) -> Result<Value, Thrown> {
    let source = bytes(args, 0);
    let flags = int(args, 1);
    let pname = if mode == Mode::File {
        "filename"
    } else {
        "source"
    };
    if source.is_empty() {
        return Err(value_error(cx, 1, pname, "must not be empty"));
    }
    let (intern, _) = this_doc(cx, this)?;
    let mut saved = [0i32; 6];
    unsafe { sys::rhp_xml_sanitize_globals(saved.as_mut_ptr()) };
    let parser = match mode {
        Mode::File => {
            if source.contains(&0) {
                unsafe { sys::rhp_xml_restore_globals(saved.as_ptr()) };
                return Err(value_error(cx, 1, pname, "must not contain any null bytes"));
            }
            let Some(f) = valid_file_path(&source) else {
                unsafe { sys::rhp_xml_restore_globals(saved.as_ptr()) };
                cx.warn("Invalid Schema file source")?;
                return Ok(Value::Bool(false));
            };
            let cf = c(&f);
            unsafe { sys::xmlSchemaNewParserCtxt(cf.as_ptr().cast()) }
        }
        Mode::String => unsafe {
            sys::xmlSchemaNewMemParserCtxt(source.as_ptr().cast(), source.len() as c_int)
        },
    };
    let sptr = cx.with_libxml_owned(
        || unsafe {
            sys::rhp_xml_schema_parser_use_php_errors(parser);
            let s = sys::xmlSchemaParse(parser);
            sys::xmlSchemaFreeParserCtxt(parser);
            s
        },
        |s| unsafe {
            if !s.is_null() {
                sys::xmlSchemaFree(s)
            }
        },
    );
    unsafe { sys::rhp_xml_restore_globals(saved.as_ptr()) };
    let sptr = sptr?;
    if sptr.is_null() {
        cx.warn("Invalid Schema")?;
        return Ok(Value::Bool(false));
    }
    let docp: *mut XmlDoc = intern.node().cast();
    let vptr = unsafe { sys::xmlSchemaNewValidCtxt(sptr) };
    if vptr.is_null() {
        unsafe { sys::xmlSchemaFree(sptr) };
        return Err(Thrown::error("Error", "Invalid Schema Validation Context"));
    }
    let valid_opts = (flags & XML_SCHEMA_VAL_VC_I_CREATE) as c_int;
    let is_valid = cx.with_libxml(|| unsafe {
        let mut saved = [0i32; 6];
        sys::rhp_xml_sanitize_globals(saved.as_mut_ptr());
        sys::xmlSchemaSetValidOptions(vptr, valid_opts);
        sys::rhp_xml_schema_valid_use_php_errors(vptr);
        let v = sys::xmlSchemaValidateDoc(vptr, docp);
        sys::xmlSchemaFree(sptr);
        sys::xmlSchemaFreeValidCtxt(vptr);
        sys::rhp_xml_restore_globals(saved.as_ptr());
        v
    })?;
    Ok(Value::Bool(is_valid == 0))
}

pub fn schema_validate(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    schema_validate_common(cx, this, args, Mode::File)
}

pub fn schema_validate_source(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    schema_validate_common(cx, this, args, Mode::String)
}

fn relaxng_validate_common(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
    mode: Mode,
) -> Result<Value, Thrown> {
    let source = bytes(args, 0);
    let pname = if mode == Mode::File {
        "filename"
    } else {
        "source"
    };
    if source.is_empty() {
        return Err(value_error(cx, 1, pname, "must not be empty"));
    }
    let (intern, _) = this_doc(cx, this)?;
    let parser = match mode {
        Mode::File => {
            if source.contains(&0) {
                return Err(value_error(cx, 1, pname, "must not contain any null bytes"));
            }
            let Some(f) = valid_file_path(&source) else {
                cx.warn("Invalid RelaxNG file source")?;
                return Ok(Value::Bool(false));
            };
            let cf = c(&f);
            unsafe { sys::xmlRelaxNGNewParserCtxt(cf.as_ptr().cast()) }
        }
        Mode::String => unsafe {
            sys::xmlRelaxNGNewMemParserCtxt(source.as_ptr().cast(), source.len() as c_int)
        },
    };
    let sptr = cx.with_libxml_owned(
        || unsafe {
            let mut saved = [0i32; 6];
            sys::rhp_xml_sanitize_globals(saved.as_mut_ptr());
            sys::rhp_xml_relaxng_parser_use_php_errors(parser);
            let s = sys::xmlRelaxNGParse(parser);
            sys::xmlRelaxNGFreeParserCtxt(parser);
            sys::rhp_xml_restore_globals(saved.as_ptr());
            s
        },
        |s| unsafe {
            if !s.is_null() {
                sys::xmlRelaxNGFree(s)
            }
        },
    )?;
    if sptr.is_null() {
        cx.warn("Invalid RelaxNG")?;
        return Ok(Value::Bool(false));
    }
    let docp: *mut XmlDoc = intern.node().cast();
    let vptr = unsafe { sys::xmlRelaxNGNewValidCtxt(sptr) };
    if vptr.is_null() {
        unsafe { sys::xmlRelaxNGFree(sptr) };
        return Err(Thrown::error("Error", "Invalid RelaxNG Validation Context"));
    }
    let is_valid = cx.with_libxml(|| unsafe {
        sys::rhp_xml_relaxng_valid_use_php_errors(vptr);
        let v = sys::xmlRelaxNGValidateDoc(vptr, docp);
        sys::xmlRelaxNGFree(sptr);
        sys::xmlRelaxNGFreeValidCtxt(vptr);
        v
    })?;
    Ok(Value::Bool(is_valid == 0))
}

pub fn relaxng_validate(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    relaxng_validate_common(cx, this, args, Mode::File)
}

pub fn relaxng_validate_source(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    relaxng_validate_common(cx, this, args, Mode::String)
}

fn load_html_common(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
    mode: Mode,
) -> Result<Value, Thrown> {
    let source = bytes(args, 0);
    let options = int(args, 1);
    let pname = if mode == Mode::File {
        "filename"
    } else {
        "source"
    };
    if source.is_empty() {
        return Err(value_error(cx, 1, pname, "must not be empty"));
    }
    if !(i32::MIN as i64..=INT_MAX).contains(&options) {
        cx.warn("Invalid options")?;
        return Ok(Value::Bool(false));
    }
    let ctxt = if mode == Mode::File {
        if source.contains(&0) {
            return Err(value_error(cx, 1, pname, "must not contain any null bytes"));
        }
        let cs = c(&source);
        cx.with_libxml_owned(
            || unsafe { sys::htmlCreateFileParserCtxt(cs.as_ptr().cast(), ptr::null()) },
            free_html_ctxt,
        )?
    } else {
        if source.len() > INT_MAX as usize {
            cx.warn("Input string is too long")?;
            return Ok(Value::Bool(false));
        }
        cx.with_libxml_owned(
            || unsafe {
                sys::htmlCreateMemoryParserCtxt(source.as_ptr().cast(), source.len() as c_int)
            },
            free_html_ctxt,
        )?
    };
    if ctxt.is_null() {
        return Ok(Value::Bool(false));
    }
    let newdoc = cx.with_libxml_owned(
        || unsafe {
            sys::rhp_xml_ctxt_use_php_errors(ctxt);
            sys::rhp_xml_sanitize_parse_ctxt_options(ctxt);
            if options != 0 {
                sys::htmlCtxtUseOptions(ctxt, options as c_int);
            }
            sys::htmlParseDocument(ctxt);
            let d = sys::rhp_xml_ctxt_take_doc(ctxt);
            sys::htmlFreeParserCtxt(ctxt);
            d
        },
        |d| unsafe {
            if !d.is_null() {
                sys::xmlFreeDoc(d)
            }
        },
    )?;
    finish_loading(this, newdoc)
}

pub fn load_html(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    load_html_common(cx, this, args, Mode::String)
}

pub fn load_html_file(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    load_html_common(cx, this, args, Mode::File)
}

pub fn save_html_file(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let file = bytes(args, 0);
    if file.contains(&0) {
        return Err(value_error(
            cx,
            1,
            "filename",
            "must not contain any null bytes",
        ));
    }
    if file.is_empty() {
        return Err(value_error(cx, 1, "filename", "must not be empty"));
    }
    let (intern, docp) = this_doc(cx, this)?;
    let format = lxn::doc_props_read_only(intern.document()).formatoutput;
    let cf = c(&file);
    let bytes = cx.with_libxml(|| unsafe {
        let encoding = sys::htmlGetMetaEncoding(docp);
        sys::htmlSaveFileFormat(cf.as_ptr().cast(), docp, encoding.cast(), format as c_int)
    })?;
    if bytes == -1 {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Int(bytes as i64))
}

pub fn save_html(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let nodev = arg(args, 0);
    let (intern, docp) = this_doc(cx, this)?;
    let format = unsafe { lxn::doc_props_mut(intern.document()) }
        .map(|p| p.formatoutput)
        .unwrap_or(false) as c_int;
    if !matches!(nodev, Value::Null) {
        let (_, node) = get_obj(cx, &nodev)?;
        if unsafe { (*node).doc } != docp {
            return throw(cx, WRONG_DOCUMENT_ERR, intern.strict());
        }
        enum Out {
            Str(Vec<u8>),
            False,
            Warn(&'static str),
        }
        let out = cx.with_libxml(|| unsafe {
            let buf = sys::xmlBufferCreate();
            if buf.is_null() {
                return Out::Warn("Could not fetch buffer");
            }
            let out_buf = sys::xmlOutputBufferCreateBuffer(buf, ptr::null_mut());
            if out_buf.is_null() {
                sys::xmlBufferFree(buf);
                return Out::Warn("Could not fetch output buffer");
            }
            if (*node).type_ == sys::XML_DOCUMENT_FRAG_NODE {
                let mut ch = (*node).children;
                while !ch.is_null() {
                    sys::htmlNodeDumpFormatOutput(out_buf, docp, ch, ptr::null(), format);
                    if sys::rhp_xml_output_buffer_error(out_buf) != 0 {
                        break;
                    }
                    ch = (*ch).next;
                }
            } else {
                sys::htmlNodeDumpFormatOutput(out_buf, docp, node, ptr::null(), format);
            }
            let r = if sys::rhp_xml_output_buffer_error(out_buf) == 0 {
                sys::xmlOutputBufferFlush(out_buf);
                let mem = sys::xmlBufferContent(buf);
                if mem.is_null() {
                    Out::False
                } else {
                    let size = sys::xmlBufferLength(buf) as usize;
                    Out::Str(std::slice::from_raw_parts(mem, size).to_vec())
                }
            } else {
                Out::Warn("Error dumping HTML node")
            };
            sys::xmlOutputBufferClose(out_buf);
            sys::xmlBufferFree(buf);
            r
        })?;
        return match out {
            Out::Str(s) => Ok(Value::str(s)),
            Out::False => Ok(Value::Bool(false)),
            Out::Warn(m) => {
                cx.warn(m)?;
                Ok(Value::Bool(false))
            }
        };
    }
    let s = cx.with_libxml(|| unsafe {
        let mut mem: *mut u8 = ptr::null_mut();
        let mut size: c_int = 0;
        sys::htmlDocDumpMemoryFormat(docp, &mut mem, &mut size, format);
        let r = if size == 0 || mem.is_null() {
            None
        } else {
            Some(std::slice::from_raw_parts(mem, size as usize).to_vec())
        };
        sys::xml_free(mem);
        r
    })?;
    Ok(s.map(Value::str).unwrap_or(Value::Bool(false)))
}

/// A class name argument (zend_parse_arg_class): the class's real name.
fn class_arg(
    cx: &mut Cx,
    v: &Value,
    n: usize,
    pname: &str,
    base: Option<&str>,
) -> Result<Option<(Vec<u8>, bool)>, Thrown> {
    if matches!(v, Value::Null) && base.is_none() {
        return Ok(None);
    }
    let name = crate::ext::libxml::io::to_string(cx.host, v)?;
    let info = cx
        .host
        .reflect_op(b"class", &[Value::str(name.clone()), Value::Bool(true)])?;
    let found = match &info {
        Value::Array(a) => {
            let real = a
                .get(&ArrayKey::from_bytes(b"name"))
                .map(rhp_values::conv::to_string_bytes);
            let abs = a
                .get(&ArrayKey::from_bytes(b"abstract"))
                .is_some_and(rhp_values::conv::to_bool);
            real.map(|r| (r, abs))
        }
        _ => None,
    };
    let given = String::from_utf8_lossy(&name).into_owned();
    if let Some(b) = base {
        let ok = found
            .as_ref()
            .is_some_and(|(r, _)| cx.host.reflect_is_a(r, b.as_bytes()));
        if !ok {
            return Err(Thrown::type_error(format!(
                "{}(): Argument #{n} (${pname}) must be a class name derived from {b}, {given} given",
                cx.fname
            )));
        }
    }
    match found {
        Some(f) => Ok(Some(f)),
        None => Err(Thrown::type_error(format!(
            "{}(): Argument #{n} (${pname}) must be a valid class name, {given} given",
            cx.fname
        ))),
    }
}

pub fn register_node_class(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let Some((basece, base_abstract)) =
        class_arg(cx, &arg(args, 0), 1, "baseClass", Some("DOMNode"))?
    else {
        return Ok(Value::Null);
    };
    let ce = class_arg(cx, &arg(args, 1), 2, "extendedClass", None)?;
    if base_abstract {
        return Err(value_error(
            cx,
            1,
            "baseClass",
            "must not be an abstract class",
        ));
    }
    let derived = ce
        .as_ref()
        .is_none_or(|(c, _)| cx.host.reflect_is_a(c, &basece));
    if derived {
        if let Some((_, true)) = &ce {
            return Err(value_error(
                cx,
                2,
                "extendedClass",
                "must not be an abstract class",
            ));
        }
        let intern = get_intern(cx, this)?;
        set_doc_classmap(intern.document(), &basece, ce.map(|(c, _)| c));
        return Ok(Value::Bool(true));
    }
    Err(Thrown::error(
        "Error",
        format!(
            "{}(): Argument #2 ($extendedClass) must be a class name derived from {} or null, {} given",
            cx.fname,
            String::from_utf8_lossy(&basece),
            String::from_utf8_lossy(&ce.map(|(c, _)| c).unwrap_or_default())
        ),
    ))
}

/// dom_set_doc_classmap
fn set_doc_classmap(document: *mut lxn::RefObj, basece: &[u8], ce: Option<Vec<u8>>) {
    let Some(props) = (unsafe { lxn::doc_props_mut(document) }) else {
        return;
    };
    let lc = basece.to_ascii_lowercase();
    props.classmap.retain(|(b, _)| b != &lc);
    if let Some(c) = ce {
        props.classmap.push((lc, c));
    }
}

/// Frees an unlinked node a callback's exception orphaned.
fn free_node(n: *mut XmlNode) {
    if !n.is_null() {
        unsafe { sys::xmlFreeNode(n) };
    }
}

fn free_html_ctxt(p: sys::Ctxt) {
    if !p.is_null() {
        unsafe { sys::htmlFreeParserCtxt(p) };
    }
}
