//! The `Dom\` collections' own parts: the dimension handlers of
//! Dom\NodeList, Dom\NamedNodeMap and Dom\HTMLCollection (php_dom.c,
//! nodelist.c, html_collection.c), HTMLCollection::namedItem, and
//! Dom\TokenList (token_list.c). PHP License 3.01.

use std::rc::Rc;

use rhp_values::{conv, ObjectHandle, Value};

use super::lists::{self, Handler};
use super::util::*;
use super::{create_object, intern_of, Cx, DomObject, Kind};
use crate::ext::libxml::node as lxn;
use crate::ext::libxml::sys::{self, XmlNode};
use crate::zend::object_handlers::{DimType, DimensionHandlers};
use crate::{Host, Thrown};

fn class(host: &dyn Host, h: &ObjectHandle) -> String {
    super::class_of(host, &Value::Object(h.clone()))
}

fn not_array(host: &dyn Host, h: &ObjectHandle) -> Thrown {
    Thrown::error(
        "Error",
        format!("Cannot use object of type {} as array", class(host, h)),
    )
}

fn illegal(host: &dyn Host, h: &ObjectHandle, offset: &Value, is: bool) -> Thrown {
    let tn = crate::zend::zpp::zval_type_name(host, offset);
    if is {
        Thrown::type_error(format!(
            "Cannot access offset of type {tn} in isset or empty"
        ))
    } else {
        Thrown::type_error(format!(
            "Cannot access offset of type {tn} on {}",
            class(host, h)
        ))
    }
}

/// ZEND_HANDLE_NUMERIC_STR: a canonical decimal integer key.
fn handle_numeric(s: &[u8]) -> Option<i64> {
    let digits = s.strip_prefix(b"-").unwrap_or(s);
    if digits.is_empty()
        || !digits.iter().all(u8::is_ascii_digit)
        || (digits.len() > 1 && digits[0] == b'0')
        || s == b"-0"
    {
        return None;
    }
    std::str::from_utf8(s).ok()?.parse().ok()
}

/// dom_modern_nodelist_get_index
enum Index {
    Long(i64),
    Str(Vec<u8>),
    Illegal,
}

fn index_of(host: &mut dyn Host, v: &Value) -> Result<Index, Thrown> {
    Ok(match super::deref(v) {
        Value::Int(n) => Index::Long(n),
        Value::Float(f) => Index::Long(crate::zend::operators::dval_to_lval_safe(host, f)?),
        Value::Str(s) => match handle_numeric(s.as_bytes()) {
            Some(n) => Index::Long(n),
            None => Index::Str(s.as_bytes().to_vec()),
        },
        _ => Index::Illegal,
    })
}

fn with_cx<R>(host: &mut dyn Host, f: impl FnOnce(&mut Cx) -> R) -> R {
    let mut cx = Cx {
        host,
        fname: String::new(),
        quiet: false,
        lazy_fname: true,
    };
    f(&mut cx)
}

fn state(h: &ObjectHandle) -> Option<Rc<DomObject>> {
    intern_of(&Value::Object(h.clone()))
}

fn appended(host: &dyn Host, h: &ObjectHandle) -> Thrown {
    Thrown::error("Error", format!("Cannot append to {}", class(host, h)))
}

fn read_list(
    host: &mut dyn Host,
    h: &ObjectHandle,
    offset: Option<&Value>,
    ty: DimType,
) -> Result<Option<Value>, Thrown> {
    let Some(offset) = offset else {
        return Err(appended(host, h));
    };
    let Some(d) = state(h) else { return Ok(None) };
    let named = matches!(class(host, h).as_str(), "Dom\\HTMLCollection");
    let v = match index_of(host, offset)? {
        Index::Illegal => return Err(illegal(host, h, offset, false)),
        Index::Str(s) if named => with_cx(host, |cx| named_item(cx, &d, &s))?,
        Index::Str(_) => return Err(illegal(host, h, offset, false)),
        Index::Long(i) => with_cx(host, |cx| lists::get_item(cx, &d, i))?,
    };
    Ok(match (ty, v) {
        (DimType::Is, Value::Null) => None,
        (_, v) => Some(v),
    })
}

