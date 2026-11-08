//! DOMCharacterData, DOMText, DOMCdataSection, DOMComment, DOMAttr,
//! DOMEntityReference, DOMProcessingInstruction, DOMDocumentFragment and
//! DOMImplementation (php-src ext/dom/characterdata.c, text.c,
//! cdatasection.c, comment.c, attr.c, entityreference.c,
//! processinginstruction.c, documentfragment.c, domimplementation.c).

use std::ffi::c_int;
use std::ptr;

use rhp_values::Value;

use super::element::set_constructed_node;
use super::util::*;
use super::{
    arg, bytes, create_object, get_obj, int, intern_of, throw, Cx, DomObject, INDEX_SIZE_ERR,
    INVALID_CHARACTER_ERR, INVALID_STATE_ERR, NAMESPACE_ERR, NO_MODIFICATION_ALLOWED_ERR,
    WRONG_DOCUMENT_ERR,
};
use crate::ext::libxml::node as lxn;
use crate::ext::libxml::sys::{self, XmlNode};
use crate::Thrown;

const INT_MAX: i64 = i32::MAX as i64;

fn content_or_empty(n: *const XmlNode) -> *const u8 {
    unsafe {
        if (*n).content.is_null() {
            c"".as_ptr().cast()
        } else {
            (*n).content
        }
    }
}

fn utf8_len(s: *const u8) -> i64 {
    unsafe { sys::xmlUTF8Strlen(s) as i64 }
}

/// dom_convert_number_unsigned: a negative number is an error for the
/// legacy API and wraps (as a C unsigned int) for the modern one.
fn unsigned(cx: &mut Cx, intern: &DomObject, input: i64) -> Result<Option<u32>, Thrown> {
    let strict = intern.strict();
    if input < 0 {
        if super::modern::is_modern(intern) {
            return Ok(Some(input as u32));
        }
        throw(cx, INDEX_SIZE_ERR, strict)?;
        return Ok(None);
    }
    Ok(Some(input as u32))
}

fn substr(s: *const u8, start: i64, len: i64) -> *mut u8 {
    unsafe { sys::xmlUTF8Strsub(s, start as c_int, len as c_int) }
}

pub fn substring_data(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, node) = get_obj(cx, this)?;
    let (offset_in, count_in) = (int(args, 0), int(args, 1));
    let cur = content_or_empty(node);
    let length = utf8_len(cur);
    let strict = intern.strict();
    if offset_in > INT_MAX || count_in > INT_MAX {
        throw(cx, INDEX_SIZE_ERR, strict)?;
        return Ok(Value::Bool(false));
    }
    let Some(offset) = unsigned(cx, &intern, offset_in)? else {
        return Ok(Value::Bool(false));
    };
    let Some(mut count) = unsigned(cx, &intern, count_in)? else {
        return Ok(Value::Bool(false));
    };
    if offset as i64 > length {
        throw(cx, INDEX_SIZE_ERR, strict)?;
        return Ok(Value::Bool(false));
    }
    if count as i64 > length - offset as i64 {
        count = (length - offset as i64) as u32;
    }
    let s = substr(cur, offset as i64, count as i64);
    Ok(Value::str(
        unsafe { sys::take_string(s) }.unwrap_or_default(),
    ))
}

pub fn append_data(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (_, node) = get_obj(cx, this)?;
    let data = bytes(args, 0);
    unsafe { sys::xmlTextConcat(node, nul(&data).as_ptr(), data.len() as c_int) };
    Ok(Value::Bool(true))
}

pub fn insert_data(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, node) = get_obj(cx, this)?;
    let offset_in = int(args, 0);
    let data = bytes(args, 1);
    let cur = content_or_empty(node);
    let length = utf8_len(cur);
    let strict = intern.strict();
    if offset_in > INT_MAX {
        throw(cx, INDEX_SIZE_ERR, strict)?;
        return Ok(Value::Bool(false));
    }
    let Some(offset) = unsigned(cx, &intern, offset_in)? else {
        return Ok(Value::Bool(false));
    };
    if offset as i64 > length {
        throw(cx, INDEX_SIZE_ERR, strict)?;
        return Ok(Value::Bool(false));
    }
    let cd = c(&data);
    unsafe {
        let first = sys::xmlUTF8Strndup(cur, offset as c_int);
        let second = substr(cur, offset as i64, length - offset as i64);
        sys::xmlNodeSetContent(node, first);
        sys::xmlNodeAddContent(node, cd.as_ptr());
        sys::xmlNodeAddContent(node, second);
        sys::xml_free(first);
        sys::xml_free(second);
    }
    Ok(Value::Bool(true))
}

