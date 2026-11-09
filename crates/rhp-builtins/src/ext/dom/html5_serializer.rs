//! The HTML5 fragment serialization algorithm, ported from php-src
//! ext/dom/html5_serializer.c (PHP License 3.01). Writes UTF-8; the
//! caller converts to the document's encoding.

use super::nscompat::{self, DomPrivate};
use super::util::b;
use crate::ext::libxml::sys::{self, XmlDtd, XmlNode};

fn escape(out: &mut Vec<u8>, content: &[u8], attribute_mode: bool) {
    let mut i = 0;
    let mut last = 0;
    while i < content.len() {
        let rep: Option<&[u8]> = match content[i] {
            b'&' => Some(b"&amp;"),
            0xC2 if content.get(i + 1) == Some(&0xA0) => {
                out.extend_from_slice(&content[last..i]);
                out.extend_from_slice(b"&nbsp;");
                i += 2;
                last = i;
                continue;
            }
            b'"' if attribute_mode => Some(b"&quot;"),
            b'<' if !attribute_mode => Some(b"&lt;"),
            b'>' if !attribute_mode => Some(b"&gt;"),
            _ => None,
        };
        if let Some(r) = rep {
            out.extend_from_slice(&content[last..i]);
            out.extend_from_slice(r);
            last = i + 1;
        }
        i += 1;
    }
    out.extend_from_slice(&content[last..]);
}

const RAW_TEXT_PARENTS: &[&[u8]] = &[
    b"style",
    b"script",
    b"xmp",
    b"iframe",
    b"noembed",
    b"noframes",
    b"plaintext",
];

fn text_node(out: &mut Vec<u8>, node: *const XmlNode) {
    unsafe {
        if (*node).content.is_null() {
            return;
        }
        let content = b((*node).content);
        let parent = (*node).parent;
        if !parent.is_null()
            && (*parent).type_ == sys::XML_ELEMENT_NODE
            && nscompat::ns_is_fast(parent, &nscompat::HTML)
            && RAW_TEXT_PARENTS.contains(&b((*parent).name))
        {
            out.extend_from_slice(content);
            return;
        }
        escape(out, content, false);
    }
}

fn tag_name(out: &mut Vec<u8>, node: *const XmlNode) {
    unsafe {
        let ns = (*node).ns;
        if !ns.is_null()
            && !(*ns).prefix.is_null()
            && !(nscompat::ns_is_fast(node, &nscompat::HTML)
                || nscompat::ns_is_fast(node, &nscompat::MATHML)
                || nscompat::ns_is_fast(node, &nscompat::SVG))
        {
            out.extend_from_slice(b((*ns).prefix));
            out.push(b':');
        }
        out.extend_from_slice(b((*node).name));
    }
}

fn element_start(out: &mut Vec<u8>, node: *const XmlNode) {
    out.push(b'<');
    tag_name(out, node);
    unsafe {
        let mut attr = (*node).properties;
        while !attr.is_null() {
            out.push(b' ');
            let name = b((*attr).name);
            let an = attr as *const XmlNode;
            if (*attr).ns.is_null() {
                out.extend_from_slice(name);
            } else if nscompat::ns_is_fast(an, &nscompat::XML) {
                out.extend_from_slice(b"xml:");
                out.extend_from_slice(name);
            } else if nscompat::ns_is_fast(an, &nscompat::XMLNS) {
                if name == b"xmlns" {
                    out.extend_from_slice(b"xmlns");
                } else {
                    out.extend_from_slice(b"xmlns:");
                    out.extend_from_slice(name);
                }
            } else if nscompat::ns_is_fast(an, &nscompat::XLINK) {
                out.extend_from_slice(b"xlink:");
                out.extend_from_slice(name);
            } else if (*(*attr).ns).prefix.is_null() {
                out.extend_from_slice(name);
            } else {
                out.extend_from_slice(b((*(*attr).ns).prefix));
                out.push(b':');
                out.extend_from_slice(name);
            }
            out.extend_from_slice(b"=\"");
            let mut child = (*attr).children;
            while !child.is_null() {
                if (*child).type_ == sys::XML_TEXT_NODE {
                    if !(*child).content.is_null() {
                        escape(out, b((*child).content), true);
                    }
                } else if (*child).type_ == sys::XML_ENTITY_REF_NODE {
                    out.push(b'&');
                    escape(out, b((*child).name), true);
                    out.push(b';');
                }
                child = (*child).next;
            }
            out.push(b'"');
            attr = (*attr).next;
        }
    }
    out.push(b'>');
}

const VOID: &[&[u8]] = &[
    b"area",
    b"base",
    b"br",
    b"col",
    b"embed",
    b"hr",
    b"img",
    b"input",
    b"link",
    b"meta",
    b"source",
    b"track",
    b"wbr",
    b"basefont",
    b"bgsound",
    b"frame",
    b"keygen",
    b"param",
];

