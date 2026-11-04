//! The `SimpleXML` extension over libxml2 trees, shared with DOM, ported
//! from php-src ext/simplexml/simplexml.c (PHP License 3.01).
//!
//! `SimpleXMLElement` is PHP source (`prelude.php`). Its methods call
//! `__sxe("SimpleXMLElement::method", $this, ...)`. Property and dimension
//! access reach php's handlers (`sxe_prop_dim_read`/`write`/`exists`/
//! `delete`) through the magic accessors (`__sxe_prop`) and ArrayAccess
//! (`__sxe_dim`). An object's state is its payload, an [`SxeObject`]: php's
//! `php_sxe_object`, whose node and document references are php's
//! (`crate::ext::libxml::node`).

use std::cell::{Cell, RefCell};
use std::ffi::c_int;
use std::ptr;
use std::rc::Rc;

use rhp_semantics::deref;
use rhp_values::{conv, ArrayKey, ObjectHandle, PhpArray, PhpStr, Value};

use crate::argparse::{zval_get_string, zval_value_name};
use crate::ext::libxml::node::{self as lxn, ClassType, NodeObject};
use crate::ext::libxml::sys::{self, XmlAttr, XmlDoc, XmlNode, XmlNs};
use crate::ext::libxml::{self as lx, ffi};
use crate::object::{self, CastTarget, ClassHandlers, CloneObj, ObjectHandlers, PropPurpose};
use crate::{Host, Thrown};

const INT_MAX: i64 = i32::MAX as i64;

/// `SXE_ITER`
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum IterType {
    #[default]
    None,
    Element,
    Child,
    AttrList,
}

/// php's `sxe->iter`.
#[derive(Default)]
struct Iter {
    name: Option<Vec<u8>>,
    nsprefix: Option<Vec<u8>>,
    isprefix: bool,
    ty: IterType,
    data: Option<Value>,
}

/// php's `php_sxe_object`, as an object's payload.
pub(crate) struct SxeObject {
    base: NodeObject,
    /// The class of the objects this one hands out (`sxe->zo.ce`).
    class: Vec<u8>,
    iter: RefCell<Iter>,
    xpath: Cell<*mut sys::xmlXPathContext>,
}

impl SxeObject {
    fn new(class: Vec<u8>) -> SxeObject {
        SxeObject {
            base: NodeObject::default(),
            class,
            iter: RefCell::new(Iter::default()),
            xpath: Cell::new(ptr::null_mut()),
        }
    }

    fn ty(&self) -> IterType {
        self.iter.borrow().ty
    }

    fn nsprefix(&self) -> (Option<Vec<u8>>, bool) {
        let it = self.iter.borrow();
        (it.nsprefix.clone(), it.isprefix)
    }

    fn document_ptr(&self) -> *mut XmlDoc {
        let d = self.base.document.get();
        if d.is_null() {
            ptr::null_mut()
        } else {
            unsafe { (*d).ptr }
        }
    }

    /// sxe_object_free_iterxpath
    fn free_iterxpath(&self) {
        let data = {
            let mut it = self.iter.borrow_mut();
            it.name = None;
            it.nsprefix = None;
            it.data.take()
        };
        drop(data);
        lxn::node_decrement_resource(&self.base);
        let x = self.xpath.replace(ptr::null_mut());
        if !x.is_null() {
            unsafe { sys::xmlXPathFreeContext(x) };
        }
    }
}

impl Drop for SxeObject {
    fn drop(&mut self) {
        self.free_iterxpath();
    }
}

/// The payload of a SimpleXML object.
fn sxe_payload(v: &Value) -> Option<Rc<SxeObject>> {
    object::payload::<SxeObject>(v)
}

/// Whether `v` is a SimpleXMLElement.
pub fn is_sxe_object(v: &Value) -> bool {
    sxe_payload(v).is_some()
}

/// The payload of `this`, created empty for an object that has none yet
/// (a subclass constructor that skipped the parent's, or
/// `newInstanceWithoutConstructor`).
fn sxe_of(host: &dyn Host, v: &Value) -> Option<Rc<SxeObject>> {
    if let Some(s) = sxe_payload(v) {
        return Some(s);
    }
    let h = object::object_of(v)?;
    let class = host
        .object_class_name(v)
        .unwrap_or_else(|| b"SimpleXMLElement".to_vec());
    let s = Rc::new(SxeObject::new(class));
    *s.base.this.borrow_mut() = Rc::downgrade(&h);
    object::set_payload(&h, s.clone());
    Some(s)
}

/// A builtin's view of one call.
struct Cx<'h> {
    host: &'h mut dyn Host,
    fname: String,
    /// `fname` is the user function running, looked up when first needed
    /// (a property handler's warnings name it).
    lazy: bool,
}

impl Cx<'_> {
    fn fname(&mut self) -> String {
        if self.lazy {
            self.fname = active_user_function(self.host);
            self.lazy = false;
        }
        self.fname.clone()
    }

    fn warn(&mut self, msg: &str) -> Result<(), Thrown> {
        let f = self.fname();
        crate::warn(self.host, &f, msg)
    }

    fn with_libxml<R>(&mut self, f: impl FnOnce() -> R) -> Result<R, Thrown> {
        let fname = self.fname();
        ffi::with_host(self.host, &fname, f)
    }

    /// [`Self::with_libxml`] for a call whose result owns memory.
    fn with_libxml_owned<R>(
        &mut self,
        f: impl FnOnce() -> R,
        free: impl FnOnce(R),
    ) -> Result<R, Thrown> {
        let fname = self.fname();
        match ffi::with_host_keep(self.host, &fname, f) {
            (r, Some(t)) => {
                free(r);
                Err(t)
            }
            (r, None) => Ok(r),
        }
    }
}

/// The user function a SimpleXML property or dimension handler runs for:
/// the frame below the prelude's accessor, or `main` at top level.
fn active_user_function(host: &dyn Host) -> String {
    const ACCESSORS: &[&[u8]] = &[
        b"__get",
        b"__set",
        b"__isset",
        b"__unset",
        b"offsetget",
        b"offsetset",
        b"offsetexists",
        b"offsetunset",
    ];
    let st = host.call_stack();
    let mut it = st.iter().skip_while(|(f, c, ..)| {
        !(c.as_deref() == Some(b"SimpleXMLElement")
            && ACCESSORS.contains(&f.to_ascii_lowercase().as_slice()))
    });
    if it.next().is_none() {
        return "main".into();
    }
    match it.next() {
        Some((f, c, ..)) if !f.is_empty() => match c {
            Some(c) => format!(
                "{}::{}",
                String::from_utf8_lossy(c),
                String::from_utf8_lossy(f)
            ),
            None => String::from_utf8_lossy(f).into_owned(),
        },
        _ => "main".into(),
    }
}

fn not_initialized() -> Thrown {
    Thrown::error("Error", "SimpleXMLElement is not properly initialized")
}

/// GET_NODE
fn get_node(sxe: &SxeObject) -> Result<*mut XmlNode, Thrown> {
    let n = sxe.base.xml_node();
    if n.is_null() {
        Err(not_initialized())
    } else {
        Ok(n)
    }
}

// ---- strings ----

/// A PHP string as C reads it: up to its first NUL.
fn cut(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]
}

/// `xmlStrEqual(p, s)` for a PHP string `s`.
fn xeq(p: *const u8, s: &[u8]) -> bool {
    !p.is_null() && unsafe { sys::cstr(p) } == cut(s)
}

fn cs(s: &[u8]) -> Vec<u8> {
    sys::cbytes(s)
}

fn xs(p: *const u8) -> Vec<u8> {
    unsafe { sys::cstr(p) }.to_vec()
}

/// `sxe_xmlNodeListGetString`
fn node_list_string(doc: *mut XmlDoc, list: *mut XmlNode) -> Vec<u8> {
    unsafe { sys::take_string(sys::xmlNodeListGetString(doc, list, 1)) }.unwrap_or_default()
}

/// match_ns
fn match_ns(node: *const XmlNode, name: Option<&[u8]>, prefix: bool) -> bool {
    unsafe {
        let ns = (*node).ns;
        if name.is_none() && (ns.is_null() || (*ns).prefix.is_null()) {
            return true;
        }
        if !ns.is_null() {
            let s = if prefix { (*ns).prefix } else { (*ns).href };
            if let Some(n) = name {
                if xeq(s, n) {
                    return true;
                }
            }
        }
        false
    }
}

/// php_sxe_get_first_node_non_destructive
fn first_node(cx: &mut Cx, sxe: &SxeObject, node: *mut XmlNode) -> Result<*mut XmlNode, Thrown> {
    if sxe.ty() != IterType::None {
        reset_iterator_no_clear(cx, sxe, false)
    } else {
        Ok(node)
    }
}

/// node_as_zval
fn node_as_zval(
    cx: &mut Cx,
    sxe: &SxeObject,
    node: *mut XmlNode,
    ty: IterType,
    name: Option<&[u8]>,
    nsprefix: Option<&[u8]>,
    isprefix: bool,
) -> Result<Value, Thrown> {
    let (v, sub) = new_object(cx.host, &sxe.class)?;
    let doc = sxe.base.document.get();
    sub.base.document.set(doc);
    if !doc.is_null() {
        unsafe { (*doc).refcount += 1 };
    }
    {
        let mut it = sub.iter.borrow_mut();
        it.ty = ty;
        it.name = name.map(<[u8]>::to_vec);
        if let Some(p) = nsprefix.filter(|p| !p.is_empty()) {
            it.nsprefix = Some(p.to_vec());
            it.isprefix = isprefix;
        }
    }
    lxn::increment_node_ptr(&sub.base, node, ptr::null());
    Ok(v)
}

/// php_sxe_object_new: an object of `class` with an empty payload.
fn new_object(host: &mut dyn Host, class: &[u8]) -> Result<(Value, Rc<SxeObject>), Thrown> {
    let Some(v) = host.instantiate(class, Vec::new()) else {
        return Err(host
            .take_pending_throw()
            .unwrap_or_else(|| Thrown::error("Error", "Cannot instantiate SimpleXMLElement")));
    };
    let h = object::object_of(&v).expect("a new object");
    let s = Rc::new(SxeObject::new(class.to_vec()));
    *s.base.this.borrow_mut() = Rc::downgrade(&h);
    object::set_payload(&h, s.clone());
    Ok((v, s))
}