fn has_list(
    host: &mut dyn Host,
    h: &ObjectHandle,
    offset: &Value,
    _: bool,
) -> Result<bool, Thrown> {
    let Some(d) = state(h) else { return Ok(false) };
    let named = matches!(class(host, h).as_str(), "Dom\\HTMLCollection");
    match index_of(host, offset)? {
        Index::Illegal => Err(illegal(host, h, offset, true)),
        Index::Str(s) if named => Ok(with_cx(host, |cx| named_item_node(cx, &d, &s)).is_some()),
        Index::Str(_) => Err(illegal(host, h, offset, true)),
        Index::Long(i) => Ok(i >= 0 && i < with_cx(host, |cx| lists::nodelist_length(cx, &d))),
    }
}

pub static LIST_DIMENSION: DimensionHandlers = DimensionHandlers {
    read: read_list,
    write: |host, h, _, _| Err(not_array(host, h)),
    has: has_list,
    unset: |host, h, _| Err(not_array(host, h)),
};

/// dom_modern_nodemap_read_dimension
fn read_map(
    host: &mut dyn Host,
    h: &ObjectHandle,
    offset: Option<&Value>,
    ty: DimType,
) -> Result<Option<Value>, Thrown> {
    let Some(offset) = offset else {
        return Err(appended(host, h));
    };
    let Some(d) = state(h) else { return Ok(None) };
    let v = match index_of(host, offset)? {
        Index::Illegal => return Err(illegal(host, h, offset, false)),
        Index::Str(s) => with_cx(host, |cx| lists::named_item_value(cx, &d, &s, None))?,
        Index::Long(i) => with_cx(host, |cx| lists::get_item(cx, &d, i))?,
    };
    Ok(match (ty, v) {
        (DimType::Is, Value::Null) => None,
        (_, v) => Some(v),
    })
}

fn has_map(host: &mut dyn Host, h: &ObjectHandle, offset: &Value, _: bool) -> Result<bool, Thrown> {
    let Some(d) = state(h) else { return Ok(false) };
    match index_of(host, offset)? {
        Index::Illegal => Err(illegal(host, h, offset, true)),
        Index::Str(s) => Ok(!lists::get_named_item(&d, &s, None).is_null()),
        Index::Long(i) => Ok(i >= 0 && i < with_cx(host, |cx| lists::map_length(cx, &d))),
    }
}

pub static MAP_DIMENSION: DimensionHandlers = DimensionHandlers {
    read: read_map,
    write: |host, h, _, _| Err(not_array(host, h)),
    has: has_map,
    unset: |host, h, _| Err(not_array(host, h)),
};

/// The elements a collection's named-item search visits, in order.
fn candidates(d: &DomObject) -> Vec<*mut XmlNode> {
    let mut out = Vec::new();
    let Kind::Map(m) = &d.kind else { return out };
    let m = m.borrow();
    let Some(base) = m.base.as_ref() else {
        return out;
    };
    let basep = base.node();
    if basep.is_null() || unsafe { (*basep).children }.is_null() {
        return out;
    }
    unsafe {
        match m.handler {
            Handler::ChildElements => {
                let mut c = (*basep).children;
                while !c.is_null() {
                    if (*c).type_ == sys::XML_ELEMENT_NODE {
                        out.push(c);
                    }
                    c = (*c).next;
                }
            }
            Handler::ByTagName => {
                let mut i = 0;
                loop {
                    let mut cur = 0;
                    let n = elements_by_tag_name_ns_raw(
                        basep,
                        (*basep).children,
                        m.ns.as_deref(),
                        &m.local,
                        &mut cur,
                        i,
                    );
                    if n.is_null() {
                        break;
                    }
                    out.push(n);
                    i += 1;
                }
            }
            Handler::ByClassName => {
                drop(m);
                let mut i = 0;
                loop {
                    let v = lists::nth_node(d, i);
                    if v.is_null() {
                        break;
                    }
                    out.push(v);
                    i += 1;
                }
            }
            _ => {}
        }
    }
    out
}

