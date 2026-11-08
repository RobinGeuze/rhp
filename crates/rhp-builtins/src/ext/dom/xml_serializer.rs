//! The modern DOM's XML serializer (the DOM Parsing spec's algorithm with
//! php's errata), ported from php-src ext/dom/xml_serializer.c and
//! xml_document.c (PHP License 3.01). Output goes through an
//! `xmlOutputBuffer` in the document's encoding; the XML declaration,
//! DTDs and the other node types go through libxml2's save context.

use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use super::nscompat::{self, DomPrivate, XMLNS_NS_URI, XML_NS_URI};
use super::util::b;
use crate::ext::libxml::sys::{self, XmlAttr, XmlNode};

type R = Result<(), ()>;

fn w(out: *mut sys::xmlOutputBuffer, s: &[u8]) -> R {
    if unsafe { sys::xmlOutputBufferWrite(out, s.len() as c_int, s.as_ptr().cast()) } < 0 {
        Err(())
    } else {
        Ok(())
    }
}

fn wp(out: *mut sys::xmlOutputBuffer, p: *const u8) -> R {
    w(out, unsafe { b(p) })
}

fn try_(rc: c_int) -> R {
    if rc < 0 {
        Err(())
    } else {
        Ok(())
    }
}

/// The namespace prefix map: URI to candidate prefixes, in order.
#[derive(Clone, Default)]
struct PrefixMap(HashMap<Vec<u8>, Vec<Vec<u8>>>);

/// The local prefixes map: prefix to URI.
#[derive(Default)]
struct LocalMap(HashMap<Vec<u8>, Vec<u8>>);

fn opt(p: *const u8) -> Option<&'static [u8]> {
    if p.is_null() {
        None
    } else {
        Some(unsafe { b(p) })
    }
}

/// dom_xml_str_equals_treat_nulls_as_empty
fn eq_empty(a: Option<&[u8]>, c: Option<&[u8]>) -> bool {
    a.unwrap_or(b"") == c.unwrap_or(b"")
}

impl PrefixMap {
    fn add(&mut self, prefix: &[u8], ns: Option<&[u8]>) {
        self.0
            .entry(ns.unwrap_or(b"").to_vec())
            .or_default()
            .push(prefix.to_vec());
    }

    fn found(&self, prefix: &[u8], ns: &[u8]) -> bool {
        self.0
            .get(ns)
            .is_some_and(|l| l.iter().any(|p| p.as_slice() == prefix))
    }
}

impl LocalMap {
    fn add(&mut self, prefix: &[u8], ns: &[u8]) {
        self.0.entry(prefix.to_vec()).or_insert_with(|| ns.to_vec());
    }

    fn find(&self, prefix: &[u8]) -> Option<&[u8]> {
        self.0.get(prefix).map(|v| v.as_slice())
    }

    fn conflicts(&self, prefix: &[u8], ns: &[u8]) -> bool {
        match self.find(prefix) {
            None => false,
            Some(r) => !eq_empty(Some(r), Some(ns)),
        }
    }
}

fn attr_value(attr: *const XmlAttr) -> &'static [u8] {
    unsafe {
        let c = (*attr).children;
        if c.is_null() || (*c).content.is_null() {
            b""
        } else {
            b((*c).content)
        }
    }
}

/// dom_recording_the_namespace_information
fn record_ns_info(
    map: &mut PrefixMap,
    local: &mut LocalMap,
    element: *mut XmlNode,
) -> Option<Vec<u8>> {
    let mut default_ns = None;
    unsafe {
        let mut attr = (*element).properties;
        while !attr.is_null() {
            if nscompat::ns_is_fast(attr.cast(), &nscompat::XMLNS) {
                if (*(*attr).ns).prefix.is_null() {
                    default_ns = Some(attr_value(attr).to_vec());
                } else {
                    let prefix_def = b((*attr).name);
                    let ns_def = attr_value(attr);
                    if ns_def != XML_NS_URI && !map.found(prefix_def, ns_def) {
                        map.add(
                            prefix_def,
                            if ns_def.is_empty() {
                                None
                            } else {
                                Some(ns_def)
                            },
                        );
                        local.add(prefix_def, ns_def);
                    }
                }
            }
            attr = (*attr).next;
        }
    }
    default_ns
}