/// sxe_get_element_by_offset
fn element_by_offset(
    sxe: &SxeObject,
    offset: i64,
    mut node: *mut XmlNode,
    cnt: &mut i64,
) -> *mut XmlNode {
    let it = sxe.iter.borrow();
    if it.ty == IterType::None {
        if offset == 0 {
            *cnt = 0;
            return node;
        }
        return ptr::null_mut();
    }
    let mut nodendx = 0;
    unsafe {
        while !node.is_null() && (offset < 0 || nodendx <= offset) {
            if (*node).type_ == sys::XML_ELEMENT_NODE
                && match_ns(node, it.nsprefix.as_deref(), it.isprefix)
                && (it.ty == IterType::Child
                    || (it.ty == IterType::Element
                        && xeq((*node).name, it.name.as_deref().unwrap_or_default())))
            {
                if nodendx == offset {
                    break;
                }
                nodendx += 1;
            }
            node = (*node).next;
        }
    }
    *cnt = nodendx;
    node
}

/// sxe_find_element_by_name
fn find_element_by_name(sxe: &SxeObject, mut node: *mut XmlNode, name: &[u8]) -> *mut XmlNode {
    let (ns, isprefix) = sxe.nsprefix();
    unsafe {
        while !node.is_null() {
            if (*node).type_ == sys::XML_ELEMENT_NODE
                && match_ns(node, ns.as_deref(), isprefix)
                && xeq((*node).name, name)
            {
                return node;
            }
            node = (*node).next;
        }
    }
    ptr::null_mut()
}

/// The attribute-name test of an attribute list (`test` in php).
fn attr_matches(sxe: &SxeObject, attr: *const XmlAttr, test: bool) -> bool {
    let it = sxe.iter.borrow();
    unsafe {
        (!test || xeq((*attr).name, it.name.as_deref().unwrap_or_default()))
            && match_ns(attr.cast(), it.nsprefix.as_deref(), it.isprefix)
    }
}

/// How a property or dimension is fetched (php's `BP_VAR_*`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fetch {
    R,
    W,
    Is,
}

/// A property or dimension key as php's handlers see it: an integer, or
/// a string.
enum Member {
    Long(i64),
    Name(Vec<u8>),
}

/// The key of a dimension access: integers stay integers, anything else
/// becomes a string (zval_try_get_string_func).
fn member_of(cx: &mut Cx, v: &Value) -> Result<Member, Thrown> {
    Ok(match v {
        Value::Int(i) => Member::Long(*i),
        Value::Str(s) => Member::Name(s.as_bytes().to_vec()),
        other => Member::Name(zval_get_string(cx.host, other)?),
    })
}

/// sxe_prop_dim_read: `member` is None for `$sxe[]`.
fn prop_dim_read(
    cx: &mut Cx,
    sxe: &SxeObject,
    member: Option<Member>,
    mut attribs: bool,
    fetch: Fetch,
) -> Result<Value, Thrown> {
    let ity = sxe.ty();
    match &member {
        None => {
            if ity == IterType::AttrList {
                return Err(Thrown::error("Error", "Cannot create unnamed attribute"));
            }
            attribs = false;
        }
        Some(Member::Long(_)) => {
            if ity != IterType::AttrList {
                attribs = false;
            }
        }
        Some(Member::Name(_)) => {}
    }
    let mut node = get_node(sxe)?;
    let mut attr: *mut XmlAttr = ptr::null_mut();
    let mut test = false;
    unsafe {
        if ity == IterType::AttrList {
            attribs = true;
            node = first_node(cx, sxe, node)?;
            attr = node.cast();
            test = sxe.iter.borrow().name.is_some();
        } else if ity != IterType::Child {
            node = first_node(cx, sxe, node)?;
            attr = if node.is_null() {
                ptr::null_mut()
            } else {
                (*node).properties
            };
            if member.is_none()
                && !node.is_null()
                && !(*node).parent.is_null()
                && (*(*node).parent).type_ == sys::XML_DOCUMENT_NODE
            {
                return Err(Thrown::error("Error", "Cannot create unnamed attribute"));
            }
        }
    }
    if node.is_null() {
        return Ok(Value::Null);
    }
    let (nsprefix, isprefix) = sxe.nsprefix();
    let nsp = nsprefix.as_deref();
    unsafe {
        if attribs {
            match &member {
                Some(Member::Long(l)) => {
                    let mut nodendx = 0;
                    while !attr.is_null() && nodendx <= *l {
                        if attr_matches(sxe, attr, test) {
                            if nodendx == *l {
                                return node_as_zval(
                                    cx,
                                    sxe,
                                    attr.cast(),
                                    IterType::None,
                                    None,
                                    nsp,
                                    isprefix,
                                );
                            }
                            nodendx += 1;
                        }
                        attr = (*attr).next;
                    }
                }
                Some(Member::Name(name)) => {
                    while !attr.is_null() {
                        if attr_matches(sxe, attr, test) && xeq((*attr).name, name) {
                            return node_as_zval(
                                cx,
                                sxe,
                                attr.cast(),
                                IterType::None,
                                None,
                                nsp,
                                isprefix,
                            );
                        }
                        attr = (*attr).next;
                    }
                }
                None => {}
            }
            return Ok(Value::Null);
        }
        if sxe.base.node.get().is_null() {
            lxn::increment_node_ptr(&sxe.base, node, ptr::null());
        }
        match &member {
            None | Some(Member::Long(_)) => {
                let long = match &member {
                    Some(Member::Long(l)) => Some(*l),
                    _ => None,
                };
                let mut cnt = 0;
                let mut appendable = true;
                let mynode = node;
                if ity == IterType::Child {
                    node = first_node(cx, sxe, node)?;
                }
                if ity == IterType::None {
                    if long.is_some_and(|l| l != 0) {
                        node = ptr::null_mut();
                        appendable = false;
                    }
                } else if let Some(l) = long {
                    node = element_by_offset(sxe, l, node, &mut cnt);
                } else {
                    node = ptr::null_mut();
                }
                if !node.is_null() {
                    return node_as_zval(cx, sxe, node, IterType::None, None, nsp, isprefix);
                }
                if fetch == Fetch::W {
                    if let Some(l) = long.filter(|&l| l < 0 || cnt < l) {
                        cx.warn(&format!(
                            "Cannot add element {} number {l} when only {cnt} such elements exist",
                            String::from_utf8_lossy(sys::cstr((*mynode).name))
                        ))?;
                    }
                    if appendable && long.is_none_or(|l| l >= 0) {
                        let n = sys::xmlNewTextChild(
                            (*mynode).parent,
                            (*mynode).ns,
                            (*mynode).name,
                            ptr::null(),
                        );
                        return node_as_zval(cx, sxe, n, IterType::None, None, nsp, isprefix);
                    }
                }
                Ok(Value::Null)
            }
            Some(Member::Name(name)) => {
                if fetch != Fetch::Is
                    || !find_element_by_name(sxe, (*node).children, name).is_null()
                {
                    return node_as_zval(
                        cx,
                        sxe,
                        node,
                        IterType::Element,
                        Some(name),
                        nsp,
                        isprefix,
                    );
                }
                Ok(Value::Null)
            }
        }
    }
}

/// change_node_zval
fn change_node_zval(node: *mut XmlNode, value: &[u8]) {
    let v = cs(value);
    unsafe {
        let buffer = sys::xmlEncodeEntitiesReentrant((*node).doc, v.as_ptr());
        if !buffer.is_null() {
            sys::xmlNodeSetContent(node, buffer);
            sys::xml_free(buffer);
        }
    }
}

/// sxe_unlink_node
fn unlink_node(node: *mut XmlNode) {
    unsafe {
        sys::xmlUnlinkNode(node);
        if (*node)._private.is_null() {
            lxn::node_free_resource(node);
        }
    }
}

/// The new node a write created (php's `pnewnode`).
type WriteResult = Result<*mut XmlNode, Thrown>;

