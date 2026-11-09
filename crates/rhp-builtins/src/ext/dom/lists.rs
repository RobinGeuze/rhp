//! DOMNodeList and DOMNamedNodeMap: php's `dom_nnodemap_object` and its
//! handlers (ext/dom/obj_map.c, nodelist.c, namednodemap.c,
//! dom_iterators.c). A list is live: it walks the tree on every access,
//! caching the last item and the length until the document changes.

use std::rc::Rc;

use rhp_values::{conv, Value};

use super::util::*;
use super::{create_object, int, Cx, DomObject, Kind};
use crate::ext::libxml::node as lxn;
use crate::ext::libxml::sys::{self, XmlNode};
use crate::Thrown;

/// php_dom_obj_map_handler
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Handler {
    Attributes,
    ByTagName,
    ChildNodes,
    Nodeset,
    Entities(*mut sys::xmlHashTable),
    Notations(*mut sys::xmlHashTable),
    /// Dom\HTMLCollection of an element's element children.
    ChildElements,
    /// getElementsByClassName (the classes are in the map).
    ByClassName,
    Noop,
}

impl Handler {
    fn use_cache(self) -> bool {
        matches!(
            self,
            Handler::ByTagName
                | Handler::ChildNodes
                | Handler::ChildElements
                | Handler::ByClassName
        )
    }

    /// Keyed by position rather than by name when iterated.
    fn nameless(self) -> bool {
        !matches!(
            self,
            Handler::Attributes | Handler::Entities(_) | Handler::Notations(_)
        )
    }
}

/// dom_nnodemap_object
pub struct Map {
    /// The node the list is about, kept alive (`baseobj`).
    pub baseobj: Option<Value>,
    pub base: Option<Rc<DomObject>>,
    pub handler: Handler,
    pub cached_length: i64,
    /// An XPath query's nodes.
    pub array: Vec<Value>,
    pub local: Vec<u8>,
    pub ns: Option<Vec<u8>>,
    pub cache_tag: usize,
    pub cached_obj: Option<Value>,
    pub cached_obj_index: i64,
    /// getElementsByClassName's class set.
    pub class_names: Vec<Vec<u8>>,
}

impl Map {
    pub fn new() -> Map {
        Map {
            baseobj: None,
            base: None,
            handler: Handler::Noop,
            cached_length: -1,
            array: Vec::new(),
            local: Vec::new(),
            ns: None,
            cache_tag: 0,
            cached_obj: None,
            cached_obj_index: 0,
            class_names: Vec::new(),
        }
    }

    /// dom_nnodemap_objects_free_storage
    pub fn release(&mut self) {
        self.cached_obj = None;
        self.array.clear();
        self.base = None;
        self.baseobj = None;
    }

    fn base_node(&self) -> *mut XmlNode {
        self.base
            .as_ref()
            .map_or(std::ptr::null_mut(), |b| b.node())
    }

    fn reset_cache(&mut self) {
        self.cached_obj = None;
        self.cached_obj_index = 0;
        self.cached_length = -1;
    }
}

impl Default for Map {
    fn default() -> Self {
        Map::new()
    }
}

/// php_dom_is_cache_tag_stale_from_node
fn is_stale(tag: usize, node: *const XmlNode) -> bool {
    match unsafe { lxn::object_of_node(node) } {
        Some(obj) => match unsafe { obj.document.get().as_ref() } {
            Some(d) => d.cache_tag != tag,
            None => true,
        },
        None => true,
    }
}

/// php_dom_mark_cache_tag_up_to_date_from_node
fn tag_of(node: *const XmlNode) -> Option<usize> {
    let obj = unsafe { lxn::object_of_node(node) }?;
    unsafe { obj.document.get().as_ref() }.map(|d| d.cache_tag)
}

/// A new DOMNodeList / DOMNamedNodeMap over `base` (php_dom_create_obj_map).
pub fn new_map(
    cx: &mut Cx,
    base: &Rc<DomObject>,
    handler: Handler,
    class: &[u8],
) -> Result<Value, Thrown> {
    new_map_named(cx, base, handler, class, None, None)
}

