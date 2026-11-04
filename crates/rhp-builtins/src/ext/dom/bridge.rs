//! A temporary copy bridge between DOM (libxml2 trees) and SimpleXML /
//! XMLReader, which still run on the hand-written tree in
//! `ext/libxml/tree.rs`. php shares one node between the APIs; until
//! SimpleXML and XMLReader move to libxml2, `simplexml_import_dom`,
//! `dom_import_simplexml` and `XMLReader::expand` copy the document across,
//! node by node, and hand out the matching node of the copy. Changes made
//! through one API are not seen by the other.

use std::ptr;

use rhp_values::Value;

use super::util::{b, c, nul};
use super::{create_object, intern_of, Cx};
use crate::ext::libxml::sys::{self, XmlDoc, XmlNode, XmlNs};
use crate::ext::libxml::tree::{self, Doc, DocId, DocTypeInfo, Kind, Node, NodeId};
use crate::{Host, Thrown};

/// Copy `n` (a libxml2 node) and its subtree under `parent` of `doc`;
/// records the copy of `target`.
fn copy_to_old(
    doc: &mut Doc,
    parent: NodeId,
    n: *mut XmlNode,
    target: *mut XmlNode,
    found: &mut Option<NodeId>,
) {
    unsafe {
        let kind = match (*n).type_ {
            sys::XML_ELEMENT_NODE => Kind::Element,
            sys::XML_TEXT_NODE => Kind::Text,
            sys::XML_CDATA_SECTION_NODE => Kind::CData,
            sys::XML_ENTITY_REF_NODE => Kind::EntityRef,
            sys::XML_PI_NODE => Kind::Pi,
            sys::XML_COMMENT_NODE => Kind::Comment,
            sys::XML_DTD_NODE => Kind::DocType,
            _ => return,
        };
        let mut node = Node::new(kind);
        node.line = (*n).line as u32;
        match kind {
            Kind::Element => {
                node.name = b((*n).name).to_vec();
                if !(*n).ns.is_null() {
                    node.prefix = b((*(*n).ns).prefix).to_vec();
                    node.ns = Some(b((*(*n).ns).href).to_vec());
                }
                let mut ns = (*n).nsDef;
                while !ns.is_null() {
                    node.ns_defs
                        .push((b((*ns).prefix).to_vec(), b((*ns).href).to_vec()));
                    ns = (*ns).next;
                }
            }
            Kind::DocType => {
                let dtd = n as *mut sys::XmlDtd;
                node.name = b((*dtd).name).to_vec();
                let opt = |p: *const u8| (!p.is_null()).then(|| b(p).to_vec());
                node.doctype = Some(Box::new(DocTypeInfo {
                    public_id: opt((*dtd).ExternalID),
                    system_id: opt((*dtd).SystemID),
                    subset: None,
                }));
            }
            Kind::EntityRef | Kind::Pi => {
                node.name = b((*n).name).to_vec();
                node.content = b((*n).content).to_vec();
            }
            _ => node.content = b((*n).content).to_vec(),
        }
        let id = doc.add(node);
        doc.insert(parent, id, None);
        if n == target {
            *found = Some(id);
        }
        if kind != Kind::Element {
            return;
        }
        let mut a = (*n).properties;
        while !a.is_null() {
            let mut attr = Node::new(Kind::Attr);
            attr.name = b((*a).name).to_vec();
            if !(*a).ns.is_null() {
                attr.prefix = b((*(*a).ns).prefix).to_vec();
                attr.ns = Some(b((*(*a).ns).href).to_vec());
            }
            attr.content = crate::ext::libxml::node::attr_value(a);
            let aid = doc.add(attr);
            doc.insert(id, aid, None);
            if a.cast::<XmlNode>() == target {
                *found = Some(aid);
            }
            a = (*a).next;
        }
        let mut ch = (*n).children;
        while !ch.is_null() {
            copy_to_old(doc, id, ch, target, found);
            ch = (*ch).next;
        }
    }
}

/// The old tree's (document, node) for a DOM node: a copy of its document.
pub fn to_old_tree(v: &Value) -> Option<(DocId, NodeId)> {
    let d = intern_of(v)?;
    let node = d.node();
    if node.is_null() {
        return None;
    }
    let docp = unsafe { (*node).doc };
    if docp.is_null() {
        return None;
    }
    let mut doc = Doc::new();
    unsafe {
        let opt = |p: *const u8| (!p.is_null()).then(|| b(p).to_vec());
        doc.version = opt((*docp).version);
        doc.encoding = opt((*docp).encoding);
        doc.standalone = (*docp).standalone.clamp(-1, 1) as i8;
        doc.url = opt((*docp).URL);
    }
    let mut found = (node == docp.cast()).then_some(0);
    let mut ch = unsafe { (*docp).children };
    while !ch.is_null() {
        copy_to_old(&mut doc, 0, ch, node, &mut found);
        ch = unsafe { (*ch).next };
    }
    let found = found?;
    Some((tree::register(doc), found))
}

/// The namespace `prefix` names at `node`, the xml namespace included.
unsafe fn search_ns(doc: *mut XmlDoc, node: *mut XmlNode, prefix: &[u8]) -> *mut XmlNs {
    let cp = c(prefix);
    unsafe {
        sys::xmlSearchNs(
            doc,
            node,
            if prefix.is_empty() {
                ptr::null()
            } else {
                cp.as_ptr()
            },
        )
    }
}