/// dom_retrieve_a_preferred_prefix_string
fn preferred_prefix(
    map: &PrefixMap,
    local: &LocalMap,
    preferred: Option<&[u8]>,
    ns: Option<&[u8]>,
) -> Option<Vec<u8>> {
    let ns = ns.unwrap_or(b"");
    let list = map.0.get(ns)?;
    let mut last_non_conflicting = None;
    for prefix in list.iter().rev() {
        if !local.conflicts(prefix, ns) {
            if eq_empty(preferred, Some(prefix)) {
                return Some(prefix.clone());
            }
            if last_non_conflicting.is_none() {
                last_non_conflicting = Some(prefix.clone());
            }
        }
    }
    last_non_conflicting
}

/// dom_xml_generate_a_prefix
fn generate_prefix(
    map: &mut PrefixMap,
    local: &mut LocalMap,
    ns: &[u8],
    index: &mut u32,
) -> Vec<u8> {
    let mut p;
    loop {
        p = format!("ns{index}").into_bytes();
        *index += 1;
        if local.find(&p).is_none() {
            break;
        }
    }
    map.add(&p, Some(ns));
    local.add(&p, ns);
    p
}

fn output_qname(out: *mut sys::xmlOutputBuffer, prefix: Option<&[u8]>, name: &[u8]) -> R {
    if let Some(p) = prefix {
        w(out, p)?;
        w(out, b":")?;
    }
    w(out, name)
}

/// dom_xml_common_text_serialization
fn text(out: *mut sys::xmlOutputBuffer, content: &[u8], attribute_mode: bool) -> R {
    let mut last = 0;
    for (i, &ch) in content.iter().enumerate() {
        let rep: &[u8] = match ch {
            b'&' => b"&amp;",
            b'<' => b"&lt;",
            b'>' => b"&gt;",
            b'"' if attribute_mode => b"&quot;",
            b'\t' if attribute_mode => b"&#9;",
            b'\n' if attribute_mode => b"&#10;",
            b'\r' if attribute_mode => b"&#13;",
            _ => continue,
        };
        w(out, &content[last..i])?;
        w(out, rep)?;
        last = i + 1;
    }
    w(out, &content[last..])
}

/// dom_xml_check_char_production
fn check_char_production(content: *const u8) -> R {
    let mut p = content;
    unsafe {
        while *p != 0 {
            let mut len: c_int = 4;
            let c = sys::xmlGetUTF8Char(p, &mut len);
            let ok = c >= 0
                && (c == 0x9
                    || c == 0xA
                    || c == 0xD
                    || (0x20..=0xD7FF).contains(&c)
                    || (0xE000..=0xFFFD).contains(&c)
                    || (0x10000..=0x10FFFF).contains(&c));
            if !ok {
                return Err(());
            }
            p = p.add(len as usize);
        }
    }
    Ok(())
}

fn attr_ns(attr: *const XmlAttr) -> Option<&'static [u8]> {
    unsafe {
        if (*attr).ns.is_null() {
            None
        } else {
            opt((*(*attr).ns).href)
        }
    }
}

fn attr_node_value(out: *mut sys::xmlOutputBuffer, attr: *mut XmlAttr) -> R {
    unsafe {
        wp(out, (*attr).name)?;
        w(out, b"=\"")?;
        let mut child = (*attr).children;
        while !child.is_null() {
            if (*child).type_ == sys::XML_TEXT_NODE {
                if !(*child).content.is_null() {
                    text(out, b((*child).content), true)?;
                }
            } else if (*child).type_ == sys::XML_ENTITY_REF_NODE {
                w(out, b"&")?;
                text(out, b((*child).name), true)?;
                w(out, b";")?;
            }
            child = (*child).next;
        }
    }
    w(out, b"\"")
}

fn validate_ncname(name: *const u8) -> bool {
    unsafe { sys::xmlValidateNCName(name, 0) == 0 }
}