pub fn new_map_named(
    cx: &mut Cx,
    base: &Rc<DomObject>,
    handler: Handler,
    class: &[u8],
    local: Option<&[u8]>,
    ns: Option<&[u8]>,
) -> Result<Value, Thrown> {
    let v = new_list_object(cx, class)?;
    let d = super::intern_of(&v).expect("list state");
    if let Kind::Map(m) = &d.kind {
        let mut m = m.borrow_mut();
        m.handler = handler;
        m.baseobj = base.base.object();
        m.base = Some(base.clone());
        if let Some(l) = local {
            m.local = l.to_vec();
        }
        m.ns = ns.map(|n| n.to_vec());
    }
    Ok(v)
}

/// object_init_ex of a list class (dom_nnodemap_objects_new).
pub fn new_list_object(cx: &mut Cx, class: &[u8]) -> Result<Value, Thrown> {
    let v = cx
        .host
        .instantiate(class, Vec::new())
        .ok_or_else(|| Thrown::error("Error", "Cannot create a DOM list"))?;
    super::ensure_intern(&v, || Kind::Map(std::cell::RefCell::new(Map::new())));
    Ok(v)
}

/// A DOMNodeList of the nodes an XPath query found.
pub fn nodeset_list(cx: &mut Cx, class: &[u8], nodes: Vec<Value>) -> Result<Value, Thrown> {
    let v = new_list_object(cx, class)?;
    let d = super::intern_of(&v).expect("list state");
    if let Kind::Map(m) = &d.kind {
        let mut m = m.borrow_mut();
        m.handler = Handler::Nodeset;
        m.array = nodes;
    }
    Ok(v)
}

fn map_of(d: &DomObject) -> Option<&std::cell::RefCell<Map>> {
    match &d.kind {
        Kind::Map(m) => Some(m),
        _ => None,
    }
}

// ---- lengths ----

fn handler_length(m: &Map) -> i64 {
    let nodep = m.base_node();
    unsafe {
        match m.handler {
            Handler::Entities(ht) | Handler::Notations(ht) => {
                if ht.is_null() {
                    0
                } else {
                    sys::xmlHashSize(ht) as i64
                }
            }
            Handler::Nodeset => m.array.len() as i64,
            Handler::Attributes => {
                let mut count = 0;
                if !nodep.is_null() {
                    let mut a = (*nodep).properties;
                    while !a.is_null() {
                        count += 1;
                        a = (*a).next;
                    }
                }
                count
            }
            Handler::ChildNodes => {
                let mut count = 0;
                if !nodep.is_null() {
                    let mut c = iter_start_first_child(nodep);
                    while !c.is_null() {
                        count += 1;
                        c = (*c).next;
                    }
                }
                count
            }
            Handler::ByTagName => {
                let mut count = 0;
                if !nodep.is_null() {
                    let first = first_child_of_container(nodep);
                    elements_by_tag_name_ns_raw(
                        nodep,
                        first,
                        m.ns.as_deref(),
                        &m.local,
                        &mut count,
                        i64::MAX - 1,
                    );
                }
                count
            }
            Handler::ChildElements => {
                let mut count = 0;
                if !nodep.is_null() {
                    let mut c = iter_start_first_child(nodep);
                    while !c.is_null() {
                        if (*c).type_ == sys::XML_ELEMENT_NODE {
                            count += 1;
                        }
                        c = (*c).next;
                    }
                }
                count
            }
            Handler::ByClassName => {
                let mut count = 0;
                if !nodep.is_null() {
                    let mut n = first_child_of_container(nodep);
                    while !n.is_null() {
                        if matches_class_name(m, n) {
                            count += 1;
                        }
                        n = next_in_tree_order(n, nodep);
                    }
                }
                count
            }
            Handler::Noop => 0,
        }
    }
}