/// dom_html_collection_named_item
fn named_item_node(_cx: &mut Cx, d: &DomObject, key: &[u8]) -> Option<*mut XmlNode> {
    if key.is_empty() {
        return None;
    }
    for c in candidates(d) {
        unsafe {
            let attr = sys::xmlHasNsProp(c, c"id".as_ptr().cast(), std::ptr::null());
            if !attr.is_null() && lxn::attr_value(attr) == key {
                return Some(c);
            }
            if super::nscompat::ns_is_fast(c, &super::nscompat::HTML) {
                let attr = sys::xmlHasNsProp(c, c"name".as_ptr().cast(), std::ptr::null());
                if !attr.is_null() && lxn::attr_value(attr) == key {
                    return Some(c);
                }
            }
        }
    }
    None
}

fn named_item(cx: &mut Cx, d: &DomObject, key: &[u8]) -> Result<Value, Thrown> {
    match named_item_node(cx, d, key) {
        Some(n) => {
            let base = match &d.kind {
                Kind::Map(m) => m.borrow().base.clone(),
                _ => None,
            };
            create_object(cx, n, base.as_deref())
        }
        None => Ok(Value::Null),
    }
}

pub fn html_collection_named_item(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let Some(d) = intern_of(this) else {
        return Ok(Value::Null);
    };
    named_item(cx, &d, &super::bytes(args, 0))
}

// ---- Dom\TokenList (token_list.c) ----

/// dom_token_list_object: the element, the token set as php's hash keeps
/// it (deleted tokens leave holes that iterators skip), the attribute
/// value the set was parsed from, and the cache tag iterators compare.
pub struct TokenListState {
    pub element: Option<Value>,
    buckets: Vec<Option<Vec<u8>>>,
    table_size: usize,
    cached: Option<Vec<u8>>,
    tag: u64,
}

impl TokenListState {
    pub fn new(element: Option<Value>) -> Self {
        TokenListState {
            element,
            buckets: Vec::new(),
            table_size: 8,
            cached: None,
            tag: 0,
        }
    }

    fn live(&self) -> impl Iterator<Item = &Vec<u8>> {
        self.buckets.iter().flatten()
    }

    fn len(&self) -> usize {
        self.live().count()
    }

    fn contains(&self, t: &[u8]) -> bool {
        self.live().any(|x| x == t)
    }

    /// zend_hash_add (no overwrite), with zend_hash_do_resize's choice
    /// between compacting and growing.
    fn add(&mut self, t: &[u8]) {
        if self.contains(t) {
            return;
        }
        if self.buckets.len() >= self.table_size {
            let n = self.len();
            if self.buckets.len() > n + (n >> 5) {
                self.buckets.retain(Option::is_some);
            } else {
                self.table_size *= 2;
            }
        }
        self.buckets.push(Some(t.to_vec()));
    }

    /// zend_hash_del: a hole, trailing holes trimmed.
    fn del(&mut self, t: &[u8]) {
        if let Some(i) = self.buckets.iter().position(|b| b.as_deref() == Some(t)) {
            self.del_at(i);
        }
    }

    fn del_at(&mut self, i: usize) {
        self.buckets[i] = None;
        while matches!(self.buckets.last(), Some(None)) {
            self.buckets.pop();
        }
    }

    fn clear(&mut self) {
        self.buckets.clear();
        self.table_size = 8;
    }

    /// _zend_hash_get_valid_pos
    fn valid_pos(&self, mut pos: usize) -> usize {
        while pos < self.buckets.len() && self.buckets[pos].is_none() {
            pos += 1;
        }
        pos
    }
}

fn tl_state(d: &DomObject) -> Option<&std::cell::RefCell<TokenListState>> {
    match &d.kind {
        Kind::TokenList(s) => Some(s),
        _ => None,
    }
}