/// dom_xml_serialize_attributes
#[allow(clippy::too_many_arguments)]
fn attributes(
    out: *mut sys::xmlOutputBuffer,
    element: *mut XmlNode,
    map: &mut PrefixMap,
    local: &mut LocalMap,
    index: &mut u32,
    ignore_ns_def_attr: bool,
    wf: bool,
) -> R {
    let mut localname_set: Vec<(Option<Vec<u8>>, Vec<u8>)> = Vec::new();
    unsafe {
        let mut attr = (*element).properties;
        while !attr.is_null() {
            let next = (*attr).next;
            if wf {
                let key = (attr_ns(attr).map(|s| s.to_vec()), b((*attr).name).to_vec());
                if localname_set.contains(&key) {
                    return Err(());
                }
                localname_set.push(key);
            }
            let attribute_ns = attr_ns(attr);
            let mut candidate: Option<Vec<u8>> = None;
            if let Some(ans) = attribute_ns {
                let aprefix = opt((*(*attr).ns).prefix);
                candidate = preferred_prefix(map, local, aprefix, Some(ans));
                if nscompat::ns_is_fast(attr.cast(), &nscompat::XMLNS) {
                    let value = attr_value(attr);
                    if value == XML_NS_URI {
                        attr = next;
                        continue;
                    }
                    if ignore_ns_def_attr && aprefix.is_none() {
                        let ens = if (*element).ns.is_null() {
                            None
                        } else {
                            opt((*(*element).ns).href)
                        };
                        if !eq_empty(ens, Some(value)) {
                            attr = next;
                            continue;
                        }
                    }
                    if aprefix.is_some() {
                        let name = b((*attr).name);
                        let lv = local.find(name);
                        if (lv.is_none() || lv != Some(value)) && map.found(name, value) {
                            attr = next;
                            continue;
                        }
                    }
                    if aprefix == Some(b"xmlns") {
                        candidate = Some(b"xmlns".to_vec());
                    }
                    if wf && (value == XMLNS_NS_URI || (value.is_empty() && candidate.is_some())) {
                        return Err(());
                    }
                } else if candidate.is_none() {
                    let c = match aprefix {
                        Some(p) if local.find(p).is_none() => {
                            map.add(p, Some(ans));
                            local.add(p, ans);
                            p.to_vec()
                        }
                        _ => generate_prefix(map, local, ans, index),
                    };
                    w(out, b" xmlns:")?;
                    w(out, &c)?;
                    w(out, b"=\"")?;
                    text(out, ans, true)?;
                    w(out, b"\"")?;
                    candidate = Some(c);
                }
            }
            w(out, b" ")?;
            if let Some(c) = &candidate {
                w(out, c)?;
                w(out, b":")?;
            }
            if wf
                && (!validate_ncname((*attr).name)
                    || (b((*attr).name) == b"xmlns" && attr_ns(attr).is_none()))
            {
                return Err(());
            }
            attr_node_value(out, attr)?;
            attr = next;
        }
    }
    Ok(())
}

fn should_format_element(element: *mut XmlNode) -> bool {
    unsafe {
        let mut child = (*element).children;
        while !child.is_null() {
            if matches!(
                (*child).type_,
                sys::XML_TEXT_NODE | sys::XML_ENTITY_REF_NODE | sys::XML_CDATA_SECTION_NODE
            ) {
                return false;
            }
            child = (*child).next;
        }
    }
    true
}

fn indents(out: *mut sys::xmlOutputBuffer, indent: i32) -> R {
    w(out, b"\n")?;
    for _ in 0..indent {
        w(out, b"  ")?;
    }
    Ok(())
}

const VOID_ELEMENTS: &[&[u8]] = &[
    b"area",
    b"base",
    b"basefont",
    b"bgsound",
    b"br",
    b"col",
    b"embed",
    b"frame",
    b"hr",
    b"img",
    b"input",
    b"keygen",
    b"link",
    b"menuitem",
    b"meta",
    b"param",
    b"source",
    b"track",
    b"wbr",
];

struct Ctx<'a> {
    ctxt: *mut sys::xmlSaveCtxt,
    out: *mut sys::xmlOutputBuffer,
    private: Option<&'a DomPrivate>,
}