/// dom_matches_class_name
fn matches_class_name(m: &Map, nodep: *mut XmlNode) -> bool {
    unsafe {
        if (*nodep).type_ != sys::XML_ELEMENT_NODE {
            return false;
        }
        let attr = sys::xmlHasNsProp(nodep, c"class".as_ptr().cast(), std::ptr::null());
        if attr.is_null() || (*attr).children.is_null() {
            return false;
        }
        let value = lxn::attr_value(attr);
        let quirks = m
            .base
            .as_ref()
            .and_then(|b| b.document().as_ref())
            .is_some_and(|d| d.quirks_mode == lxn::QuirksMode::Quirks);
        let present = super::modern::ordered_set_parser(&value, quirks);
        m.class_names.iter().all(|c| present.contains(c))
    }
}

/// dom_nodelist_iter_start_first_child
fn iter_start_first_child(nodep: *mut XmlNode) -> *mut XmlNode {
    unsafe {
        if (*nodep).type_ == sys::XML_ENTITY_REF_NODE {
            super::props::entity_reference_sync(nodep);
        }
        (*nodep).children
    }
}

/// php_dom_get_nodelist_length
pub fn nodelist_length(_cx: &mut Cx, d: &DomObject) -> i64 {
    let Some(m) = map_of(d) else { return 0 };
    let mut m = m.borrow_mut();
    if m.handler.use_cache() {
        let nodep = m.base_node();
        if nodep.is_null() {
            return 0;
        }
        if !is_stale(m.cache_tag, nodep) {
            if m.cached_length >= 0 {
                return m.cached_length;
            }
        } else {
            if let Some(t) = tag_of(nodep) {
                m.cache_tag = t;
            }
            m.reset_cache();
        }
    }
    let count = handler_length(&m);
    if m.handler.use_cache() {
        m.cached_length = count;
    }
    count
}

/// php_dom_get_namednodemap_length
pub fn map_length(_cx: &mut Cx, d: &DomObject) -> i64 {
    map_of(d).map_or(0, |m| handler_length(&m.borrow()))
}

// ---- items ----

/// The cached item to continue from, as (node, remaining index)
/// (dom_obj_map_get_start_point).
fn start_point(m: &mut Map, basep: *mut XmlNode, index: i64) -> (*mut XmlNode, i64) {
    if index >= m.cached_obj_index && m.cached_obj.is_some() && !is_stale(m.cache_tag, basep) {
        let cached = m
            .cached_obj
            .as_ref()
            .and_then(super::intern_of)
            .map_or(std::ptr::null_mut(), |d| d.node());
        if cached.is_null() {
            m.reset_cache();
        } else {
            return (cached, index - m.cached_obj_index);
        }
    }
    (std::ptr::null_mut(), index)
}

/// dom_map_cache_obj
fn cache_obj(m: &mut Map, itemnode: *mut XmlNode, index: i64, v: &Value) {
    if is_stale(m.cache_tag, itemnode) {
        if let Some(t) = tag_of(itemnode) {
            m.cache_tag = t;
        }
        m.reset_cache();
    } else {
        m.cached_obj = None;
    }
    m.cached_obj_index = index;
    m.cached_obj = Some(v.clone());
}