pub fn delete_data(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, node) = get_obj(cx, this)?;
    let (offset, count_in) = (int(args, 0), int(args, 1));
    let cur = content_or_empty(node);
    let length = utf8_len(cur);
    let strict = intern.strict();
    if !(0..=INT_MAX).contains(&offset) || count_in > INT_MAX || offset > length {
        throw(cx, INDEX_SIZE_ERR, strict)?;
        return Ok(Value::Bool(false));
    }
    let Some(mut count) = unsigned(cx, &intern, count_in)? else {
        return Ok(Value::Bool(false));
    };
    unsafe {
        let mut substring = if offset > 0 {
            substr(cur, 0, offset)
        } else {
            ptr::null_mut()
        };
        if count as i64 > length - offset {
            count = (length - offset) as u32;
        }
        let second = substr(cur, offset + count as i64, length - offset);
        substring = sys::xmlStrcat(substring, second);
        sys::xmlNodeSetContent(node, substring);
        sys::xml_free(second);
        sys::xml_free(substring);
    }
    Ok(Value::Bool(true))
}

pub fn replace_data(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, node) = get_obj(cx, this)?;
    let (offset, count_in) = (int(args, 0), int(args, 1));
    let data = bytes(args, 2);
    let cur = content_or_empty(node);
    let length = utf8_len(cur);
    let strict = intern.strict();
    if !(0..=INT_MAX).contains(&offset) || count_in > INT_MAX || offset > length {
        throw(cx, INDEX_SIZE_ERR, strict)?;
        return Ok(Value::Bool(false));
    }
    let Some(mut count) = unsigned(cx, &intern, count_in)? else {
        return Ok(Value::Bool(false));
    };
    let cd = c(&data);
    unsafe {
        let mut substring = if offset > 0 {
            substr(cur, 0, offset)
        } else {
            ptr::null_mut()
        };
        if count as i64 > length - offset {
            count = (length - offset) as u32;
        }
        let mut second = ptr::null_mut();
        if offset < length {
            second = substr(cur, offset + count as i64, length - offset);
        }
        substring = sys::xmlStrcat(substring, cd.as_ptr());
        substring = sys::xmlStrcat(substring, second);
        sys::xmlNodeSetContent(node, substring);
        sys::xml_free(second);
        sys::xml_free(substring);
    }
    Ok(Value::Bool(true))
}

/// An optional string argument php leaves NULL when it is not passed.
fn opt_c(args: &[Value], i: usize) -> Option<Vec<u8>> {
    super::opt_bytes(args, i).map(|v| c(&v))
}

fn ptr_of(v: &Option<Vec<u8>>) -> *const u8 {
    v.as_ref().map_or(ptr::null(), |v| v.as_ptr())
}

pub fn text_construct(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let data = opt_c(args, 0);
    let nodep = unsafe { sys::xmlNewText(ptr_of(&data)) };
    if nodep.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    set_constructed_node(this, nodep);
    Ok(Value::Null)
}

pub fn text_split(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, node) = get_obj(cx, this)?;
    let offset = int(args, 0);
    if offset < 0 {
        return Err(crate::zend::zpp::value_error(
            &cx.fname,
            1,
            "offset",
            "must be greater than or equal to 0",
        ));
    }
    let cur = content_or_empty(node);
    let length = utf8_len(cur);
    if offset > INT_MAX || offset > length {
        return Ok(Value::Bool(false));
    }
    let nnode;
    unsafe {
        let first = sys::xmlUTF8Strndup(cur, offset as c_int);
        let second = substr(cur, offset, length - offset);
        sys::xmlNodeSetContent(node, first);
        nnode = sys::xmlNewDocText((*node).doc, second);
        sys::xml_free(first);
        sys::xml_free(second);
        if nnode.is_null() {
            throw(cx, INVALID_STATE_ERR, true)?;
        }
        if !(*node).parent.is_null() {
            (*nnode).type_ = sys::XML_ELEMENT_NODE;
            sys::xmlAddNextSibling(node, nnode);
            (*nnode).type_ = sys::XML_TEXT_NODE;
        }
    }
    create_object(cx, nnode, Some(&intern))
}