/// dom_xml_serialize_element_node
fn element(
    ctx: &Ctx,
    namespace: Option<&[u8]>,
    ns_map: &PrefixMap,
    element: *mut XmlNode,
    index: &mut u32,
    mut indent: i32,
    wf: bool,
) -> R {
    let out = ctx.out;
    unsafe {
        if wf && !validate_ncname((*element).name) {
            return Err(());
        }
        let should_format =
            indent >= 0 && !(*element).children.is_null() && should_format_element(element);
        w(out, b"<")?;
        let name = b((*element).name);
        let mut qprefix: Option<Vec<u8>> = None;
        let mut skip_end_tag = false;
        let mut ignore_ns_def_attr = false;
        let mut map = ns_map.clone();
        let mut local = LocalMap::default();
        let local_default = record_ns_info(&mut map, &mut local, element);
        let mut inherited: Option<Vec<u8>> = namespace.map(|s| s.to_vec());
        let ns = if (*element).ns.is_null() {
            None
        } else {
            opt((*(*element).ns).href)
        };
        if inherited.as_deref() == ns {
            if local_default.is_some() {
                ignore_ns_def_attr = true;
            }
            if nscompat::ns_is_fast(element, &nscompat::XML) {
                qprefix = Some(b"xml".to_vec());
            }
            output_qname(out, qprefix.as_deref(), name)?;
        } else {
            let prefix = if (*element).ns.is_null() {
                None
            } else {
                opt((*(*element).ns).prefix)
            };
            let mut candidate = if prefix.is_none() && eq_empty(ns, local_default.as_deref()) {
                None
            } else {
                preferred_prefix(&map, &local, prefix, ns)
            };
            if prefix == Some(b"xmlns") {
                if wf {
                    return Err(());
                }
                candidate = prefix.map(|p| p.to_vec());
            }
            if let Some(c) = candidate {
                if let Some(ld) = &local_default {
                    if ld.as_slice() != XML_NS_URI {
                        inherited = if ld.is_empty() {
                            None
                        } else {
                            Some(ld.clone())
                        };
                    }
                }
                qprefix = Some(c);
                output_qname(out, qprefix.as_deref(), name)?;
            } else if let Some(p) = prefix {
                let nsb = ns.unwrap_or(b"");
                let p = if local.find(p).is_some() {
                    generate_prefix(&mut map, &mut local, nsb, index)
                } else {
                    map.add(p, Some(nsb));
                    local.add(p, nsb);
                    p.to_vec()
                };
                qprefix = Some(p.clone());
                output_qname(out, qprefix.as_deref(), name)?;
                w(out, b" xmlns:")?;
                w(out, &p)?;
                w(out, b"=\"")?;
                text(out, nsb, true)?;
                w(out, b"\"")?;
                if let Some(ld) = &local_default {
                    inherited = if ld.is_empty() {
                        None
                    } else {
                        Some(ld.clone())
                    };
                }
            } else if local_default.is_none() || !eq_empty(local_default.as_deref(), ns) {
                ignore_ns_def_attr = true;
                inherited = ns.map(|s| s.to_vec());
                output_qname(out, None, name)?;
                w(out, b" xmlns=\"")?;
                text(out, ns.unwrap_or(b""), true)?;
                w(out, b"\"")?;
            } else {
                inherited = ns.map(|s| s.to_vec());
                output_qname(out, None, name)?;
            }
        }
        attributes(
            out,
            element,
            &mut map,
            &mut local,
            index,
            ignore_ns_def_attr,
            wf,
        )?;
        if (*element).children.is_null() {
            let save_no_empty = sys::rhp_xml_set_save_no_empty_tags(0);
            sys::rhp_xml_set_save_no_empty_tags(save_no_empty);
            if save_no_empty != 0 {
            } else if nscompat::ns_is_fast(element, &nscompat::HTML) {
                if VOID_ELEMENTS.contains(&name) {
                    w(out, b" /")?;
                    skip_end_tag = true;
                }
            } else {
                w(out, b"/")?;
                skip_end_tag = true;
            }
        }
        w(out, b">")?;
        if !skip_end_tag {
            if should_format {
                indent += 1;
            } else {
                indent = -1;
            }
            let mut child = if nscompat::ns_is_fast(element, &nscompat::HTML) && name == b"template"
            {
                match ctx.private {
                    Some(p) => nscompat::retrieve_templated_content(p, element),
                    None => ptr::null_mut(),
                }
            } else {
                (*element).children
            };
            while !child.is_null() {
                if should_format {
                    indents(out, indent)?;
                }
                algorithm(ctx, &map, child, inherited.as_deref(), index, indent, wf)?;
                child = (*child).next;
            }
            if should_format {
                indent -= 1;
                indents(out, indent)?;
            }
            w(out, b"</")?;
            output_qname(out, qprefix.as_deref(), name)?;
            w(out, b">")?;
        }
    }
    Ok(())
}