/// The handler's get_item.
pub fn get_item(cx: &mut Cx, d: &DomObject, index: i64) -> Result<Value, Thrown> {
    let Some(cell) = map_of(d) else {
        return Ok(Value::Null);
    };
    let (handler, base, nodep) = {
        let m = cell.borrow();
        (m.handler, m.base.clone(), m.base_node())
    };
    let ret = |cx: &mut Cx, n: *mut XmlNode| -> Result<Value, Thrown> {
        if n.is_null() {
            Ok(Value::Null)
        } else {
            create_object(cx, n, base.as_deref())
        }
    };
    unsafe {
        match handler {
            Handler::Entities(ht) => ret(cx, hash_iter(ht, index)),
            Handler::Notations(ht) => {
                let mut n = hash_iter(ht, index);
                if !n.is_null() {
                    let no = n as *mut sys::XmlNotation;
                    n = create_notation((*no).name, (*no).PublicID, (*no).SystemID);
                }
                ret(cx, n)
            }
            Handler::Nodeset => {
                let m = cell.borrow();
                Ok(if index >= 0 {
                    m.array.get(index as usize).cloned().unwrap_or(Value::Null)
                } else {
                    Value::Null
                })
            }
            Handler::Attributes => {
                let mut item: *mut XmlNode = std::ptr::null_mut();
                if !nodep.is_null() && index >= 0 {
                    item = (*nodep).properties.cast();
                    let mut i = index;
                    while i > 0 && !item.is_null() {
                        item = (*item).next;
                        i -= 1;
                    }
                }
                ret(cx, item)
            }
            Handler::ChildNodes | Handler::ByTagName => {
                let mut item: *mut XmlNode = std::ptr::null_mut();
                if !nodep.is_null() && index >= 0 {
                    let (sp, rel) = start_point(&mut cell.borrow_mut(), nodep, index);
                    if handler == Handler::ChildNodes {
                        item = if sp.is_null() {
                            iter_start_first_child(nodep)
                        } else {
                            sp
                        };
                        let mut i = rel;
                        while i > 0 && !item.is_null() {
                            item = (*item).next;
                            i -= 1;
                        }
                    } else {
                        let start = if sp.is_null() {
                            first_child_of_container(nodep)
                        } else {
                            sp
                        };
                        let m = cell.borrow();
                        let mut count = 0;
                        item = elements_by_tag_name_ns_raw(
                            nodep,
                            start,
                            m.ns.as_deref(),
                            &m.local,
                            &mut count,
                            rel,
                        );
                    }
                }
                let v = ret(cx, item)?;
                if !item.is_null() {
                    cache_obj(&mut cell.borrow_mut(), item, index, &v);
                }
                Ok(v)
            }
            Handler::ChildElements | Handler::ByClassName => {
                let mut item: *mut XmlNode = std::ptr::null_mut();
                if !nodep.is_null() && index >= 0 {
                    let (sp, rel) = start_point(&mut cell.borrow_mut(), nodep, index);
                    let m = cell.borrow();
                    let ok = |n: *mut XmlNode| {
                        if handler == Handler::ChildElements {
                            (*n).type_ == sys::XML_ELEMENT_NODE
                        } else {
                            matches_class_name(&m, n)
                        }
                    };
                    let step = |n: *mut XmlNode| {
                        if handler == Handler::ChildElements {
                            (*n).next
                        } else {
                            next_in_tree_order(n, nodep)
                        }
                    };
                    item = if sp.is_null() {
                        let mut n = if handler == Handler::ChildElements {
                            (*nodep).children
                        } else {
                            first_child_of_container(nodep)
                        };
                        while !n.is_null() && !ok(n) {
                            n = step(n);
                        }
                        n
                    } else {
                        sp
                    };
                    let mut i = rel;
                    while i > 0 && !item.is_null() {
                        item = step(item);
                        while !item.is_null() && !ok(item) {
                            item = step(item);
                        }
                        i -= 1;
                    }
                }
                let v = ret(cx, item)?;
                if !item.is_null() {
                    cache_obj(&mut cell.borrow_mut(), item, index, &v);
                }
                Ok(v)
            }
            Handler::Noop => Ok(Value::Null),
        }
    }
}