/// sxe_prop_dim_write: `member` is None for `$sxe[]`, `value` None for
/// get_property_ptr_ptr's creation of a missing element.
fn prop_dim_write(
    cx: &mut Cx,
    sxe: &SxeObject,
    member: Option<&Value>,
    value: Option<&Value>,
    mut attribs: bool,
) -> WriteResult {
    let ity = sxe.ty();
    let member: Option<Member> = match member {
        None => {
            if ity == IterType::AttrList {
                return Err(Thrown::error("Error", "Cannot append to an attribute list"));
            }
            attribs = false;
            None
        }
        Some(Value::Int(i)) => {
            if ity != IterType::AttrList {
                attribs = false;
            }
            Some(Member::Long(*i))
        }
        Some(m) => {
            let name = match m {
                Value::Str(s) => s.as_bytes().to_vec(),
                other => {
                    let s = zval_get_string(cx.host, other)?;
                    php_trim(&s).to_vec()
                }
            };
            if name.is_empty() {
                return Err(Thrown::Error {
                    class: "ValueError",
                    message: format!(
                        "Cannot create {} with an empty name",
                        if attribs { "attribute" } else { "element" }
                    ),
                });
            }
            Some(Member::Name(name))
        }
    };
    let long = match &member {
        Some(Member::Long(l)) => Some(*l),
        _ => None,
    };
    let mut node = get_node(sxe)?;
    let mut mynode: *mut XmlNode;
    let mut attr: *mut XmlAttr = ptr::null_mut();
    let mut test = false;
    unsafe {
        if ity == IterType::AttrList {
            attribs = true;
            attr = first_node(cx, sxe, node)?.cast();
            test = sxe.iter.borrow().name.is_some();
        } else if ity != IterType::Child {
            mynode = node;
            node = first_node(cx, sxe, node)?;
            attr = if node.is_null() {
                ptr::null_mut()
            } else {
                (*node).properties
            };
            if member.is_none()
                && !node.is_null()
                && !(*node).parent.is_null()
                && (*(*node).parent).type_ == sys::XML_DOCUMENT_NODE
            {
                return Err(Thrown::Error {
                    class: "ValueError",
                    message: "Cannot append to an attribute list".into(),
                });
            }
            if attribs && node.is_null() && ity == IterType::Element {
                let name = cs(sxe.iter.borrow().name.as_deref().unwrap_or_default());
                node = sys::xmlNewChild(mynode, (*mynode).ns, name.as_ptr(), ptr::null());
                attr = (*node).properties;
            }
        }
    }
    mynode = node;

    let value_str: Option<Vec<u8>> = match value {
        None => None,
        Some(v) => match v {
            Value::Int(_) | Value::Bool(_) | Value::Float(_) | Value::Null | Value::Str(_) => {
                Some(conv::to_string_bytes(v))
            }
            Value::Object(h)
                if cx.host.object_class_name(v).as_deref() == Some(b"SimpleXMLElement") =>
            {
                Some(cast_string(cx, h)?)
            }
            other => {
                return Err(Thrown::type_error(format!(
                    "It's not possible to assign a complex type to {}, {} given",
                    if attribs { "attributes" } else { "properties" },
                    zval_value_name(cx.host, other)
                )));
            }
        },
    };
    let vs = value_str.as_deref().map(cs);
    let vp = vs.as_ref().map_or(ptr::null(), |v| v.as_ptr());

    let mut newnode: *mut XmlNode = ptr::null_mut();
    if node.is_null() {
        return Ok(newnode);
    }
    let (nsprefix, isprefix) = sxe.nsprefix();
    let mut counter = 0;
    let mut is_attr = false;
    let mut nodendx = 0;
    let mut cnt = 0;
    unsafe {
        lxn::invalidate_node_list_cache_from_doc((*node).doc);
        if attribs {
            match &member {
                Some(Member::Long(l)) => {
                    while !attr.is_null() && nodendx <= *l {
                        if attr_matches(sxe, attr, test) {
                            if nodendx == *l {
                                is_attr = true;
                                counter += 1;
                                break;
                            }
                            nodendx += 1;
                        }
                        attr = (*attr).next;
                    }
                }
                Some(Member::Name(name)) => {
                    while !attr.is_null() {
                        if attr_matches(sxe, attr, test) && xeq((*attr).name, name) {
                            is_attr = true;
                            counter += 1;
                            break;
                        }
                        attr = (*attr).next;
                    }
                }
                None => {}
            }
        } else if member.is_none() || long.is_some() {
            if (*node).type_ == sys::XML_ATTRIBUTE_NODE {
                return Err(Thrown::error("Error", "Cannot create duplicate attribute"));
            }
            if ity == IterType::None {
                if let Some(l) = long.filter(|&l| l != 0) {
                    cx.warn(&format!(
                        "Cannot add element {} number {l} when only 0 such elements exist",
                        String::from_utf8_lossy(sys::cstr((*mynode).name))
                    ))?;
                    return Ok(newnode);
                }
                newnode = node;
                counter += 1;
            } else if let Some(l) = long {
                newnode = element_by_offset(sxe, l, node, &mut cnt);
                if !newnode.is_null() {
                    counter += 1;
                }
            }
        } else if let Some(Member::Name(name)) = &member {
            node = (*node).children;
            while !node.is_null() {
                if (*node).type_ != sys::XML_TEXT_NODE
                    && xeq((*node).name, name)
                    && match_ns(node, nsprefix.as_deref(), isprefix)
                {
                    newnode = node;
                    counter += 1;
                }
                node = (*node).next;
            }
        }

        if counter == 1 {
            if is_attr {
                newnode = attr.cast();
            }
            if let Some(v) = &value_str {
                loop {
                    let child = (*newnode).children;
                    if child.is_null() {
                        break;
                    }
                    unlink_node(child);
                }
                change_node_zval(newnode, v);
            }
        } else if counter > 1 {
            cx.warn("Cannot assign to an array of nodes (duplicate subnodes or attr detected)")?;
        } else if !attribs {
            if node.is_null() {
                if let Some(Member::Name(name)) = &member {
                    let n = cs(name);
                    newnode = sys::xmlNewTextChild(mynode, ptr::null_mut(), n.as_ptr(), vp);
                } else {
                    newnode =
                        sys::xmlNewTextChild((*mynode).parent, (*mynode).ns, (*mynode).name, vp);
                }
            } else if member.is_none() || long.is_some() {
                if let Some(l) = long.filter(|&l| l < 0 || cnt < l) {
                    cx.warn(&format!(
                        "Cannot add element {} number {l} when only {cnt} such elements exist",
                        String::from_utf8_lossy(sys::cstr((*mynode).name))
                    ))?;
                }
                if !long.is_some_and(|l| l < 0) {
                    newnode =
                        sys::xmlNewTextChild((*mynode).parent, (*mynode).ns, (*mynode).name, vp);
                }
            }
        } else if let Some(l) = long {
            cx.warn(&format!(
                "Cannot change attribute number {l} when only {nodendx} attributes exist"
            ))?;
        } else if let Some(Member::Name(name)) = &member {
            let n = cs(name);
            newnode = sys::xmlNewProp(node, n.as_ptr(), vp).cast();
        }
    }
    Ok(newnode)
}

/// php_trim(str, NULL, 0, 3): the default whitespace off both ends.
fn php_trim(s: &[u8]) -> &[u8] {
    let ws = |c: &u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\0');
    let start = s.iter().position(|c| !ws(c)).unwrap_or(s.len());
    let end = s.iter().rposition(|c| !ws(c)).map_or(start, |e| e + 1);
    &s[start..end]
}

/// sxe_property_get_adr: a write below `$sxe->name` (`$sxe->a->b = …`,
/// `$sxe->a[] = …`) creates the element when it is missing.
fn property_get_adr(cx: &mut Cx, sxe: &SxeObject, name: &[u8]) -> Result<Value, Thrown> {
    let node = get_node(sxe)?;
    if !element_by_name(cx, sxe, node, name)?.is_null() {
        return prop_dim_read(cx, sxe, Some(Member::Name(name.to_vec())), false, Fetch::W);
    }
    let member = Value::str(name.to_vec());
    let node = prop_dim_write(cx, sxe, Some(&member), None, false)?;
    let (nsprefix, isprefix) = sxe.nsprefix();
    node_as_zval(
        cx,
        sxe,
        node,
        IterType::None,
        None,
        nsprefix.as_deref(),
        isprefix,
    )
}

/// sxe_get_element_by_name
fn element_by_name(
    cx: &mut Cx,
    sxe: &SxeObject,
    mut node: *mut XmlNode,
    name: &[u8],
) -> Result<*mut XmlNode, Thrown> {
    let mut orgnode = node;
    let ity = sxe.ty();
    if ity != IterType::AttrList {
        if ity == IterType::None {
            sxe.iter.borrow_mut().ty = IterType::Child;
        }
        let r = first_node(cx, sxe, node);
        sxe.iter.borrow_mut().ty = ity;
        node = r?;
    }
    unsafe {
        if ity == IterType::Element {
            let iname = sxe.iter.borrow().name.clone().unwrap_or_default();
            orgnode = find_element_by_name(sxe, node, &iname);
            if orgnode.is_null() {
                return Ok(ptr::null_mut());
            }
            node = (*orgnode).children;
        }
        let (ns, isprefix) = sxe.nsprefix();
        while !node.is_null() {
            if (*node).type_ == sys::XML_ELEMENT_NODE
                && match_ns(node, ns.as_deref(), isprefix)
                && xeq((*node).name, name)
            {
                return Ok(orgnode);
            }
            node = (*node).next;
        }
    }
    Ok(ptr::null_mut())
}

/// sxe_prop_dim_exists: `check_empty` 0 for isset, 1 for empty.
fn prop_dim_exists(
    cx: &mut Cx,
    sxe: &SxeObject,
    member: &Value,
    check_empty: i32,
    mut attribs: bool,
) -> Result<bool, Thrown> {
    let member = member_of(cx, member)?;
    let ity = sxe.ty();
    let mut node = get_node(sxe)?;
    if let Member::Long(_) = member {
        if ity != IterType::AttrList {
            attribs = false;
            if ity == IterType::Child {
                node = first_node(cx, sxe, node)?;
            }
        }
    }
    let mut attr: *mut XmlAttr = ptr::null_mut();
    let mut test = false;
    unsafe {
        if ity == IterType::AttrList {
            attribs = true;
            node = first_node(cx, sxe, node)?;
            attr = node.cast();
            test = sxe.iter.borrow().name.is_some();
        } else if ity != IterType::Child {
            node = first_node(cx, sxe, node)?;
            attr = if node.is_null() {
                ptr::null_mut()
            } else {
                (*node).properties
            };
        }
        if node.is_null() {
            return Ok(false);
        }
        let empty_text = |c: *const XmlNode| {
            (*c).content.is_null() || *(*c).content == 0 || xeq((*c).content, b"0")
        };
        if attribs {
            let mut exists = false;
            match &member {
                Member::Long(l) => {
                    let mut nodendx = 0;
                    while !attr.is_null() && nodendx <= *l {
                        if attr_matches(sxe, attr, test) {
                            if nodendx == *l {
                                exists = true;
                                break;
                            }
                            nodendx += 1;
                        }
                        attr = (*attr).next;
                    }
                }
                Member::Name(name) => {
                    while !attr.is_null() {
                        if attr_matches(sxe, attr, test) && xeq((*attr).name, name) {
                            exists = true;
                            break;
                        }
                        attr = (*attr).next;
                    }
                }
            }
            if exists
                && check_empty == 1
                && ((*attr).children.is_null() || empty_text((*attr).children))
            {
                exists = false;
            }
            return Ok(exists);
        }
        let found = match &member {
            Member::Long(l) => {
                if ity == IterType::Child {
                    node = first_node(cx, sxe, node)?;
                }
                let mut cnt = 0;
                element_by_offset(sxe, *l, node, &mut cnt)
            }
            Member::Name(name) => find_element_by_name(sxe, (*node).children, name),
        };
        if found.is_null() {
            return Ok(false);
        }
        let c = (*found).children;
        if check_empty == 1
            && (c.is_null()
                || ((*c).type_ == sys::XML_TEXT_NODE && (*c).next.is_null() && empty_text(c)))
        {
            return Ok(false);
        }
        Ok(true)
    }
}

/// sxe_prop_dim_delete
fn prop_dim_delete(
    cx: &mut Cx,
    sxe: &SxeObject,
    member: &Value,
    mut attribs: bool,
) -> Result<(), Thrown> {
    let member = member_of(cx, member)?;
    let ity = sxe.ty();
    let mut node = get_node(sxe)?;
    if let Member::Long(_) = member {
        if ity != IterType::AttrList {
            attribs = false;
            if ity == IterType::Child {
                node = first_node(cx, sxe, node)?;
            }
        }
    }
    let mut attr: *mut XmlAttr = ptr::null_mut();
    let mut test = false;
    unsafe {
        if ity == IterType::AttrList {
            attribs = true;
            node = first_node(cx, sxe, node)?;
            attr = node.cast();
            test = sxe.iter.borrow().name.is_some();
        } else if ity != IterType::Child {
            node = first_node(cx, sxe, node)?;
            attr = if node.is_null() {
                ptr::null_mut()
            } else {
                (*node).properties
            };
        }
        if node.is_null() {
            return Ok(());
        }
        lxn::invalidate_node_list_cache_from_doc((*node).doc);
        if attribs {
            match &member {
                Member::Long(l) => {
                    let mut nodendx = 0;
                    while !attr.is_null() && nodendx <= *l {
                        if attr_matches(sxe, attr, test) {
                            if nodendx == *l {
                                unlink_node(attr.cast());
                                break;
                            }
                            nodendx += 1;
                        }
                        attr = (*attr).next;
                    }
                }
                Member::Name(name) => {
                    while !attr.is_null() {
                        let anext = (*attr).next;
                        if attr_matches(sxe, attr, test) && xeq((*attr).name, name) {
                            unlink_node(attr.cast());
                            break;
                        }
                        attr = anext;
                    }
                }
            }
            return Ok(());
        }
        match &member {
            Member::Long(l) => {
                if ity == IterType::Child {
                    node = first_node(cx, sxe, node)?;
                }
                let mut cnt = 0;
                let n = element_by_offset(sxe, *l, node, &mut cnt);
                if !n.is_null() {
                    unlink_node(n);
                }
            }
            Member::Name(name) => {
                let (ns, isprefix) = sxe.nsprefix();
                node = (*node).children;
                while !node.is_null() {
                    let nnext = (*node).next;
                    if (*node).type_ != sys::XML_TEXT_NODE
                        && xeq((*node).name, name)
                        && match_ns(node, ns.as_deref(), isprefix)
                    {
                        unlink_node(node);
                    }
                    node = nnext;
                }
            }
        }
    }
    Ok(())
}