/// dom_xml_serialization_algorithm
fn algorithm(
    ctx: &Ctx,
    map: &PrefixMap,
    node: *mut XmlNode,
    namespace: Option<&[u8]>,
    index: &mut u32,
    indent: i32,
    wf: bool,
) -> R {
    let out = ctx.out;
    unsafe {
        match (*node).type_ {
            sys::XML_ELEMENT_NODE => element(ctx, namespace, map, node, index, indent, wf),
            sys::XML_DOCUMENT_FRAG_NODE => {
                let mut child = (*node).children;
                while !child.is_null() {
                    algorithm(ctx, map, child, namespace, index, indent, wf)?;
                    child = (*child).next;
                }
                Ok(())
            }
            sys::XML_HTML_DOCUMENT_NODE | sys::XML_DOCUMENT_NODE => {
                let mut child = (*node).children;
                (*node).children = ptr::null_mut();
                try_(sys::xmlOutputBufferFlush(out))?;
                let r = sys::xmlSaveDoc(ctx.ctxt, node.cast());
                let r2 = sys::xmlSaveFlush(ctx.ctxt);
                (*node).children = child;
                if r < 0 || r2 < 0 {
                    return Err(());
                }
                while !child.is_null() {
                    algorithm(ctx, map, child, namespace, index, indent, wf)?;
                    child = (*child).next;
                }
                Ok(())
            }
            sys::XML_TEXT_NODE => {
                if wf && !(*node).content.is_null() {
                    check_char_production((*node).content)?;
                }
                if (*node).content.is_null() {
                    return Ok(());
                }
                text(out, b((*node).content), false)
            }
            sys::XML_COMMENT_NODE => {
                let p = (*node).content;
                if wf && !p.is_null() {
                    check_char_production(p)?;
                    let s = b(p);
                    if s.windows(2).any(|x| x == b"--") || s.last() == Some(&b'-') {
                        return Err(());
                    }
                }
                w(out, b"<!--")?;
                if !p.is_null() {
                    wp(out, p)?;
                }
                w(out, b"-->")
            }
            sys::XML_PI_NODE => {
                if wf {
                    let n = b((*node).name);
                    if n.contains(&b':') || n.eq_ignore_ascii_case(b"xml") {
                        return Err(());
                    }
                    if !(*node).content.is_null() {
                        check_char_production((*node).content)?;
                        if b((*node).content).windows(2).any(|x| x == b"?>") {
                            return Err(());
                        }
                    }
                }
                w(out, b"<?")?;
                wp(out, (*node).name)?;
                w(out, b" ")?;
                if !(*node).content.is_null() {
                    wp(out, (*node).content)?;
                }
                w(out, b"?>")
            }
            sys::XML_CDATA_SECTION_NODE => {
                w(out, b"<![CDATA[")?;
                if !(*node).content.is_null() {
                    wp(out, (*node).content)?;
                }
                w(out, b"]]>")
            }
            sys::XML_ATTRIBUTE_NODE => {
                if !(*node).ns.is_null() && !(*(*node).ns).prefix.is_null() {
                    wp(out, (*(*node).ns).prefix)?;
                    w(out, b":")?;
                }
                attr_node_value(out, node.cast())
            }
            t => {
                try_(sys::xmlOutputBufferFlush(out))?;
                if sys::xmlSaveTree(ctx.ctxt, node) < 0 {
                    return Err(());
                }
                try_(sys::xmlSaveFlush(ctx.ctxt))?;
                if t == sys::XML_DTD_NODE {
                    w(out, b"\n")?;
                }
                Ok(())
            }
        }
    }
}