/// The handler's get_ns_named_item.
pub fn get_named_item(d: &DomObject, named: &[u8], ns: Option<&[u8]>) -> *mut XmlNode {
    let Some(cell) = map_of(d) else {
        return std::ptr::null_mut();
    };
    let m = cell.borrow();
    let cn = c(named);
    unsafe {
        match m.handler {
            Handler::Entities(ht) => {
                if ht.is_null() {
                    std::ptr::null_mut()
                } else {
                    sys::xmlHashLookup(ht, cn.as_ptr()).cast()
                }
            }
            Handler::Notations(ht) => {
                if ht.is_null() {
                    return std::ptr::null_mut();
                }
                let no = sys::xmlHashLookup(ht, cn.as_ptr()) as *mut sys::XmlNotation;
                if no.is_null() {
                    std::ptr::null_mut()
                } else {
                    create_notation((*no).name, (*no).PublicID, (*no).SystemID)
                }
            }
            Handler::Attributes => {
                let nodep = m.base_node();
                if nodep.is_null() {
                    return std::ptr::null_mut();
                }
                match ns {
                    Some(ns) => {
                        let cns = c(ns);
                        let item: *mut XmlNode =
                            sys::xmlHasNsProp(nodep, cn.as_ptr(), cns.as_ptr()).cast();
                        if !item.is_null() && (*item).type_ == sys::XML_ATTRIBUTE_DECL {
                            std::ptr::null_mut()
                        } else {
                            item
                        }
                    }
                    None => {
                        let modern = m
                            .base
                            .as_ref()
                            .is_some_and(|b| super::nscompat::follow_spec(b.document()));
                        if modern {
                            super::modern::get_attribute_node(nodep, named).cast()
                        } else {
                            sys::xmlHasProp(nodep, cn.as_ptr()).cast()
                        }
                    }
                }
            }
            _ => std::ptr::null_mut(),
        }
    }
}

pub fn named_item_value(
    cx: &mut Cx,
    d: &DomObject,
    named: &[u8],
    ns: Option<&[u8]>,
) -> Result<Value, Thrown> {
    let n = get_named_item(d, named, ns);
    if n.is_null() {
        return Ok(Value::Null);
    }
    let base = map_of(d).and_then(|m| m.borrow().base.clone());
    create_object(cx, n, base.as_deref())
}

/// dom_nodemap_or_nodelist_process_offset_as_named: Err(()) for a
/// non-numeric string (a name), else the index.
fn offset_index(v: &Value) -> Result<i64, ()> {
    match v {
        Value::Str(s) => match conv::scan_number(s.as_bytes()) {
            None => Err(()),
            Some((conv::Num::Int(n), _)) => Ok(n),
            Some((conv::Num::Float(f), _)) => Ok(f as i64),
        },
        other => Ok(conv::to_int(other)),
    }
}

/// The list's state; a list made with `new` has an empty one.
fn this_map(_cx: &mut Cx, this: &Value) -> Result<Rc<DomObject>, Thrown> {
    super::ensure_intern(this, || Kind::Map(std::cell::RefCell::new(Map::new())))
        .ok_or_else(|| Thrown::error("Error", "Couldn't fetch the list"))
}

pub fn nodelist_count(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let d = this_map(cx, this)?;
    Ok(Value::Int(nodelist_length(cx, &d)))
}

pub fn nodelist_item(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let d = this_map(cx, this)?;
    get_item(cx, &d, int(args, 0))
}