fn tl_element_obj(d: &DomObject) -> Option<Rc<DomObject>> {
    tl_state(d).and_then(|s| s.borrow().element.as_ref().and_then(intern_of))
}

/// A token list's element.
fn tl_element(d: &DomObject) -> *mut XmlNode {
    tl_element_obj(d).map_or(std::ptr::null_mut(), |e| e.node())
}

fn tl_document(d: &DomObject) -> *mut lxn::RefObj {
    tl_element_obj(d).map_or(std::ptr::null_mut(), |e| e.document())
}

/// dom_token_list_get_class_value
fn class_attr(el: *mut XmlNode) -> Option<Vec<u8>> {
    if el.is_null() {
        return None;
    }
    unsafe {
        let attr = sys::xmlHasNsProp(el, c"class".as_ptr().cast(), std::ptr::null());
        if attr.is_null() || (*attr).children.is_null() {
            None
        } else {
            Some(lxn::attr_value(attr))
        }
    }
}

/// dom_token_list_ensure_set_up_to_date
fn tl_sync(d: &DomObject) {
    let value = class_attr(tl_element(d));
    let Some(cell) = tl_state(d) else { return };
    let mut st = cell.borrow_mut();
    if value != st.cached {
        st.tag += 1;
        st.clear();
        if let Some(v) = &value {
            for t in super::modern::ordered_set_parser(v, false) {
                st.add(&t);
            }
        }
        st.cached = value;
    }
}

/// dom_token_list_update
fn tl_update(d: &DomObject) {
    let el = tl_element(d);
    let doc = tl_document(d);
    let Some(cell) = tl_state(d) else { return };
    let mut st = cell.borrow_mut();
    st.tag += 1;
    lxn::invalidate_node_list_cache(doc);
    if el.is_null() {
        return;
    }
    unsafe {
        let attr = sys::xmlHasNsProp(el, c"class".as_ptr().cast(), std::ptr::null());
        if attr.is_null() && st.len() == 0 {
            return;
        }
        let value: Vec<u8> = st.live().cloned().collect::<Vec<_>>().join(&b' ');
        let cv = c(&value);
        sys::xmlSetNsProp(
            el,
            std::ptr::null_mut(),
            c"class".as_ptr().cast(),
            cv.as_ptr(),
        );
        st.cached = Some(value);
    }
}

/// A new Dom\TokenList for an element's classList (dom_token_list_ctor).
pub fn new_token_list(cx: &mut Cx, element: &Rc<DomObject>) -> Result<Value, Thrown> {
    let v = cx
        .host
        .instantiate(b"Dom\\TokenList", Vec::new())
        .ok_or_else(|| Thrown::error("Error", "Cannot create a token list"))?;
    let elv = element.base.object();
    let d = super::ensure_intern(&v, || {
        Kind::TokenList(std::cell::RefCell::new(TokenListState::new(elv)))
    });
    if let Some(d) = d {
        tl_sync(&d);
    }
    Ok(v)
}

fn tl(this: &Value) -> Rc<DomObject> {
    intern_of(this).unwrap_or_else(|| {
        Rc::new(DomObject::new(Kind::TokenList(std::cell::RefCell::new(
            TokenListState::new(None),
        ))))
    })
}

/// dom_token_list_item_read
fn tl_item(d: &DomObject, index: i64) -> Value {
    tl_sync(d);
    let Some(cell) = tl_state(d) else {
        return Value::Null;
    };
    let st = cell.borrow();
    if index < 0 {
        return Value::Null;
    }
    let v = st
        .live()
        .nth(index as usize)
        .map_or(Value::Null, |t| Value::str(t.clone()));
    v
}

pub fn tl_item_m(_: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    Ok(tl_item(&tl(this), super::int(args, 0)))
}

pub fn tl_contains(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let token = super::bytes(args, 0);
    if token.contains(&0) {
        return Err(crate::zend::zpp::value_error(
            &cx.fname,
            1,
            "token",
            "must not contain any null bytes",
        ));
    }
    let d = tl(this);
    tl_sync(&d);
    Ok(Value::Bool(
        tl_state(&d).is_some_and(|s| s.borrow().contains(&token)),
    ))
}