pub fn text_is_whitespace(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let (_, node) = get_obj(cx, this)?;
    Ok(Value::Bool(unsafe { sys::xmlIsBlankNode(node) } != 0))
}

pub fn cdata_construct(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let data = bytes(args, 0);
    let nodep =
        unsafe { sys::xmlNewCDataBlock(ptr::null_mut(), nul(&data).as_ptr(), data.len() as c_int) };
    if nodep.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    set_constructed_node(this, nodep);
    Ok(Value::Null)
}

pub fn comment_construct(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let data = opt_c(args, 0);
    let nodep = unsafe { sys::xmlNewComment(ptr_of(&data)) };
    if nodep.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    set_constructed_node(this, nodep);
    Ok(Value::Null)
}

pub fn attr_construct(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let value = opt_c(args, 1);
    if !valid_name(&name) {
        throw(cx, INVALID_CHARACTER_ERR, true)?;
    }
    let cn = c(&name);
    let nodep = unsafe { sys::xmlNewProp(ptr::null_mut(), cn.as_ptr(), ptr_of(&value)) };
    if nodep.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    set_constructed_node(this, nodep.cast());
    Ok(Value::Null)
}

pub fn attr_is_id(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let (_, node) = get_obj(cx, this)?;
    Ok(Value::Bool(
        unsafe { (*(node as *mut sys::XmlAttr)).atype } == sys::XML_ATTRIBUTE_ID,
    ))
}

pub fn entityref_construct(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    if !valid_name(&name) {
        throw(cx, INVALID_CHARACTER_ERR, true)?;
    }
    let cn = c(&name);
    let node = unsafe { sys::xmlNewReference(ptr::null(), cn.as_ptr()) };
    if node.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    set_constructed_node(this, node);
    Ok(Value::Null)
}

pub fn pi_construct(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let value = opt_c(args, 1);
    if !valid_name(&name) {
        throw(cx, INVALID_CHARACTER_ERR, true)?;
    }
    let cn = c(&name);
    let nodep = unsafe { sys::xmlNewPI(cn.as_ptr(), ptr_of(&value)) };
    if nodep.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    set_constructed_node(this, nodep);
    Ok(Value::Null)
}

pub fn fragment_construct(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let nodep = unsafe { sys::xmlNewDocFragment(ptr::null_mut()) };
    if nodep.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    set_constructed_node(this, nodep);
    Ok(Value::Null)
}

pub fn fragment_append_xml(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let data = bytes(args, 0);
    if node_is_read_only(nodep) {
        throw(cx, NO_MODIFICATION_ALLOWED_ERR, intern.strict())?;
        return Ok(Value::Bool(false));
    }
    let cd = c(&data);
    let mut lst: *mut XmlNode = ptr::null_mut();
    let err = cx.with_libxml(|| unsafe {
        let mut saved = [0i32; 6];
        sys::rhp_xml_sanitize_globals(saved.as_mut_ptr());
        let e = sys::xmlParseBalancedChunkMemory(
            (*nodep).doc,
            ptr::null_mut(),
            ptr::null_mut(),
            0,
            cd.as_ptr(),
            &mut lst,
        );
        sys::rhp_xml_restore_globals(saved.as_ptr());
        e
    });
    let err = match err {
        Ok(e) => e,
        Err(t) => {
            if !lst.is_null() {
                unsafe { sys::xmlFreeNodeList(lst) };
            }
            return Err(t);
        }
    };
    if err != 0 {
        return Ok(Value::Bool(false));
    }
    unsafe { sys::xmlAddChildList(nodep, lst) };
    Ok(Value::Bool(true))
}

pub fn impl_has_feature(_: &mut Cx, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(has_feature(&bytes(args, 0), &bytes(args, 1))))
}