/// dom_nodelist_read_dimension / dom_nodemap_read_dimension: `$list[]`
/// below a write (`$list[][] = …`) has no offset.
fn no_offset(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<(), Thrown> {
    if matches!(super::arg(args, 0), Value::Null)
        && cx.host.magic_fetch() == crate::MagicFetch::Write
    {
        return Err(Thrown::error(
            "Error",
            format!(
                "Cannot access {} without offset",
                super::class_of(cx.host, this)
            ),
        ));
    }
    Ok(())
}

pub fn nodelist_offset_get(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    no_offset(cx, this, args)?;
    let d = this_map(cx, this)?;
    match offset_index(&super::arg(args, 0)) {
        Err(()) => Ok(Value::Null),
        Ok(i) => get_item(cx, &d, i),
    }
}

pub fn nodelist_offset_exists(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let d = this_map(cx, this)?;
    match offset_index(&super::arg(args, 0)) {
        Err(()) => Ok(Value::Bool(false)),
        Ok(i) => Ok(Value::Bool(i >= 0 && i < nodelist_length(cx, &d))),
    }
}

pub fn map_get_named_item(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let d = this_map(cx, this)?;
    named_item_value(cx, &d, &super::bytes(args, 0), None)
}

pub fn map_get_named_item_ns(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let d = this_map(cx, this)?;
    let uri = super::opt_bytes(args, 0).unwrap_or_default();
    let modern = map_of(&d)
        .and_then(|m| m.borrow().base.clone())
        .is_some_and(|b| super::nscompat::follow_spec(b.document()));
    let ns = if uri.is_empty() && modern {
        None
    } else {
        Some(&uri[..])
    };
    named_item_value(cx, &d, &super::bytes(args, 1), ns)
}

const INT_MAX: i64 = i32::MAX as i64;

pub fn map_item(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let index = int(args, 0);
    if !(0..=INT_MAX).contains(&index) {
        return Err(crate::zend::zpp::value_error(
            "DOMNamedNodeMap::item",
            1,
            "index",
            &format!("must be between 0 and {INT_MAX}"),
        ));
    }
    let d = this_map(cx, this)?;
    get_item(cx, &d, index)
}

pub fn map_count(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let d = this_map(cx, this)?;
    Ok(Value::Int(map_length(cx, &d)))
}

pub fn map_offset_get(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    no_offset(cx, this, args)?;
    let d = this_map(cx, this)?;
    let off = super::arg(args, 0);
    match offset_index(&off) {
        Err(()) => named_item_value(cx, &d, &conv::to_string_bytes(&off), None),
        Ok(i) => {
            if !(0..=INT_MAX).contains(&i) {
                return Err(Thrown::error(
                    "ValueError",
                    format!("must be between 0 and {INT_MAX}"),
                ));
            }
            get_item(cx, &d, i)
        }
    }
}

pub fn map_offset_exists(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let d = this_map(cx, this)?;
    let off = super::arg(args, 0);
    match offset_index(&off) {
        Err(()) => Ok(Value::Bool(
            !get_named_item(&d, &conv::to_string_bytes(&off), None).is_null(),
        )),
        Ok(i) => Ok(Value::Bool(i >= 0 && i < map_length(cx, &d))),
    }
}

/// php_dom_iterator_move_forward: the item after `cur` (the first one for
/// null), or null.
fn next_item(cx: &mut Cx, d: &Rc<DomObject>, cur: &Value, index: i64) -> Result<Value, Thrown> {
    let Some(cell) = map_of(d) else {
        return Ok(Value::Null);
    };
    let handler = cell.borrow().handler;
    if matches!(cur, Value::Null) {
        return get_item(cx, d, 0);
    }
    let Some(ci) = super::intern_of(cur) else {
        return Ok(Value::Null);
    };
    if ci.base.node.get().is_null() {
        return Ok(Value::Null);
    }
    // php keeps the sibling walk for the legacy API only.
    if !super::nscompat::follow_spec(ci.document())
        && matches!(handler, Handler::Attributes | Handler::ChildNodes)
    {
        let n = ci.node();
        let nn = if n.is_null() {
            std::ptr::null_mut()
        } else {
            unsafe { (*n).next }
        };
        if nn.is_null() {
            return Ok(Value::Null);
        }
        let base = cell.borrow().base.clone();
        return create_object(cx, nn, base.as_deref());
    }
    get_item(cx, d, index)
}

/// php_dom_iterator_current_key
fn item_key(d: &DomObject, cur: &Value, index: i64) -> Value {
    let nameless = map_of(d).is_none_or(|m| m.borrow().handler.nameless());
    if nameless {
        return Value::Int(index);
    }
    match super::intern_of(cur) {
        Some(ni) if !ni.node().is_null() => {
            let n = ni.node();
            if unsafe { (*n).type_ } == sys::XML_ATTRIBUTE_NODE
                && super::nscompat::follow_spec(ni.document())
            {
                Value::str(qualified_name(n))
            } else {
                super::xs(unsafe { (*n).name })
            }
        }
        _ => Value::Null,
    }
}

/// php_dom_iterator: the list, its current item and index. It has no
/// rewind function.
struct ListIter {
    list: Rc<DomObject>,
    _obj: Value,
    cur: Rc<std::cell::RefCell<Value>>,
    index: std::cell::Cell<i64>,
    /// getIterator()'s zend_object_iterator, which php registers as an
    /// object too (it takes an object id while the iterator lives).
    _iterator: Option<Value>,
}

fn iter_cx(host: &mut dyn crate::Host) -> Cx<'_> {
    Cx {
        host,
        fname: String::new(),
        quiet: false,
        lazy_fname: false,
    }
}