/// dom_validate_token
fn validate_token(cx: &mut Cx, t: &[u8]) -> Result<(), Thrown> {
    if t.is_empty() {
        super::throw_msg(
            cx,
            super::SYNTAX_ERR,
            "The empty string is not a valid token",
            true,
        )?;
    }
    if t.iter()
        .any(|c| super::modern::ASCII_WHITESPACE.contains(c))
    {
        super::throw_msg(
            cx,
            super::INVALID_CHARACTER_ERR,
            "The token must not contain any ASCII whitespace",
            true,
        )?;
    }
    Ok(())
}

/// dom_validate_tokens_varargs
fn validate_varargs(cx: &mut Cx, args: &[Value]) -> Result<Vec<Vec<u8>>, Thrown> {
    let mut out = Vec::new();
    for (i, a) in args.iter().enumerate() {
        let a = super::deref(a);
        let Value::Str(s) = &a else {
            return Err(Thrown::type_error(format!(
                "{}(): Argument #{} must be of type string, {} given",
                cx.fname,
                i + 1,
                crate::zend::zpp::zval_value_name(cx.host, &a)
            )));
        };
        if s.as_bytes().contains(&0) {
            return Err(Thrown::error(
                "ValueError",
                format!(
                    "{}(): Argument #{} must not contain any null bytes",
                    cx.fname,
                    i + 1
                ),
            ));
        }
        validate_token(cx, s.as_bytes())?;
        out.push(s.as_bytes().to_vec());
    }
    Ok(out)
}

pub fn tl_add(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let tokens = validate_varargs(cx, args)?;
    let d = tl(this);
    tl_sync(&d);
    if let Some(cell) = tl_state(&d) {
        let mut st = cell.borrow_mut();
        for t in &tokens {
            st.add(t);
        }
    }
    tl_update(&d);
    Ok(Value::Null)
}

pub fn tl_remove(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let tokens = validate_varargs(cx, args)?;
    let d = tl(this);
    tl_sync(&d);
    if let Some(cell) = tl_state(&d) {
        let mut st = cell.borrow_mut();
        for t in &tokens {
            st.del(t);
        }
    }
    tl_update(&d);
    Ok(Value::Null)
}

pub fn tl_toggle(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let token = super::bytes(args, 0);
    let force = super::node::opt_bool(args, 1);
    if token.contains(&0) {
        return Err(crate::zend::zpp::value_error(
            &cx.fname,
            1,
            "token",
            "must not contain any null bytes",
        ));
    }
    validate_token(cx, &token)?;
    let d = tl(this);
    tl_sync(&d);
    let Some(cell) = tl_state(&d) else {
        return Ok(Value::Bool(false));
    };
    let found = cell.borrow().contains(&token);
    if found {
        if force != Some(true) {
            cell.borrow_mut().del(&token);
            tl_update(&d);
            return Ok(Value::Bool(false));
        }
        return Ok(Value::Bool(true));
    }
    if force != Some(false) {
        cell.borrow_mut().add(&token);
        tl_update(&d);
        return Ok(Value::Bool(true));
    }
    Ok(Value::Bool(false))
}

pub fn tl_replace(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let token = super::bytes(args, 0);
    let new_token = super::bytes(args, 1);
    for (i, (t, p)) in [(&token, "token"), (&new_token, "newToken")]
        .into_iter()
        .enumerate()
    {
        if t.contains(&0) {
            return Err(crate::zend::zpp::value_error(
                &cx.fname,
                i + 1,
                p,
                "must not contain any null bytes",
            ));
        }
    }
    validate_token(cx, &token)?;
    validate_token(cx, &new_token)?;
    let d = tl(this);
    tl_sync(&d);
    let Some(cell) = tl_state(&d) else {
        return Ok(Value::Bool(false));
    };
    {
        let mut st = cell.borrow_mut();
        let Some(pos) = st
            .buckets
            .iter()
            .position(|b| b.as_deref() == Some(&token[..]))
        else {
            return Ok(Value::Bool(false));
        };
        // zend_hash_set_bucket_key: in place, or deleted when the new key
        // exists elsewhere.
        if st.contains(&new_token) && new_token != token {
            st.del_at(pos);
        } else {
            st.buckets[pos] = Some(new_token);
        }
    }
    tl_update(&d);
    Ok(Value::Bool(true))
}