/// dom_xml_serialize
pub fn serialize(
    ctxt: *mut sys::xmlSaveCtxt,
    out: *mut sys::xmlOutputBuffer,
    node: *mut XmlNode,
    format: bool,
    require_well_formed: bool,
    private: Option<&DomPrivate>,
) -> c_int {
    let mut map = PrefixMap::default();
    map.add(b"xml", Some(XML_NS_URI));
    let mut index = 1;
    let ctx = Ctx { ctxt, out, private };
    let indent = if format { 0 } else { -1 };
    match algorithm(
        &ctx,
        &map,
        node,
        None,
        &mut index,
        indent,
        require_well_formed,
    ) {
        Ok(()) => 0,
        Err(()) => -1,
    }
}

unsafe extern "C" fn write_vec(ctx: *mut c_void, buf: *const c_char, len: c_int) -> c_int {
    let v = unsafe { &mut *(ctx as *mut Vec<u8>) };
    v.extend_from_slice(unsafe { std::slice::from_raw_parts(buf.cast::<u8>(), len as usize) });
    len
}

pub const XML_SAVE_FORMAT: c_int = 1 << 0;
pub const XML_SAVE_AS_XML: c_int = 1 << 5;

/// php_new_dom_dump_node_to_str_ex: None on failure.
pub fn dump_to_str(
    node: *mut XmlNode,
    options: c_int,
    format: bool,
    encoding: *const u8,
    require_well_formed: bool,
    private: Option<&DomPrivate>,
) -> Option<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    let bp = &mut buf as *mut Vec<u8> as *mut c_void;
    let mut status: c_int = -1;
    unsafe {
        let ctxt = sys::xmlSaveToIO(
            Some(write_vec),
            None,
            bp,
            encoding.cast(),
            XML_SAVE_AS_XML | options,
        );
        if !ctxt.is_null() {
            let handler = sys::xmlFindCharEncodingHandler(encoding.cast());
            let out = sys::xmlOutputBufferCreateIO(Some(write_vec), None, bp, handler);
            if !out.is_null() {
                status = serialize(ctxt, out, node, format, require_well_formed, private);
                status |= sys::xmlOutputBufferFlush(out);
                status |= sys::xmlOutputBufferClose(out);
            } else if !handler.is_null() {
                sys::xmlCharEncCloseFunc(handler);
            }
            status |= sys::xmlSaveClose(ctxt);
        }
    }
    (status >= 0).then_some(buf)
}

/// dom_element_html_fragment_serialize's XML half: the children of `node`
/// with the require-well-formed flag, each on its own prefix map.
pub fn fragment_to_str(node: *mut XmlNode, private: Option<&DomPrivate>) -> Option<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    let bp = &mut buf as *mut Vec<u8> as *mut c_void;
    let mut status: c_int = -1;
    unsafe {
        let ctxt = sys::xmlSaveToIO(
            Some(write_vec),
            None,
            bp,
            c"UTF-8".as_ptr(),
            XML_SAVE_AS_XML,
        );
        if !ctxt.is_null() {
            let handler = sys::xmlFindCharEncodingHandler(c"UTF-8".as_ptr());
            let out = sys::xmlOutputBufferCreateIO(Some(write_vec), None, bp, handler);
            if !out.is_null() {
                status = 0;
                let mut child = (*node).children;
                while !child.is_null() && status == 0 {
                    status = serialize(ctxt, out, child, false, true, private);
                    child = (*child).next;
                }
                status |= sys::xmlOutputBufferFlush(out);
                status |= sys::xmlOutputBufferClose(out);
            }
            status |= sys::xmlSaveClose(ctxt);
        }
    }
    (status >= 0).then_some(buf)
}