/// dom_html5_serializes_as_void
pub fn serializes_as_void(node: *const XmlNode) -> bool {
    nscompat::ns_is_fast(node, &nscompat::HTML) && VOID.contains(&unsafe { b((*node).name) })
}

fn element_end(out: &mut Vec<u8>, node: *const XmlNode) {
    if !serializes_as_void(node) {
        out.extend_from_slice(b"</");
        tag_name(out, node);
        out.push(b'>');
    }
}

fn template_content(p: Option<&DomPrivate>, node: *const XmlNode) -> *mut XmlNode {
    match p {
        Some(p) => nscompat::retrieve_templated_content(p, node),
        None => std::ptr::null_mut(),
    }
}

/// dom_html5_serialize_node
fn serialize_node(
    out: &mut Vec<u8>,
    p: Option<&DomPrivate>,
    mut node: *const XmlNode,
    bound: *const XmlNode,
) {
    unsafe {
        while !node.is_null() {
            match (*node).type_ {
                sys::XML_DTD_NODE => {
                    out.extend_from_slice(b"<!DOCTYPE ");
                    out.extend_from_slice(b((*(node as *const XmlDtd)).name));
                    out.push(b'>');
                }
                sys::XML_CDATA_SECTION_NODE | sys::XML_TEXT_NODE => text_node(out, node),
                sys::XML_PI_NODE => {
                    out.extend_from_slice(b"<?");
                    out.extend_from_slice(b((*node).name));
                    out.push(b' ');
                    if !(*node).content.is_null() {
                        out.extend_from_slice(b((*node).content));
                    }
                    out.push(b'>');
                }
                sys::XML_COMMENT_NODE => {
                    out.extend_from_slice(b"<!--");
                    if !(*node).content.is_null() {
                        out.extend_from_slice(b((*node).content));
                    }
                    out.extend_from_slice(b"-->");
                }
                sys::XML_ELEMENT_NODE => {
                    element_start(out, node);
                    let mut children = (*node).children as *const XmlNode;
                    if nscompat::ns_is_fast(node, &nscompat::HTML) && b((*node).name) == b"template"
                    {
                        children = template_content(p, node);
                    }
                    if !children.is_null() {
                        if !serializes_as_void(node) {
                            node = children;
                            continue;
                        }
                    } else {
                        element_end(out, node);
                    }
                }
                sys::XML_DOCUMENT_FRAG_NODE => {
                    if !(*node).children.is_null() {
                        node = (*node).children;
                        continue;
                    }
                }
                sys::XML_ENTITY_REF_NODE => {
                    out.push(b'&');
                    out.extend_from_slice(b((*node).name));
                    out.push(b';');
                }
                _ => {}
            }
            if !(*node).next.is_null() {
                node = (*node).next;
            } else {
                loop {
                    node = (*node).parent;
                    if node == bound || node.is_null() {
                        return;
                    }
                    if (*node).type_ == sys::XML_ELEMENT_NODE {
                        element_end(out, node);
                    }
                    if !(*node).next.is_null() {
                        break;
                    }
                }
                node = (*node).next;
            }
        }
    }
}

/// dom_html5_serialize: the node's children.
pub fn serialize(out: &mut Vec<u8>, p: Option<&DomPrivate>, node: *const XmlNode) {
    unsafe {
        let t = (*node).type_;
        if !matches!(
            t,
            sys::XML_ELEMENT_NODE
                | sys::XML_DOCUMENT_FRAG_NODE
                | sys::XML_DOCUMENT_NODE
                | sys::XML_HTML_DOCUMENT_NODE
        ) {
            return;
        }
        if t == sys::XML_ELEMENT_NODE && serializes_as_void(node) {
            return;
        }
        let mut children = template_content(p, node) as *const XmlNode;
        if children.is_null() {
            children = (*node).children;
        }
        serialize_node(out, p, children, node);
    }
}

/// dom_html5_serialize_outer
pub fn serialize_outer(out: &mut Vec<u8>, p: Option<&DomPrivate>, node: *mut XmlNode) {
    unsafe {
        let t = (*node).type_;
        if matches!(
            t,
            sys::XML_DOCUMENT_NODE | sys::XML_HTML_DOCUMENT_NODE | sys::XML_DOCUMENT_FRAG_NODE
        ) {
            let c = (*node).children;
            if c.is_null() {
                return;
            }
            serialize_node(out, p, c, (*c).parent);
        } else {
            let old_next = (*node).next;
            (*node).next = std::ptr::null_mut();
            serialize_node(out, p, node, (*node).parent);
            (*node).next = old_next;
        }
    }
}