pub fn tl_supports(cx: &mut Cx, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    if super::bytes(args, 0).contains(&0) {
        return Err(crate::zend::zpp::value_error(
            &cx.fname,
            1,
            "token",
            "must not contain any null bytes",
        ));
    }
    Err(Thrown::type_error(
        "Attribute \"class\" does not define any supported tokens",
    ))
}

pub fn tl_count(_: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let d = tl(this);
    tl_sync(&d);
    Ok(Value::Int(
        tl_state(&d).map_or(0, |s| s.borrow().len()) as i64
    ))
}

pub fn tl_length(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let _ = cx;
    tl_sync(d);
    Ok(Some(Value::Int(
        tl_state(d).map_or(0, |s| s.borrow().len()) as i64,
    )))
}

pub fn tl_value(_: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    Ok(Some(Value::str(
        class_attr(tl_element(d)).unwrap_or_default(),
    )))
}

pub fn tl_value_write(_: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let s = conv::to_string_bytes(&v);
    if s.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            "Value must not contain any null bytes",
        ));
    }
    let el = tl_element(d);
    if el.is_null() {
        return Ok(());
    }
    lxn::invalidate_node_list_cache(tl_document(d));
    let cv = c(&s);
    unsafe {
        sys::xmlSetNsProp(
            el,
            std::ptr::null_mut(),
            c"class".as_ptr().cast(),
            cv.as_ptr(),
        )
    };
    Ok(())
}

fn read_tl(
    host: &mut dyn Host,
    h: &ObjectHandle,
    offset: Option<&Value>,
    ty: DimType,
) -> Result<Option<Value>, Thrown> {
    let Some(offset) = offset else {
        return Err(Thrown::error("Error", "Cannot append to Dom\\TokenList"));
    };
    let Some(i) = tl_offset(host, offset)? else {
        return Err(illegal(host, h, offset, false));
    };
    let d = tl(&Value::Object(h.clone()));
    Ok(match (ty, tl_item(&d, i)) {
        (DimType::Is, Value::Null) => None,
        (_, v) => Some(v),
    })
}

/// dom_token_list_offset_convert_to_long
fn tl_offset(host: &mut dyn Host, v: &Value) -> Result<Option<i64>, Thrown> {
    Ok(match super::deref(v) {
        Value::Str(s) => handle_numeric(s.as_bytes()),
        Value::Float(f) => Some(crate::zend::operators::dval_to_lval_safe(host, f)?),
        Value::Int(n) => Some(n),
        Value::Bool(b) => Some(b as i64),
        Value::Resource(r) => {
            let id = r.id as i64;
            host.emit_diagnostic(
                crate::E_WARNING,
                format!("Resource ID#{id} used as offset, casting to integer ({id})").as_bytes(),
            )?;
            Some(id)
        }
        _ => None,
    })
}

fn has_tl(
    host: &mut dyn Host,
    h: &ObjectHandle,
    offset: &Value,
    check_empty: bool,
) -> Result<bool, Thrown> {
    let Some(i) = tl_offset(host, offset)? else {
        return Err(illegal(host, h, offset, true));
    };
    let d = tl(&Value::Object(h.clone()));
    let v = tl_item(&d, i);
    if check_empty {
        return Ok(conv::to_bool(&v));
    }
    Ok(!matches!(v, Value::Null))
}

pub static TOKEN_LIST_DIMENSION: DimensionHandlers = DimensionHandlers {
    read: read_tl,
    write: |host, h, _, _| Err(not_array(host, h)),
    has: has_tl,
    unset: |host, h, _| Err(not_array(host, h)),
};