// ---- property tables ----

/// sxe_properties_add
fn properties_add(rv: &mut PhpArray, name: &[u8], value: Value) {
    let key = ArrayKey::Str(PhpStr::from_bytes(name.to_vec()));
    match rv.get_mut(&key) {
        Some(Value::Array(a)) => {
            a.push(value);
        }
        Some(existing) => {
            let old = std::mem::replace(existing, Value::Null);
            let mut pair = PhpArray::new();
            pair.push(old);
            pair.push(value);
            *existing = Value::Array(pair);
        }
        None => {
            rv.insert(key, value);
        }
    }
}

/// get_base_node_value
fn base_node_value(
    cx: &mut Cx,
    sxe: &SxeObject,
    node: *mut XmlNode,
    nsprefix: Option<&[u8]>,
    isprefix: bool,
) -> Result<Value, Thrown> {
    unsafe {
        let c = (*node).children;
        if !c.is_null() && (*c).type_ == sys::XML_TEXT_NODE && sys::xmlIsBlankNode(c) == 0 {
            return Ok(Value::str(node_list_string((*node).doc, c)));
        }
    }
    node_as_zval(cx, sxe, node, IterType::None, None, nsprefix, isprefix)
}

/// Whether a node of a property table's child walk shows (php's
/// `SKIP_TEXT` / text branches): `Text` adds a numeric entry, `Named` a
/// named one, `Skip` nothing.
enum Step {
    Text,
    Named,
    Skip,
}

fn classify(
    node: *mut XmlNode,
    nsprefix: Option<&[u8]>,
    isprefix: bool,
    debug_blank: bool,
) -> Step {
    unsafe {
        let has_kin =
            !(*node).children.is_null() || !(*node).prev.is_null() || !(*node).next.is_null();
        let blank = debug_blank && sys::xmlIsBlankNode(node) != 0;
        if has_kin || blank {
            if (*node).type_ == sys::XML_TEXT_NODE {
                return Step::Skip;
            }
        } else if (*node).type_ == sys::XML_TEXT_NODE {
            if *(*node).content != 0 {
                return Step::Text;
            }
            return Step::Skip;
        }
        if (*node).type_ == sys::XML_ELEMENT_NODE && !match_ns(node, nsprefix, isprefix) {
            return Step::Skip;
        }
        if (*node).name.is_null() {
            return Step::Skip;
        }
        Step::Named
    }
}

/// sxe_get_prop_hash
fn prop_hash(cx: &mut Cx, sxe: &SxeObject, is_debug: bool) -> Result<PhpArray, Thrown> {
    let mut rv = PhpArray::new();
    let Ok(mut node) = get_node(sxe) else {
        return Err(not_initialized());
    };
    let ity = sxe.ty();
    let (nsprefix, isprefix) = sxe.nsprefix();
    let nsp = nsprefix.as_deref();
    unsafe {
        if is_debug || ity != IterType::Child {
            if ity == IterType::Element {
                node = first_node(cx, sxe, node)?;
            }
            if !node.is_null() && (*node).type_ != sys::XML_ENTITY_DECL {
                let test = sxe.iter.borrow().name.is_some() && ity == IterType::AttrList;
                let mut attr = (*node).properties;
                let mut zattr: Option<PhpArray> = None;
                let doc = sxe.document_ptr();
                while !attr.is_null() {
                    if attr_matches(sxe, attr, test) {
                        let s = node_list_string(doc, (*attr).children);
                        zattr
                            .get_or_insert_with(PhpArray::new)
                            .insert(ArrayKey::from_bytes(sys::cstr((*attr).name)), Value::str(s));
                    }
                    attr = (*attr).next;
                }
                if let Some(z) = zattr {
                    properties_add(&mut rv, b"@attributes", Value::Array(z));
                }
            }
        }
        node = get_node(sxe)?;
        node = first_node(cx, sxe, node)?;
        if node.is_null() || ity == IterType::AttrList {
            return Ok(rv);
        }
        let mut use_iter = false;
        if (*node).type_ == sys::XML_ATTRIBUTE_NODE {
            rv.push(Value::str(node_list_string((*node).doc, (*node).children)));
            return Ok(rv);
        }
        if ity != IterType::Child {
            let n = node;
            if ity == IterType::None
                || (*n).children.is_null()
                || (*n).parent.is_null()
                || (*n).next.is_null()
                || !(*(*n).children).next.is_null()
                || !(*(*n).children).children.is_null()
                || (*(*n).parent).children == (*(*n).parent).last
            {
                node = (*n).children;
            } else {
                node = reset_iterator_no_clear(cx, sxe, false)?;
                use_iter = true;
            }
        }
        while !node.is_null() {
            match classify(node, nsp, isprefix, true) {
                Step::Text => {
                    rv.push(Value::str(node_list_string((*node).doc, node)));
                }
                Step::Named => {
                    let value = base_node_value(cx, sxe, node, nsp, isprefix)?;
                    if use_iter {
                        rv.push(value);
                    } else {
                        properties_add(&mut rv, sys::cstr((*node).name), value);
                    }
                }
                Step::Skip => {}
            }
            if (*node).type_ == sys::XML_ENTITY_DECL {
                break;
            }
            node = if use_iter {
                iterator_fetch(cx, sxe, (*node).next, false)?
            } else {
                (*node).next
            };
        }
    }
    Ok(rv)
}

/// sxe_prop_is_empty
fn prop_is_empty(cx: &mut Cx, sxe: &SxeObject) -> Result<bool, Thrown> {
    let mut node = sxe.base.xml_node();
    if node.is_null() {
        return Ok(true);
    }
    let ity = sxe.ty();
    let (nsprefix, isprefix) = sxe.nsprefix();
    let nsp = nsprefix.as_deref();
    unsafe {
        if ity == IterType::Element {
            node = first_node(cx, sxe, node)?;
        }
        if !node.is_null() && (*node).type_ != sys::XML_ENTITY_DECL {
            let test = sxe.iter.borrow().name.is_some() && ity == IterType::AttrList;
            let mut attr = (*node).properties;
            while !attr.is_null() {
                if attr_matches(sxe, attr, test) {
                    return Ok(false);
                }
                attr = (*attr).next;
            }
        }
        node = get_node(sxe)?;
        node = first_node(cx, sxe, node)?;
        if node.is_null() || ity == IterType::AttrList {
            return Ok(true);
        }
        let mut use_iter = false;
        if (*node).type_ == sys::XML_ATTRIBUTE_NODE {
            return Ok(false);
        }
        if ity != IterType::Child {
            let n = node;
            if ity == IterType::None
                || (*n).children.is_null()
                || (*n).parent.is_null()
                || !(*(*n).children).next.is_null()
                || !(*(*n).children).children.is_null()
                || (*(*n).parent).children == (*(*n).parent).last
            {
                node = (*n).children;
            } else {
                node = reset_iterator_no_clear(cx, sxe, false)?;
                use_iter = true;
            }
        }
        while !node.is_null() {
            match classify(node, nsp, isprefix, false) {
                Step::Text | Step::Named => return Ok(false),
                Step::Skip => {}
            }
            node = if use_iter {
                iterator_fetch(cx, sxe, (*node).next, false)?
            } else {
                (*node).next
            };
        }
    }
    Ok(true)
}

// ---- casts and comparison ----

/// The text php's cast handler converts (sxe_object_cast_ex's
/// `contents`).
fn cast_contents(cx: &mut Cx, sxe: &SxeObject) -> Result<Option<Vec<u8>>, Thrown> {
    unsafe {
        if sxe.ty() != IterType::None {
            let node = reset_iterator_no_clear(cx, sxe, false)?;
            if !node.is_null() {
                return Ok(sys::take_string(sys::xmlNodeListGetString(
                    sxe.document_ptr(),
                    (*node).children,
                    1,
                )));
            }
            return Ok(None);
        }
        if sxe.base.node.get().is_null() {
            let doc = sxe.document_ptr();
            if !doc.is_null() {
                lxn::increment_node_ptr(&sxe.base, sys::xmlDocGetRootElement(doc), ptr::null());
            }
        }
        let n = sxe.base.xml_node();
        if !n.is_null() {
            if !(*n).children.is_null() {
                return Ok(sys::take_string(sys::xmlNodeListGetString(
                    sxe.document_ptr(),
                    (*n).children,
                    1,
                )));
            }
            if (*n).type_ == sys::XML_COMMENT_NODE || (*n).type_ == sys::XML_PI_NODE {
                return Ok(Some(xs((*n).content)));
            }
        }
    }
    Ok(None)
}

/// sxe_object_cast_ex
fn cast_ex(cx: &mut Cx, sxe: &SxeObject, target: CastTarget) -> Result<Option<Value>, Thrown> {
    if target == CastTarget::Bool {
        if sxe.ty() != IterType::None && !reset_iterator_no_clear(cx, sxe, false)?.is_null() {
            return Ok(Some(Value::Bool(true)));
        }
        return Ok(Some(Value::Bool(!prop_is_empty(cx, sxe)?)));
    }
    let contents = cast_contents(cx, sxe)?;
    let v = match contents {
        Some(s) => Value::str(s),
        None => Value::Null,
    };
    Ok(match target {
        CastTarget::Str => Some(Value::str(conv::to_string_bytes(&v))),
        CastTarget::Int => Some(Value::Int(conv::to_int(&v))),
        CastTarget::Float => Some(Value::Float(conv::to_float(&v))),
        CastTarget::Number => Some(conv::to_number(&v).0.to_value()),
        _ => None,
    })
}