impl crate::zend::internal_iterator::IteratorFuncs for ListIter {
    fn valid(&self, _: &mut dyn crate::Host) -> Result<bool, Thrown> {
        Ok(!matches!(*self.cur.borrow(), Value::Null))
    }
    fn current(&self, _: &mut dyn crate::Host) -> Result<Option<Value>, Thrown> {
        Ok(match &*self.cur.borrow() {
            Value::Null => None,
            v => Some(v.clone()),
        })
    }
    fn key(&self, _: &mut dyn crate::Host) -> Result<Option<Value>, Thrown> {
        Ok(Some(item_key(
            &self.list,
            &self.cur.borrow(),
            self.index.get(),
        )))
    }
    fn move_forward(&self, host: &mut dyn crate::Host) -> Result<(), Thrown> {
        let cur = self.cur.borrow().clone();
        if matches!(cur, Value::Null) {
            return Ok(());
        }
        self.index.set(self.index.get() + 1);
        *self.cur.borrow_mut() = Value::Null;
        let next = next_item(&mut iter_cx(host), &self.list, &cur, self.index.get())?;
        *self.cur.borrow_mut() = next;
        Ok(())
    }
    fn rewind(&self, _: &mut dyn crate::Host) -> Result<(), Thrown> {
        Ok(())
    }
    fn can_rewind(&self) -> bool {
        false
    }
}

/// php_dom_get_iterator: the class's `get_iterator` (foreach), starting
/// at the first item.
pub fn get_iterator(
    host: &mut dyn crate::Host,
    h: &rhp_values::ObjectHandle,
    by_ref: bool,
) -> Result<rhp_values::ObjectHandle, Thrown> {
    new_iterator(host, h, by_ref, None)
}

fn new_iterator(
    host: &mut dyn crate::Host,
    h: &rhp_values::ObjectHandle,
    by_ref: bool,
    iterator: Option<Value>,
) -> Result<rhp_values::ObjectHandle, Thrown> {
    if by_ref {
        return Err(Thrown::error(
            "Error",
            "An iterator cannot be used with foreach by reference",
        ));
    }
    let v = Value::Object(h.clone());
    let mut cx = iter_cx(host);
    let d = this_map(&mut cx, &v)?;
    // zend_iterator_init takes the iterator's object id before the first
    // item is fetched.
    let cur = Rc::new(std::cell::RefCell::new(Value::Null));
    let it = crate::zend::internal_iterator::create(
        cx.host,
        Box::new(ListIter {
            list: d.clone(),
            _obj: v,
            cur: cur.clone(),
            index: std::cell::Cell::new(0),
            _iterator: iterator,
        }),
    )?;
    let first = next_item(&mut cx, &d, &Value::Null, 0)?;
    *cur.borrow_mut() = first;
    Ok(it)
}

/// getIterator() of the lists (zend_create_internal_iterator_zval): the
/// class's iterator, wrapped in an InternalIterator.
pub fn get_iterator_m(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let Value::Object(h) = this else {
        return Ok(Value::Null);
    };
    let inner = get_iterator(cx.host, h, false)?;
    Ok(Value::Object(new_iterator(
        cx.host,
        h,
        false,
        Some(Value::Object(inner)),
    )?))
}

/// The `index`th node of a by-class-name collection (no object made).
pub fn nth_node(d: &DomObject, index: i64) -> *mut XmlNode {
    let Some(cell) = map_of(d) else {
        return std::ptr::null_mut();
    };
    let m = cell.borrow();
    let nodep = m.base_node();
    if nodep.is_null() {
        return std::ptr::null_mut();
    }
    let mut n = first_child_of_container(nodep);
    let mut i = 0;
    while !n.is_null() {
        if matches_class_name(&m, n) {
            if i == index {
                return n;
            }
            i += 1;
        }
        n = next_in_tree_order(n, nodep);
    }
    std::ptr::null_mut()
}