/// Copy old-tree node `id` and its subtree under `parent`; returns the
/// copy of `target` if it was in it.
fn copy_from_old(
    old: &Doc,
    id: NodeId,
    docp: *mut XmlDoc,
    parent: *mut XmlNode,
    target: NodeId,
    found: &mut *mut XmlNode,
) {
    let n = &old.nodes[id];
    unsafe {
        let new: *mut XmlNode = match n.kind {
            Kind::Element => {
                let cn = c(&n.name);
                let e = sys::xmlNewDocNode(docp, ptr::null_mut(), cn.as_ptr(), ptr::null());
                for (p, u) in &n.ns_defs {
                    let cu = c(u);
                    let cp = c(p);
                    sys::xmlNewNs(
                        e,
                        cu.as_ptr(),
                        if p.is_empty() {
                            ptr::null()
                        } else {
                            cp.as_ptr()
                        },
                    );
                }
                e
            }
            Kind::Text => {
                sys::xmlNewDocTextLen(docp, nul(&n.content).as_ptr(), n.content.len() as i32)
            }
            Kind::CData => {
                sys::xmlNewCDataBlock(docp, nul(&n.content).as_ptr(), n.content.len() as i32)
            }
            Kind::Comment => {
                let cc = c(&n.content);
                sys::xmlNewDocComment(docp, cc.as_ptr())
            }
            Kind::Pi => {
                let cn = c(&n.name);
                let cc = c(&n.content);
                sys::xmlNewDocPI(docp, cn.as_ptr(), cc.as_ptr())
            }
            Kind::EntityRef => {
                let cn = c(&n.name);
                sys::xmlNewReference(docp, cn.as_ptr())
            }
            Kind::DocType => {
                let info = n.doctype.as_deref();
                let cn = c(&n.name);
                let pid = info.and_then(|i| i.public_id.as_deref()).map(c);
                let sid = info.and_then(|i| i.system_id.as_deref()).map(c);
                sys::xmlCreateIntSubset(
                    docp,
                    cn.as_ptr(),
                    pid.as_ref().map_or(ptr::null(), |p| p.as_ptr()),
                    sid.as_ref().map_or(ptr::null(), |p| p.as_ptr()),
                );
                if id == target {
                    *found = (*docp).intSubset.cast();
                }
                return;
            }
            _ => return,
        };
        if new.is_null() {
            return;
        }
        sys::xmlAddChild(parent, new);
        if id == target {
            *found = new;
        }
        if n.kind != Kind::Element {
            return;
        }
        if n.ns.is_some() {
            (*new).ns = search_ns(docp, new, &n.prefix);
        }
        for &a in &n.attrs {
            let an = &old.nodes[a];
            let cn = c(&an.name);
            let cv = c(&an.content);
            let ns = if an.prefix.is_empty() {
                ptr::null_mut()
            } else {
                search_ns(docp, new, &an.prefix)
            };
            let attr = sys::xmlSetNsProp(new, ns, cn.as_ptr(), cv.as_ptr());
            if a == target {
                *found = attr.cast();
            }
        }
        for ch in old.children(id) {
            copy_from_old(old, ch, docp, new, target, found);
        }
    }
}

/// The DOM node for `(d, n)` of the old tree: a node of a libxml2 copy of
/// its document.
pub fn from_old_tree(host: &mut dyn Host, d: DocId, n: NodeId) -> Result<Value, Thrown> {
    let built = tree::with_doc(d, |old| unsafe {
        let version = c(old.version.as_deref().unwrap_or(b"1.0"));
        let docp = sys::xmlNewDoc(version.as_ptr());
        if let Some(e) = &old.encoding {
            let ce = c(e);
            (*docp).encoding = sys::xmlStrdup(ce.as_ptr());
        }
        (*docp).standalone = old.standalone as i32;
        if let Some(u) = &old.url {
            let cu = c(u);
            (*docp).URL = sys::xmlStrdup(cu.as_ptr());
        }
        let mut found: *mut XmlNode = if n == 0 { docp.cast() } else { ptr::null_mut() };
        for ch in old.children(0) {
            copy_from_old(old, ch, docp, docp.cast(), n, &mut found);
        }
        (docp, found)
    });
    let Some((docp, found)) = built else {
        return Ok(Value::Null);
    };
    let mut cx = Cx {
        host,
        fname: String::new(),
        quiet: false,
        lazy_fname: false,
    };
    let docv = create_object(&mut cx, docp.cast(), None)?;
    if found.is_null() {
        return Ok(Value::Null);
    }
    let docobj = intern_of(&docv).expect("a document object");
    create_object(&mut cx, found, Some(&docobj))
}

/// dom_import_simplexml over the bridge.
pub fn import_simplexml(host: &mut dyn Host, v: &Value) -> Result<Value, Thrown> {
    let bad = || {
        Thrown::type_error("dom_import_simplexml(): Argument #1 ($node) is not a valid node type")
    };
    let Some((d, n)) = crate::ext::simplexml::old_tree_node(v) else {
        return Err(bad());
    };
    let kind = tree::with_doc(d, |doc| doc.nodes.get(n).map(|x| x.kind)).flatten();
    if !matches!(kind, Some(Kind::Element) | Some(Kind::Attr)) {
        return Err(bad());
    }
    from_old_tree(host, d, n)
}