/// The string value of a SimpleXMLElement, without a subclass's
/// `__toString()`.
fn cast_string(cx: &mut Cx, h: &ObjectHandle) -> Result<Vec<u8>, Thrown> {
    let v = Value::Object(h.clone());
    let Some(sxe) = sxe_of(cx.host, &v) else {
        return Ok(Vec::new());
    };
    Ok(match cast_ex(cx, &sxe, CastTarget::Str)? {
        Some(Value::Str(s)) => s.as_bytes().to_vec(),
        _ => Vec::new(),
    })
}

/// sxe_object_cast: a subclass's `__toString()` first for strings.
fn sxe_object_cast(
    host: &mut dyn Host,
    h: &ObjectHandle,
    target: CastTarget,
) -> Result<Option<Value>, Thrown> {
    if target == CastTarget::Str && user_to_string(host, h) {
        if let Some(s) = host.std_cast_object(h, target)? {
            return Ok(Some(s));
        }
    }
    let v = Value::Object(h.clone());
    let Some(sxe) = sxe_of(host, &v) else {
        return Ok(None);
    };
    let mut cx = Cx {
        host,
        fname: String::new(),
        lazy: true,
    };
    cast_ex(&mut cx, &sxe, target)
}

/// Whether a user class overrides `__toString()`.
fn user_to_string(host: &dyn Host, h: &ObjectHandle) -> bool {
    let v = Value::Object(h.clone());
    !matches!(
        host.object_class_name(&v).as_deref(),
        Some(b"SimpleXMLElement" | b"SimpleXMLIterator")
    ) && host.class_has_method(h.borrow().class, b"__tostring")
}

/// sxe_objects_compare
fn sxe_objects_compare(host: &mut dyn Host, a: &Value, b: &Value) -> Result<Option<i32>, Thrown> {
    if !object::has_handlers(host, a, &SXE_HANDLERS)
        || !object::has_handlers(host, b, &SXE_HANDLERS)
    {
        return Ok(None);
    }
    let (Some(x), Some(y)) = (sxe_of(host, a), sxe_of(host, b)) else {
        return Ok(Some(1));
    };
    let (nx, ny) = (x.base.node.get(), y.base.node.get());
    if !nx.is_null() && !ny.is_null() {
        return Ok(Some(if nx == ny { 0 } else { 1 }));
    }
    if nx.is_null() && ny.is_null() {
        let (dx, dy) = (x.base.document.get(), y.base.document.get());
        if !dx.is_null() && !dy.is_null() && unsafe { (*dx).ptr == (*dy).ptr } {
            return Ok(Some(0));
        }
    }
    Ok(Some(1))
}

/// sxe_get_properties: what `(array)`, `get_object_vars`, `var_export`
/// and `json_encode` see. The debug view is `__debugInfo()`.
fn sxe_get_properties_for(
    host: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    match purpose {
        PropPurpose::Debug | PropPurpose::Serialize => return Ok(None),
        PropPurpose::ArrayCast
        | PropPurpose::GetObjectVars
        | PropPurpose::VarExport
        | PropPurpose::Json => {}
    }
    let v = Value::Object(h.clone());
    let Some(sxe) = sxe_of(host, &v) else {
        return Ok(None);
    };
    let mut cx = Cx {
        host,
        fname: String::new(),
        lazy: true,
    };
    if sxe.base.xml_node().is_null() {
        return Ok(Some(PhpArray::new()));
    }
    let a = prop_hash(&mut cx, &sxe, false)?;
    Ok(Some(if purpose == PropPurpose::ArrayCast {
        let mut out = PhpArray::new();
        for (k, v) in a.iter() {
            let key = match k {
                ArrayKey::Str(s) => ArrayKey::from_bytes(s.as_bytes()),
                k => k.clone(),
            };
            out.insert(key, v.clone());
        }
        out
    } else {
        a
    }))
}

/// sxe_object_clone
fn sxe_clone(host: &mut dyn Host, src: &ObjectHandle, dst: &ObjectHandle) -> Result<(), Thrown> {
    let sv = Value::Object(src.clone());
    let dv = Value::Object(dst.clone());
    let Some(sxe) = sxe_of(host, &sv) else {
        return Ok(());
    };
    let class = host
        .object_class_name(&dv)
        .unwrap_or_else(|| sxe.class.clone());
    let clone = Rc::new(SxeObject::new(class));
    *clone.base.this.borrow_mut() = Rc::downgrade(dst);
    let n = sxe.base.xml_node();
    let is_root = unsafe {
        !n.is_null()
            && !(*n).parent.is_null()
            && matches!(
                (*(*n).parent).type_,
                sys::XML_DOCUMENT_NODE | sys::XML_HTML_DOCUMENT_NODE
            )
    };
    let mut docp: *mut XmlDoc = ptr::null_mut();
    unsafe {
        if is_root {
            docp = sys::xmlCopyDoc(sxe.document_ptr(), 1);
            lxn::increment_doc_ref(&clone.base, docp);
        } else {
            let d = sxe.base.document.get();
            clone.base.document.set(d);
            if !d.is_null() {
                (*d).refcount += 1;
                docp = (*d).ptr;
            }
        }
    }
    {
        let it = sxe.iter.borrow();
        let mut c = clone.iter.borrow_mut();
        c.isprefix = it.isprefix;
        c.name = it.name.clone();
        c.nsprefix = it.nsprefix.clone();
        c.ty = it.ty;
    }
    let mut nodep = ptr::null_mut();
    if !sxe.base.node.get().is_null() {
        nodep = if is_root {
            unsafe { sys::xmlDocGetRootElement(docp) }
        } else {
            unsafe { sys::xmlDocCopyNode(n, docp, 1) }
        };
    }
    lxn::increment_node_ptr(&clone.base, nodep, ptr::null());
    object::set_payload(dst, clone);
    Ok(())
}

static SXE_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(sxe_get_properties_for),
    clone_obj: CloneObj::Custom(sxe_clone),
    compare: Some(sxe_objects_compare),
    cast_object: Some(sxe_object_cast),
    ..ObjectHandlers::STD
};

pub(crate) static CLASSES: &[ClassHandlers] = &[ClassHandlers {
    class: "SimpleXMLElement",
    handlers: &SXE_HANDLERS,
    create_object: true,
}];

// ---- iteration ----

/// php_sxe_iterator_fetch
fn iterator_fetch(
    cx: &mut Cx,
    sxe: &SxeObject,
    mut node: *mut XmlNode,
    use_data: bool,
) -> Result<*mut XmlNode, Thrown> {
    let (ty, name, prefix, isprefix) = {
        let it = sxe.iter.borrow();
        (it.ty, it.name.clone(), it.nsprefix.clone(), it.isprefix)
    };
    let p = prefix.as_deref();
    unsafe {
        let want = match (ty, &name) {
            (IterType::AttrList, _) => sys::XML_ATTRIBUTE_NODE,
            _ => sys::XML_ELEMENT_NODE,
        };
        let by_name = match ty {
            IterType::AttrList => name.as_deref(),
            IterType::Element => name.as_deref(),
            _ => None,
        };
        while !node.is_null() {
            if (*node).type_ == want
                && by_name.is_none_or(|n| xeq((*node).name, n))
                && match_ns(node, p, isprefix)
            {
                break;
            }
            node = (*node).next;
        }
    }
    if !node.is_null() && use_data {
        let v = node_as_zval(cx, sxe, node, IterType::None, None, p, isprefix)?;
        sxe.iter.borrow_mut().data = Some(v);
    }
    Ok(node)
}

/// php_sxe_reset_iterator_no_clear_iter_data
fn reset_iterator_no_clear(
    cx: &mut Cx,
    sxe: &SxeObject,
    use_data: bool,
) -> Result<*mut XmlNode, Thrown> {
    let node = get_node(sxe)?;
    let start = unsafe {
        match sxe.ty() {
            IterType::AttrList => (*node).properties.cast(),
            _ => (*node).children,
        }
    };
    iterator_fetch(cx, sxe, start, use_data)
}

/// php_sxe_reset_iterator
fn reset_iterator(cx: &mut Cx, sxe: &SxeObject) -> Result<*mut XmlNode, Thrown> {
    let old = sxe.iter.borrow_mut().data.take();
    drop(old);
    reset_iterator_no_clear(cx, sxe, true)
}

/// php_sxe_move_forward_iterator
fn move_forward(cx: &mut Cx, sxe: &SxeObject) -> Result<(), Thrown> {
    let data = sxe.iter.borrow_mut().data.take();
    let mut node = ptr::null_mut();
    if let Some(d) = &data {
        if let Some(intern) = sxe_payload(d) {
            node = get_node(&intern)?;
        }
    }
    // The current object may hold the last reference to a detached node.
    let next = if node.is_null() {
        None
    } else {
        Some(unsafe { (*node).next })
    };
    drop(data);
    if let Some(next) = next {
        iterator_fetch(cx, sxe, next, true)?;
    }
    Ok(())
}

// ---- loading ----

/// xmlReadMemory / xmlReadFile under PHP_LIBXML_SANITIZE_GLOBALS.
fn read_doc(cx: &mut Cx, data: &[u8], is_file: bool, options: i64) -> Result<*mut XmlDoc, Thrown> {
    let path = if is_file { cs(data) } else { Vec::new() };
    cx.with_libxml_owned(
        || unsafe {
            let mut saved = [0 as c_int; 6];
            sys::rhp_xml_sanitize_globals(saved.as_mut_ptr());
            let d = if is_file {
                sys::xmlReadFile(path.as_ptr().cast(), ptr::null(), options as c_int)
            } else {
                sys::xmlReadMemory(
                    sys::mem_ptr(data),
                    data.len() as c_int,
                    ptr::null(),
                    ptr::null(),
                    options as c_int,
                )
            };
            sys::rhp_xml_restore_globals(saved.as_ptr());
            d
        },
        |d| unsafe {
            if !d.is_null() {
                sys::xmlFreeDoc(d)
            }
        },
    )
}

/// The `?string $class_name` argument, parsed as zpp's `C!`: null is
/// SimpleXMLElement, anything else must name (autoloading) SimpleXMLElement
/// or a subclass of it. An autoloader's exception propagates.
fn class_arg(host: &mut dyn Host, fname: &str, n: usize, v: Value) -> Result<Vec<u8>, Thrown> {
    let name = match v {
        Value::Null => return Ok(b"SimpleXMLElement".to_vec()),
        v => conv::to_string_bytes(&v),
    };
    if let Some(id) = host.resolve_class_autoload(&name)? {
        let canonical = host.class_name(id).unwrap_or_else(|| name.clone());
        if host.reflect_is_a(&canonical, b"SimpleXMLElement") {
            return Ok(canonical);
        }
    }
    Err(Thrown::type_error(format!(
        "{fname}(): Argument #{n} ($class_name) must be a class name derived from SimpleXMLElement or null, {} given",
        String::from_utf8_lossy(&name)
    )))
}