/// dom_token_list_it: a bucket position in the token set and the cache
/// tag it started with.
struct TokenIter {
    list: Rc<DomObject>,
    _obj: Value,
    pos: std::cell::Cell<usize>,
    tag: u64,
    /// getIterator()'s zend_object_iterator, which php registers as an
    /// object too (it takes an object id while the iterator lives).
    _iterator: Option<Value>,
}

impl TokenIter {
    fn with<R>(&self, f: impl FnOnce(&TokenListState) -> R) -> R {
        tl_sync(&self.list);
        let cell = tl_state(&self.list).expect("a token list");
        let st = cell.borrow();
        f(&st)
    }
}

impl crate::zend::internal_iterator::IteratorFuncs for TokenIter {
    fn valid(&self, _: &mut dyn Host) -> Result<bool, Thrown> {
        Ok(self.with(|st| {
            let p = st.valid_pos(self.pos.get());
            self.pos.set(p);
            p < st.buckets.len()
        }))
    }
    fn current(&self, _: &mut dyn Host) -> Result<Option<Value>, Thrown> {
        Ok(self.with(|st| {
            let p = st.valid_pos(self.pos.get());
            st.buckets.get(p).and_then(|b| b.clone()).map(Value::str)
        }))
    }
    fn key(&self, _: &mut dyn Host) -> Result<Option<Value>, Thrown> {
        Ok(self.with(|st| {
            if st.tag == self.tag {
                return None;
            }
            // Stale: count the positions up to this one again.
            let mut index = 0;
            let mut p = st.valid_pos(0);
            while p != self.pos.get() && p < st.buckets.len() {
                index += 1;
                p = st.valid_pos(p + 1);
            }
            Some(Value::Int(index))
        }))
    }
    fn move_forward(&self, _: &mut dyn Host) -> Result<(), Thrown> {
        self.with(|st| {
            let current = self.pos.get();
            let validated = st.valid_pos(current);
            if validated != current {
                self.pos.set(validated);
            } else if current < st.buckets.len() {
                self.pos.set(st.valid_pos(current + 1));
            }
        });
        Ok(())
    }
    fn rewind(&self, _: &mut dyn Host) -> Result<(), Thrown> {
        self.with(|st| self.pos.set(st.valid_pos(0)));
        Ok(())
    }
}

/// dom_token_list_get_iterator
pub fn tl_get_iterator(
    host: &mut dyn Host,
    h: &ObjectHandle,
    by_ref: bool,
) -> Result<ObjectHandle, Thrown> {
    tl_new_iterator(host, h, by_ref, None)
}

fn tl_new_iterator(
    host: &mut dyn Host,
    h: &ObjectHandle,
    by_ref: bool,
    iterator: Option<Value>,
) -> Result<ObjectHandle, Thrown> {
    if by_ref {
        return Err(Thrown::error(
            "Error",
            "An iterator cannot be used with foreach by reference",
        ));
    }
    let v = Value::Object(h.clone());
    let d = tl(&v);
    tl_sync(&d);
    let (pos, tag) = {
        let st = tl_state(&d).expect("a token list").borrow();
        (st.valid_pos(0), st.tag)
    };
    crate::zend::internal_iterator::create(
        host,
        Box::new(TokenIter {
            list: d,
            _obj: v,
            pos: std::cell::Cell::new(pos),
            tag,
            _iterator: iterator,
        }),
    )
}

/// Dom\TokenList::getIterator: an InternalIterator.
pub fn tl_get_iterator_m(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let Value::Object(h) = this else {
        return Ok(Value::Null);
    };
    // zend_create_internal_iterator_zval: the class's iterator, then the
    // InternalIterator wrapping it.
    let inner = tl_get_iterator(cx.host, h, false)?;
    Ok(Value::Object(tl_new_iterator(
        cx.host,
        h,
        false,
        Some(Value::Object(inner)),
    )?))
}