pub fn impl_create_document_type(cx: &mut Cx, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let publicid = bytes(args, 1);
    let systemid = bytes(args, 2);
    if name.is_empty() {
        return Err(crate::zend::zpp::value_error(
            &cx.fname,
            1,
            "qualifiedName",
            "must not be empty",
        ));
    }
    if name.windows(3).any(|w| w == b"%00") {
        cx.warn("URI must not contain percent-encoded NUL bytes")?;
        return Ok(Value::Bool(false));
    }
    let cn = c(&name);
    let mut localname = name.clone();
    unsafe {
        let uri = sys::xmlParseURI(cn.as_ptr().cast());
        if !uri.is_null() {
            let opaque = sys::rhp_xml_uri_opaque(uri);
            if !opaque.is_null() {
                localname = sys::cstr(opaque).to_vec();
                if localname.contains(&b':') {
                    sys::xmlFreeURI(uri);
                    throw(cx, NAMESPACE_ERR, true)?;
                    return Ok(Value::Bool(false));
                }
            }
            sys::xmlFreeURI(uri);
        }
    }
    let cl = c(&localname);
    let cp = c(&publicid);
    let cs = c(&systemid);
    let doctype = unsafe {
        sys::xmlCreateIntSubset(
            ptr::null_mut(),
            cl.as_ptr(),
            if publicid.is_empty() {
                ptr::null()
            } else {
                cp.as_ptr()
            },
            if systemid.is_empty() {
                ptr::null()
            } else {
                cs.as_ptr()
            },
        )
    };
    if doctype.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    create_object(cx, doctype.cast(), None)
}

pub fn impl_create_document(cx: &mut Cx, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let uri = super::opt_bytes(args, 0).unwrap_or_default();
    let name = bytes(args, 1);
    let dtv = arg(args, 2);
    let mut doctype: *mut sys::XmlDtd = ptr::null_mut();
    let mut doctobj = None;
    if !matches!(dtv, Value::Null) {
        let (d, n) = get_obj(cx, &dtv)?;
        doctype = n.cast();
        unsafe {
            if (*doctype).type_ == sys::XML_DOCUMENT_TYPE_NODE {
                return Err(crate::zend::zpp::value_error(
                    &cx.fname,
                    3,
                    "doctype",
                    "is an invalid DocumentType object",
                ));
            }
            if !(*doctype).doc.is_null() {
                throw(cx, WRONG_DOCUMENT_ERR, true)?;
            }
        }
        doctobj = Some(d);
    }
    let mut localname = None;
    let mut nsptr: *mut sys::XmlNs = ptr::null_mut();
    if !name.is_empty() {
        match check_qname(&name, 1) {
            Ok((l, prefix)) => {
                if !uri.is_empty() {
                    let cu = c(&uri);
                    let cp = prefix.as_deref().map(c);
                    nsptr = unsafe {
                        sys::xmlNewNs(
                            ptr::null_mut(),
                            cu.as_ptr(),
                            cp.as_ref().map_or(ptr::null(), |p| p.as_ptr()),
                        )
                    };
                    if nsptr.is_null() {
                        throw(cx, NAMESPACE_ERR, true)?;
                    }
                }
                localname = Some(l);
            }
            Err(code) => {
                throw(cx, code, true)?;
            }
        }
    }
    let docp = unsafe { sys::xmlNewDoc(ptr::null()) };
    if docp.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    unsafe {
        if !doctype.is_null() {
            (*docp).intSubset = doctype;
            (*doctype).parent = docp;
            (*doctype).doc = docp;
            (*docp).children = doctype.cast();
            (*docp).last = doctype.cast();
        }
        if let Some(l) = &localname {
            let cl = c(l);
            let nodep = sys::xmlNewDocNode(docp, nsptr, cl.as_ptr(), ptr::null());
            if nodep.is_null() {
                if !doctype.is_null() {
                    (*docp).intSubset = ptr::null_mut();
                    (*doctype).parent = ptr::null_mut();
                    (*doctype).doc = ptr::null_mut();
                    (*docp).children = ptr::null_mut();
                    (*docp).last = ptr::null_mut();
                }
                sys::xmlFreeDoc(docp);
                throw(cx, INVALID_STATE_ERR, true)?;
            }
            (*nodep).nsDef = nsptr;
            sys::xmlDocSetRootElement(docp, nodep);
        }
    }
    let v = create_object(cx, docp.cast(), None)?;
    if let Some(dt) = doctobj {
        if let Some(docobj) = intern_of(&v) {
            dt.base.document.set(docobj.document());
            lxn::increment_doc_ref(&dt.base, docp);
        }
    }
    Ok(v)
}