fn value_error(msg: String) -> Thrown {
    Thrown::Error {
        class: "ValueError",
        message: msg,
    }
}

/// A new SimpleXMLElement of `class` for the root of `docp`.
fn wrap_doc(
    host: &mut dyn Host,
    class: &[u8],
    docp: *mut XmlDoc,
    ns: &[u8],
    isprefix: bool,
) -> Result<Value, Thrown> {
    let (v, sxe) = match new_object(host, class) {
        Ok(x) => x,
        Err(t) => {
            unsafe { sys::xmlFreeDoc(docp) };
            return Err(t);
        }
    };
    {
        let mut it = sxe.iter.borrow_mut();
        it.nsprefix = (!ns.is_empty()).then(|| ns.to_vec());
        it.isprefix = isprefix;
    }
    lxn::increment_doc_ref(&sxe.base, docp);
    lxn::increment_node_ptr(
        &sxe.base,
        unsafe { sys::xmlDocGetRootElement(docp) },
        ptr::null(),
    );
    Ok(v)
}

fn load_common(host: &mut dyn Host, args: &[Value], is_file: bool) -> Result<Value, Thrown> {
    let fname = if is_file {
        "simplexml_load_file"
    } else {
        "simplexml_load_string"
    };
    let src = lx::str_arg(args, 0);
    let src = src.as_bytes();
    if is_file && src.contains(&0) {
        return Err(value_error(format!(
            "{fname}(): Argument #1 ($filename) must not contain any null bytes"
        )));
    }
    let class = class_arg(host, fname, 2, arg(args, 1))?;
    let options = conv::to_int(&arg(args, 2));
    let ns = bytes(args, 3);
    let isprefix = conv::to_bool(&arg(args, 4));
    if !is_file && src.len() as i64 > INT_MAX {
        return Err(value_error(format!(
            "{fname}(): Argument #1 ($data) is too long"
        )));
    }
    if !is_file && ns.len() as i64 > INT_MAX {
        return Err(value_error(format!(
            "{fname}(): Argument #4 ($namespace_or_prefix) is too long"
        )));
    }
    if options > INT_MAX || options < i32::MIN as i64 {
        return Err(value_error(format!(
            "{fname}(): Argument #3 ($options) is too large"
        )));
    }
    let mut cx = Cx {
        host,
        fname: fname.into(),
        lazy: false,
    };
    let docp = read_doc(&mut cx, src, is_file, options)?;
    if docp.is_null() {
        return Ok(Value::Bool(false));
    }
    wrap_doc(host, &class, docp, &ns, isprefix)
}

fn fn_load_string(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    load_common(host, args, false)
}

fn fn_load_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    load_common(host, args, true)
}

/// simplexml_export_node: the node a SimpleXML object stands for (for
/// php_libxml_import_node), and its document reference.
pub(crate) fn export_node(
    host: &mut dyn Host,
    v: &Value,
) -> Result<Option<(*mut XmlNode, *mut lxn::RefObj)>, Thrown> {
    let Some(sxe) = sxe_payload(v) else {
        return Ok(None);
    };
    let mut cx = Cx {
        host,
        fname: String::new(),
        lazy: true,
    };
    let node = get_node(&sxe)?;
    let node = first_node(&mut cx, &sxe, node)?;
    Ok((!node.is_null()).then(|| (node, sxe.base.document.get())))
}

/// The document reference of a SimpleXML object (NULL for any other value).
pub(crate) fn document_of(v: &Value) -> *mut lxn::RefObj {
    sxe_payload(v).map_or(ptr::null_mut(), |s| s.base.document.get())
}

fn fn_import_dom(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = arg(args, 0);
    let class = class_arg(host, "simplexml_import_dom", 2, arg(args, 1))?;
    let Some((mut nodep, document)) = lx::import_node(host, &v)? else {
        return Err(Thrown::type_error(
            "simplexml_import_dom(): Argument #1 ($node) must be a valid XML node",
        ));
    };
    unsafe {
        if (*nodep).doc.is_null() {
            crate::warn(
                host,
                "simplexml_import_dom",
                "Imported Node must have associated Document",
            )?;
            return Ok(Value::Null);
        }
        if matches!(
            (*nodep).type_,
            sys::XML_DOCUMENT_NODE | sys::XML_HTML_DOCUMENT_NODE
        ) {
            nodep = sys::xmlDocGetRootElement(nodep.cast());
        }
        if nodep.is_null() || (*nodep).type_ != sys::XML_ELEMENT_NODE {
            crate::warn(host, "simplexml_import_dom", "Invalid Nodetype to import")?;
            return Ok(Value::Null);
        }
        let (v, sxe) = new_object(host, &class)?;
        sxe.base.document.set(document);
        lxn::increment_doc_ref(&sxe.base, (*nodep).doc);
        lxn::increment_node_ptr(&sxe.base, nodep, ptr::null());
        Ok(v)
    }
}

// ---- methods ----

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(deref).unwrap_or(Value::Null)
}

fn bytes(args: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&arg(args, i))
}

fn opt_bytes(args: &[Value], i: usize) -> Option<Vec<u8>> {
    match arg(args, i) {
        Value::Null => None,
        v => Some(conv::to_string_bytes(&v)),
    }
}

type Method = fn(&mut Cx, &Rc<SxeObject>, &Value, &[Value]) -> Result<Value, Thrown>;

fn method(name: &[u8]) -> Option<Method> {
    Some(match name {
        b"SimpleXMLElement::__construct" => m_construct,
        b"SimpleXMLElement::xpath" => m_xpath,
        b"SimpleXMLElement::registerXPathNamespace" => m_register_xpath_namespace,
        b"SimpleXMLElement::asXML" | b"SimpleXMLElement::saveXML" => m_as_xml,
        b"SimpleXMLElement::getNamespaces" => m_get_namespaces,
        b"SimpleXMLElement::getDocNamespaces" => m_get_doc_namespaces,
        b"SimpleXMLElement::children" => m_children,
        b"SimpleXMLElement::attributes" => m_attributes,
        b"SimpleXMLElement::addChild" => m_add_child,
        b"SimpleXMLElement::addAttribute" => m_add_attribute,
        b"SimpleXMLElement::getName" => m_get_name,
        b"SimpleXMLElement::__toString" => m_to_string,
        b"SimpleXMLElement::__debugInfo" => m_debug_info,
        b"SimpleXMLElement::count" => m_count,
        b"SimpleXMLElement::rewind" => m_rewind,
        b"SimpleXMLElement::valid" => m_valid,
        b"SimpleXMLElement::current" => m_current,
        b"SimpleXMLElement::key" => m_key,
        b"SimpleXMLElement::next" => m_next,
        b"SimpleXMLElement::hasChildren" => m_has_children,
        b"SimpleXMLElement::getChildren" => m_get_children,
        _ => return None,
    })
}

fn m_construct(
    cx: &mut Cx,
    sxe: &Rc<SxeObject>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let data = lx::str_arg(args, 0);
    let data = data.as_bytes();
    let options = conv::to_int(&arg(args, 1));
    let is_url = conv::to_bool(&arg(args, 2));
    let ns = bytes(args, 3);
    let isprefix = conv::to_bool(&arg(args, 4));
    let exc = |n: usize, p: &str, what: &str| Thrown::Error {
        class: "Exception",
        message: format!("SimpleXMLElement::__construct(): Argument #{n} (${p}) {what}"),
    };
    if data.len() as i64 > INT_MAX {
        return Err(exc(1, "data", "is too long"));
    }
    if ns.len() as i64 > INT_MAX {
        return Err(exc(4, "namespaceOrPrefix", "is too long"));
    }
    if options > INT_MAX || options < i32::MIN as i64 {
        return Err(exc(2, "options", "is invalid"));
    }
    let docp = read_doc(cx, data, is_url, options)?;
    if docp.is_null() {
        return Err(Thrown::Error {
            class: "Exception",
            message: "String could not be parsed as XML".into(),
        });
    }
    sxe.free_iterxpath();
    {
        let mut it = sxe.iter.borrow_mut();
        it.nsprefix = (!ns.is_empty()).then_some(ns);
        it.isprefix = isprefix;
    }
    lxn::increment_doc_ref(&sxe.base, docp);
    lxn::increment_node_ptr(
        &sxe.base,
        unsafe { sys::xmlDocGetRootElement(docp) },
        ptr::null(),
    );
    Ok(Value::Null)
}

fn xpath_type_name(t: c_int) -> &'static str {
    match t {
        sys::XPATH_BOOLEAN => "bool",
        sys::XPATH_NUMBER => "number",
        sys::XPATH_STRING => "string",
        _ => "undefined",
    }
}

fn m_xpath(cx: &mut Cx, sxe: &Rc<SxeObject>, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let query = cs(&bytes(args, 0));
    if sxe.ty() == IterType::AttrList {
        return Ok(Value::Null);
    }
    let node = get_node(sxe)?;
    let nodeptr = first_node(cx, sxe, node)?;
    if nodeptr.is_null() {
        return Ok(Value::Null);
    }
    let doc = sxe.document_ptr();
    let mut ctx = sxe.xpath.get();
    unsafe {
        if ctx.is_null() {
            ctx = sys::xmlXPathNewContext(doc);
            sxe.xpath.set(ctx);
        }
        sys::rhp_xml_xpath_set_node(ctx, nodeptr);
        let ns = sys::xmlGetNsList(doc, nodeptr);
        let mut nsnbr = 0;
        if !ns.is_null() {
            while !(*ns.add(nsnbr)).is_null() {
                nsnbr += 1;
            }
        }
        sys::rhp_xml_xpath_set_namespaces(ctx, ns, nsnbr as c_int);
        let retval = cx.with_libxml_owned(
            || sys::xmlXPathEval(query.as_ptr(), ctx),
            |r| {
                if !r.is_null() {
                    sys::xmlXPathFreeObject(r)
                }
            },
        );
        if !ns.is_null() {
            sys::xml_free(ns);
            sys::rhp_xml_xpath_set_namespaces(ctx, ptr::null_mut(), 0);
        }
        let retval = retval?;
        if retval.is_null() {
            return Ok(Value::Bool(false));
        }
        struct FreeObj(*mut sys::XmlXPathObject);
        impl Drop for FreeObj {
            fn drop(&mut self) {
                unsafe { sys::xmlXPathFreeObject(self.0) };
            }
        }
        let _free = FreeObj(retval);
        if (*retval).type_ != sys::XPATH_NODESET {
            cx.warn(&format!(
                "XPath expression must return a node set, {} returned",
                xpath_type_name((*retval).type_)
            ))?;
            return Ok(Value::Bool(false));
        }
        let mut out = PhpArray::new();
        let result = (*retval).nodesetval;
        if !result.is_null() {
            for i in 0..(*result).nodeNr as usize {
                let n = *(*result).nodeTab.add(i);
                let v = match (*n).type_ {
                    sys::XML_TEXT_NODE => {
                        node_as_zval(cx, sxe, (*n).parent, IterType::None, None, None, false)?
                    }
                    sys::XML_ATTRIBUTE_NODE => {
                        let href = if (*n).ns.is_null() {
                            None
                        } else {
                            Some(xs((*(*n).ns).href))
                        };
                        let name = xs((*n).name);
                        node_as_zval(
                            cx,
                            sxe,
                            (*n).parent,
                            IterType::AttrList,
                            Some(&name),
                            href.as_deref(),
                            false,
                        )?
                    }
                    sys::XML_ELEMENT_NODE | sys::XML_PI_NODE | sys::XML_COMMENT_NODE => {
                        node_as_zval(cx, sxe, n, IterType::None, None, None, false)?
                    }
                    _ => continue,
                };
                out.push(v);
            }
        }
        Ok(Value::Array(out))
    }
}

fn m_register_xpath_namespace(
    _cx: &mut Cx,
    sxe: &Rc<SxeObject>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let prefix = cs(&bytes(args, 0));
    let uri = cs(&bytes(args, 1));
    if sxe.base.document.get().is_null() {
        return Err(not_initialized());
    }
    unsafe {
        let mut ctx = sxe.xpath.get();
        if ctx.is_null() {
            ctx = sys::xmlXPathNewContext(sxe.document_ptr());
            sxe.xpath.set(ctx);
        }
        Ok(Value::Bool(
            sys::xmlXPathRegisterNs(ctx, prefix.as_ptr(), uri.as_ptr()) == 0,
        ))
    }
}

fn m_as_xml(cx: &mut Cx, sxe: &Rc<SxeObject>, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let filename = opt_bytes(args, 0);
    if filename.as_ref().is_some_and(|f| f.contains(&0)) {
        let f = cx.fname();
        return Err(value_error(format!(
            "{f}(): Argument #1 ($filename) must not contain any null bytes"
        )));
    }
    let node = get_node(sxe)?;
    let node = first_node(cx, sxe, node)?;
    if node.is_null() {
        return Ok(Value::Bool(false));
    }
    let doc = sxe.document_ptr();
    let is_doc_child =
        unsafe { !(*node).parent.is_null() && (*(*node).parent).type_ == sys::XML_DOCUMENT_NODE };
    if let Some(f) = filename {
        let cf = cs(&f);
        let bytes = cx.with_libxml(|| unsafe {
            if is_doc_child {
                lxn::dump_doc_to_file(&cf, doc, false)
            } else {
                lxn::dump_node_to_file(&cf, doc, node, false)
            }
        })?;
        return Ok(Value::Bool(bytes != -1));
    }
    let encoding = unsafe { (*doc).encoding };
    let result = cx.with_libxml(|| {
        if is_doc_child {
            lxn::dump_doc_to_str(doc, 0, encoding)
        } else {
            lxn::dump_node_to_str(doc, node, false, encoding)
        }
    })?;
    Ok(match result {
        Some(s) => Value::str(s),
        None => Value::Bool(false),
    })
}

/// sxe_add_namespace_name_raw
fn add_namespace_name(out: &mut PhpArray, prefix: &[u8], href: &[u8]) {
    let key = ArrayKey::from_bytes(prefix);
    if out.get(&key).is_none() {
        out.insert(key, Value::str(href.to_vec()));
    }
}

fn add_ns(out: &mut PhpArray, ns: *const XmlNs) {
    unsafe { add_namespace_name(out, sys::cstr((*ns).prefix), sys::cstr((*ns).href)) }
}

/// sxe_add_namespaces
fn add_namespaces(node: *mut XmlNode, recursive: bool, out: &mut PhpArray) {
    unsafe {
        if !(*node).ns.is_null() {
            add_ns(out, (*node).ns);
        }
        let mut attr = (*node).properties;
        while !attr.is_null() {
            if !(*attr).ns.is_null() {
                add_ns(out, (*attr).ns);
            }
            attr = (*attr).next;
        }
        if recursive {
            let mut c = (*node).children;
            while !c.is_null() {
                if (*c).type_ == sys::XML_ELEMENT_NODE {
                    add_namespaces(c, recursive, out);
                }
                c = (*c).next;
            }
        }
    }
}

fn m_get_namespaces(
    cx: &mut Cx,
    sxe: &Rc<SxeObject>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let recursive = conv::to_bool(&arg(args, 0));
    let mut out = PhpArray::new();
    let node = get_node(sxe)?;
    let node = first_node(cx, sxe, node)?;
    unsafe {
        if !node.is_null() {
            if (*node).type_ == sys::XML_ELEMENT_NODE {
                add_namespaces(node, recursive, &mut out);
            } else if (*node).type_ == sys::XML_ATTRIBUTE_NODE && !(*node).ns.is_null() {
                add_ns(&mut out, (*node).ns);
            }
        }
    }
    Ok(Value::Array(out))
}

/// sxe_add_registered_namespaces
fn add_registered_namespaces(
    node: *mut XmlNode,
    recursive: bool,
    xmlns_attrs: bool,
    out: &mut PhpArray,
) {
    unsafe {
        if (*node).type_ != sys::XML_ELEMENT_NODE {
            return;
        }
        let mut ns = (*node).nsDef;
        while !ns.is_null() {
            add_ns(out, ns);
            ns = (*ns).next;
        }
        if xmlns_attrs {
            let mut attr = (*node).properties;
            while !attr.is_null() {
                let ans = (*attr).ns;
                if !ans.is_null() && xeq((*ans).href, b"http://www.w3.org/2000/xmlns/") {
                    let prefix: &[u8] = if (*ans).prefix.is_null() {
                        b""
                    } else {
                        sys::cstr((*attr).name)
                    };
                    let href = lxn::attr_value(attr);
                    add_namespace_name(out, prefix, &href);
                }
                attr = (*attr).next;
            }
        }
        if recursive {
            let mut c = (*node).children;
            while !c.is_null() {
                add_registered_namespaces(c, recursive, xmlns_attrs, out);
                c = (*c).next;
            }
        }
    }
}

fn m_get_doc_namespaces(
    _cx: &mut Cx,
    sxe: &Rc<SxeObject>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let recursive = conv::to_bool(&arg(args, 0));
    let from_root = args.len() < 2 || conv::to_bool(&arg(args, 1));
    let node = if from_root {
        if sxe.base.document.get().is_null() {
            return Err(not_initialized());
        }
        unsafe { sys::xmlDocGetRootElement(sxe.document_ptr()) }
    } else {
        get_node(sxe)?
    };
    if node.is_null() {
        return Ok(Value::Bool(false));
    }
    let doc = sxe.base.document.get();
    let modern = unsafe { (*doc).class_type == ClassType::Modern };
    let mut out = PhpArray::new();
    add_registered_namespaces(node, recursive, modern, &mut out);
    Ok(Value::Array(out))
}

fn m_children(
    cx: &mut Cx,
    sxe: &Rc<SxeObject>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let nsprefix = opt_bytes(args, 0);
    let isprefix = conv::to_bool(&arg(args, 1));
    if sxe.ty() == IterType::AttrList {
        return Ok(Value::Null);
    }
    let node = get_node(sxe)?;
    let node = first_node(cx, sxe, node)?;
    if node.is_null() {
        return Ok(Value::Null);
    }
    node_as_zval(
        cx,
        sxe,
        node,
        IterType::Child,
        None,
        nsprefix.as_deref(),
        isprefix,
    )
}

fn m_get_name(cx: &mut Cx, sxe: &Rc<SxeObject>, _: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let node = get_node(sxe)?;
    let node = first_node(cx, sxe, node)?;
    Ok(Value::str(if node.is_null() {
        Vec::new()
    } else {
        xs(unsafe { (*node).name })
    }))
}

fn m_attributes(
    cx: &mut Cx,
    sxe: &Rc<SxeObject>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let nsprefix = opt_bytes(args, 0);
    let isprefix = conv::to_bool(&arg(args, 1));
    let node = get_node(sxe)?;
    let node = first_node(cx, sxe, node)?;
    if node.is_null() || sxe.ty() == IterType::AttrList {
        return Ok(Value::Null);
    }
    node_as_zval(
        cx,
        sxe,
        node,
        IterType::AttrList,
        None,
        nsprefix.as_deref(),
        isprefix,
    )
}

/// xmlSplitQName2: (localname, prefix) for a prefixed name.
fn split_qname(qname: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let q = cs(qname);
    let mut prefix: *mut u8 = ptr::null_mut();
    unsafe {
        let local = sys::xmlSplitQName2(q.as_ptr(), &mut prefix);
        if local.is_null() {
            sys::xml_free(prefix);
            return None;
        }
        let l = sys::take_string(local).unwrap_or_default();
        let p = sys::take_string(prefix).unwrap_or_default();
        Some((l, p))
    }
}

fn m_add_child(
    cx: &mut Cx,
    sxe: &Rc<SxeObject>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let qname = bytes(args, 0);
    let value = opt_bytes(args, 1);
    let nsuri = opt_bytes(args, 2);
    if qname.is_empty() {
        let f = cx.fname();
        return Err(value_error(format!(
            "{f}(): Argument #1 ($qualifiedName) must not be empty"
        )));
    }
    let node = get_node(sxe)?;
    if sxe.ty() == IterType::AttrList {
        cx.warn("Cannot add element to attributes")?;
        return Ok(Value::Null);
    }
    let node = first_node(cx, sxe, node)?;
    if node.is_null() {
        cx.warn("Cannot add child. Parent is not a permanent member of the XML tree")?;
        return Ok(Value::Null);
    }
    unsafe {
        lxn::invalidate_node_list_cache_from_doc((*node).doc);
        let (localname, prefix) = match split_qname(&qname) {
            Some((l, p)) => (l, Some(p)),
            None => (cut(&qname).to_vec(), None),
        };
        let cl = cs(&localname);
        let cv = value.as_deref().map(cs);
        let newnode = cx.with_libxml(|| {
            sys::xmlNewChild(
                node,
                ptr::null_mut(),
                cl.as_ptr(),
                cv.as_ref().map_or(ptr::null(), |v| v.as_ptr()),
            )
        })?;
        let cp = prefix.as_deref().map(cs);
        let pp = cp.as_ref().map_or(ptr::null(), |p| p.as_ptr());
        if let Some(uri) = &nsuri {
            let cu = cs(uri);
            if uri.is_empty() {
                (*newnode).ns = ptr::null_mut();
                sys::xmlNewNs(newnode, cu.as_ptr(), pp);
            } else {
                let mut nsptr = sys::xmlSearchNsByHref((*node).doc, node, cu.as_ptr());
                if nsptr.is_null() {
                    nsptr = sys::xmlNewNs(newnode, cu.as_ptr(), pp);
                }
                (*newnode).ns = nsptr;
            }
        }
        let retprefix = if (prefix.is_some() || nsuri.is_some()) && !(*newnode).ns.is_null() {
            let p = (*(*newnode).ns).prefix;
            (!p.is_null()).then(|| xs(p))
        } else {
            None
        };
        node_as_zval(
            cx,
            sxe,
            newnode,
            IterType::None,
            Some(&localname),
            retprefix.as_deref(),
            true,
        )
    }
}

fn m_add_attribute(
    cx: &mut Cx,
    sxe: &Rc<SxeObject>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let qname = bytes(args, 0);
    let value = bytes(args, 1);
    let nsuri = opt_bytes(args, 2);
    if qname.is_empty() {
        let f = cx.fname();
        return Err(value_error(format!(
            "{f}(): Argument #1 ($qualifiedName) must not be empty"
        )));
    }
    let node = get_node(sxe)?;
    let mut node = first_node(cx, sxe, node)?;
    unsafe {
        if !node.is_null() && (*node).type_ != sys::XML_ELEMENT_NODE {
            node = (*node).parent;
        }
        if node.is_null() {
            cx.warn("Unable to locate parent Element")?;
            return Ok(Value::Null);
        }
        let (localname, prefix) = match split_qname(&qname) {
            Some((l, p)) => (l, Some(p)),
            None => {
                if nsuri.as_ref().is_some_and(|u| !u.is_empty()) {
                    cx.warn("Attribute requires prefix for namespace")?;
                    return Ok(Value::Null);
                }
                (cut(&qname).to_vec(), None)
            }
        };
        let cl = cs(&localname);
        let cu = nsuri.as_deref().map(cs);
        let up = cu.as_ref().map_or(ptr::null(), |u| u.as_ptr());
        let attrp = sys::xmlHasNsProp(node, cl.as_ptr(), up);
        if !attrp.is_null() && (*attrp).type_ != sys::XML_ATTRIBUTE_DECL {
            cx.warn("Attribute already exists")?;
            return Ok(Value::Null);
        }
        let mut nsptr: *mut XmlNs = ptr::null_mut();
        if !up.is_null() {
            nsptr = sys::xmlSearchNsByHref((*node).doc, node, up);
            if nsptr.is_null() {
                let cp = prefix.as_deref().map(cs);
                nsptr = sys::xmlNewNs(node, up, cp.as_ref().map_or(ptr::null(), |p| p.as_ptr()));
            }
        }
        let cv = cs(&value);
        sys::xmlNewNsProp(node, nsptr, cl.as_ptr(), cv.as_ptr());
    }
    Ok(Value::Null)
}

fn m_to_string(cx: &mut Cx, sxe: &Rc<SxeObject>, _: &Value, _: &[Value]) -> Result<Value, Thrown> {
    Ok(cast_ex(cx, sxe, CastTarget::Str)?.unwrap_or_else(|| Value::str("")))
}

fn m_debug_info(cx: &mut Cx, sxe: &Rc<SxeObject>, _: &Value, _: &[Value]) -> Result<Value, Thrown> {
    if sxe.base.xml_node().is_null() {
        return Ok(Value::Array(PhpArray::new()));
    }
    Ok(Value::Array(prop_hash(cx, sxe, true)?))
}

fn m_count(cx: &mut Cx, sxe: &Rc<SxeObject>, _: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let mut count = 0;
    let mut node = reset_iterator_no_clear(cx, sxe, false)?;
    while !node.is_null() {
        count += 1;
        node = iterator_fetch(cx, sxe, unsafe { (*node).next }, false)?;
    }
    Ok(Value::Int(count))
}

fn m_rewind(cx: &mut Cx, sxe: &Rc<SxeObject>, _: &Value, _: &[Value]) -> Result<Value, Thrown> {
    reset_iterator(cx, sxe)?;
    Ok(Value::Null)
}

fn m_valid(_: &mut Cx, sxe: &Rc<SxeObject>, _: &Value, _: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(sxe.iter.borrow().data.is_some()))
}

fn consumed() -> Thrown {
    Thrown::error("Error", "Iterator not initialized or already consumed")
}

fn m_current(_: &mut Cx, sxe: &Rc<SxeObject>, _: &Value, _: &[Value]) -> Result<Value, Thrown> {
    sxe.iter.borrow().data.clone().ok_or_else(consumed)
}

fn m_key(_: &mut Cx, sxe: &Rc<SxeObject>, _: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let data = sxe.iter.borrow().data.clone().ok_or_else(consumed)?;
    let intern = sxe_payload(&data).ok_or_else(consumed)?;
    let n = intern.base.xml_node();
    if intern.base.node.get().is_null() {
        return Err(consumed());
    }
    Ok(Value::str(xs(unsafe { (*n).name })))
}

fn m_next(cx: &mut Cx, sxe: &Rc<SxeObject>, _: &Value, _: &[Value]) -> Result<Value, Thrown> {
    move_forward(cx, sxe)?;
    Ok(Value::Null)
}

fn m_has_children(
    _: &mut Cx,
    sxe: &Rc<SxeObject>,
    _: &Value,
    _: &[Value],
) -> Result<Value, Thrown> {
    let data = sxe.iter.borrow().data.clone();
    let Some(data) = data.filter(|_| sxe.ty() != IterType::AttrList) else {
        return Ok(Value::Bool(false));
    };
    let Some(child) = sxe_payload(&data) else {
        return Ok(Value::Bool(false));
    };
    let mut node = get_node(&child)?;
    unsafe {
        node = (*node).children;
        while !node.is_null() && (*node).type_ != sys::XML_ELEMENT_NODE {
            node = (*node).next;
        }
    }
    Ok(Value::Bool(!node.is_null()))
}

fn m_get_children(
    _: &mut Cx,
    sxe: &Rc<SxeObject>,
    _: &Value,
    _: &[Value],
) -> Result<Value, Thrown> {
    if sxe.ty() == IterType::AttrList {
        return Ok(Value::Null);
    }
    Ok(sxe.iter.borrow().data.clone().unwrap_or(Value::Null))
}

// ---- the dispatchers ----

/// `__sxe("SimpleXMLElement::method", $this, ...$args)`
fn fn_sxe(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let Some(m) = method(&name) else {
        return Err(Thrown::error(
            "Error",
            format!(
                "Call to undefined method {}()",
                String::from_utf8_lossy(&name)
            ),
        ));
    };
    let this = arg(args, 1);
    let Some(sxe) = sxe_of(host, &this) else {
        return Err(not_initialized());
    };
    let mut cx = Cx {
        host,
        fname: String::from_utf8_lossy(&name).into_owned(),
        lazy: false,
    };
    m(&mut cx, &sxe, &this, &args[2.min(args.len())..])
}

/// `__sxe_prop($op, $this, $name [, $value])`: php's read_property (0),
/// write_property (1), has_property (2) and unset_property (3).
fn fn_sxe_prop(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mode = host.magic_fetch();
    let op = conv::to_int(&arg(args, 0));
    let this = arg(args, 1);
    let name = bytes(args, 2);
    let Some(sxe) = sxe_of(host, &this) else {
        return Ok(Value::Null);
    };
    let mut cx = Cx {
        host,
        fname: String::new(),
        lazy: true,
    };
    match op {
        0 => {
            if mode == crate::MagicFetch::Write {
                return property_get_adr(&mut cx, &sxe, &name);
            }
            prop_dim_read(&mut cx, &sxe, Some(Member::Name(name)), false, Fetch::R)
        }
        1 => {
            let value = arg(args, 3);
            prop_dim_write(&mut cx, &sxe, Some(&Value::str(name)), Some(&value), false)?;
            Ok(Value::Null)
        }
        2 => {
            let check_empty = (mode == crate::MagicFetch::Empty) as i32;
            Ok(Value::Bool(prop_dim_exists(
                &mut cx,
                &sxe,
                &Value::str(name),
                check_empty,
                false,
            )?))
        }
        _ => {
            prop_dim_delete(&mut cx, &sxe, &Value::str(name), false)?;
            Ok(Value::Null)
        }
    }
}

/// `__sxe_dim($op, $this, $offset [, $value])`: php's read_dimension (0),
/// write_dimension (1), has_dimension (2) and unset_dimension (3). A null
/// offset is `$sxe[]`.
fn fn_sxe_dim(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mode = host.magic_fetch();
    let op = conv::to_int(&arg(args, 0));
    let this = arg(args, 1);
    let offset = arg(args, 2);
    let Some(sxe) = sxe_of(host, &this) else {
        return Ok(Value::Null);
    };
    let mut cx = Cx {
        host,
        fname: String::new(),
        lazy: true,
    };
    let member = (!matches!(offset, Value::Null)).then_some(&offset);
    match op {
        0 => {
            let m = match member {
                Some(v) => Some(member_of(&mut cx, v)?),
                None => None,
            };
            let fetch = if mode == crate::MagicFetch::Write {
                Fetch::W
            } else {
                Fetch::R
            };
            prop_dim_read(&mut cx, &sxe, m, true, fetch)
        }
        1 => {
            let value = arg(args, 3);
            prop_dim_write(&mut cx, &sxe, member, Some(&value), true)?;
            Ok(Value::Null)
        }
        2 => {
            let check_empty = (mode == crate::MagicFetch::Empty) as i32;
            Ok(Value::Bool(prop_dim_exists(
                &mut cx,
                &sxe,
                &offset,
                check_empty,
                true,
            )?))
        }
        _ => {
            prop_dim_delete(&mut cx, &sxe, &offset, true)?;
            Ok(Value::Null)
        }
    }
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("simplexml_load_string", fn_load_string),
            ("simplexml_load_file", fn_load_file),
            ("simplexml_import_dom", fn_import_dom),
        ],
    );
    crate::register_helpers(
        m,
        &[
            ("__sxe", fn_sxe, 2, None),
            ("__sxe_prop", fn_sxe_prop, 3, Some(4)),
            ("__sxe_dim", fn_sxe_dim, 3, Some(4)),
        ],
    );
}

pub const PRELUDE_PHP: &str = include_str!("prelude.php");
