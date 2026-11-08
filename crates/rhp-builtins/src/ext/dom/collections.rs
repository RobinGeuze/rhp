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

/// zend_dval_to_lval
fn dval_to_lval(f: f64) -> i64 {
    if !f.is_finite() || f >= 9.223_372_036_854_776e18 || f < -9.223_372_036_854_776e18 {
        0
    } else {
        f as i64
    }
}

/// dom_modern_nodelist_get_index
enum Index {
    Long(i64),
    Str(Vec<u8>),
    Illegal,
}

fn index_of(v: &Value) -> Index {
    match super::deref(v) {
        Value::Int(n) => Index::Long(n),
        Value::Float(f) => Index::Long(dval_to_lval(f)),
        Value::Str(s) => match handle_numeric(s.as_bytes()) {
            Some(n) => Index::Long(n),
            None => Index::Str(s.as_bytes().to_vec()),
        },
        _ => Index::Illegal,
    }
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
    if matches!(ty, DimType::W | DimType::Rw | DimType::Unset) {
        return Err(not_array(host, h));
    }
    let Some(offset) = offset else {
        return Err(appended(host, h));
    };
    let Some(d) = state(h) else { return Ok(None) };
    let named = matches!(class(host, h).as_str(), "Dom\\HTMLCollection");
    let v = match index_of(offset) {
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
    match index_of(offset) {
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
    if matches!(ty, DimType::W | DimType::Rw | DimType::Unset) {
        return Err(not_array(host, h));
    }
    let Some(offset) = offset else {
        return Err(appended(host, h));
    };
    let Some(d) = state(h) else { return Ok(None) };
    let v = match index_of(offset) {
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
    match index_of(offset) {
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

// ---- Dom\TokenList ----

/// A token list's element.
fn tl_element(d: &DomObject) -> *mut XmlNode {
    match &d.kind {
        Kind::TokenList(e) => e
            .borrow()
            .as_ref()
            .and_then(intern_of)
            .map_or(std::ptr::null_mut(), |d| d.node()),
        _ => std::ptr::null_mut(),
    }
}

fn tl_document(d: &DomObject) -> *mut lxn::RefObj {
    match &d.kind {
        Kind::TokenList(e) => e
            .borrow()
            .as_ref()
            .and_then(intern_of)
            .map_or(std::ptr::null_mut(), |d| d.document()),
        _ => std::ptr::null_mut(),
    }
}

fn class_attr(el: *mut XmlNode) -> Option<Vec<u8>> {
    unsafe {
        let attr = sys::xmlHasNsProp(el, c"class".as_ptr().cast(), std::ptr::null());
        if attr.is_null() || (*attr).children.is_null() {
            None
        } else {
            Some(lxn::attr_value(attr))
        }
    }
}

fn tl_set(d: &DomObject) -> Vec<Vec<u8>> {
    let el = tl_element(d);
    if el.is_null() {
        return Vec::new();
    }
    class_attr(el).map_or_else(Vec::new, |v| super::modern::ordered_set_parser(&v, false))
}

/// dom_token_list_update
fn tl_update(d: &DomObject, set: &[Vec<u8>]) {
    let el = tl_element(d);
    if el.is_null() {
        return;
    }
    lxn::invalidate_node_list_cache(tl_document(d));
    unsafe {
        let attr = sys::xmlHasNsProp(el, c"class".as_ptr().cast(), std::ptr::null());
        if attr.is_null() && set.is_empty() {
            return;
        }
        let value = c(&set.join(&b' '));
        sys::xmlSetNsProp(
            el,
            std::ptr::null_mut(),
            c"class".as_ptr().cast(),
            value.as_ptr(),
        );
    }
}

/// A new Dom\TokenList for an element's classList.
pub fn new_token_list(cx: &mut Cx, element: &Rc<DomObject>) -> Result<Value, Thrown> {
    let v = cx
        .host
        .instantiate(b"Dom\\TokenList", Vec::new())
        .ok_or_else(|| Thrown::error("Error", "Cannot create a token list"))?;
    let elv = element.base.object();
    super::ensure_intern(&v, || Kind::TokenList(std::cell::RefCell::new(elv)));
    Ok(v)
}

fn tl(this: &Value) -> Rc<DomObject> {
    intern_of(this).unwrap_or_else(|| Rc::new(DomObject::new(Kind::TokenList(Default::default()))))
}

fn tl_item(d: &DomObject, index: i64) -> Value {
    let set = tl_set(d);
    if index >= 0 && (index as usize) < set.len() {
        Value::str(set[index as usize].clone())
    } else {
        Value::Null
    }
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
    Ok(Value::Bool(tl_set(&tl(this)).contains(&token)))
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
    let mut set = tl_set(&d);
    for t in tokens {
        if !set.contains(&t) {
            set.push(t);
        }
    }
    tl_update(&d, &set);
    Ok(Value::Null)
}

pub fn tl_remove(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let tokens = validate_varargs(cx, args)?;
    let d = tl(this);
    let mut set = tl_set(&d);
    set.retain(|t| !tokens.contains(t));
    tl_update(&d, &set);
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
    let mut set = tl_set(&d);
    if let Some(pos) = set.iter().position(|t| *t == token) {
        if force != Some(true) {
            set.remove(pos);
            tl_update(&d, &set);
            return Ok(Value::Bool(false));
        }
        return Ok(Value::Bool(true));
    }
    if force != Some(false) {
        set.push(token);
        tl_update(&d, &set);
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
    let mut set = tl_set(&d);
    let Some(pos) = set.iter().position(|t| *t == token) else {
        return Ok(Value::Bool(false));
    };
    if set.contains(&new_token) {
        set.remove(pos);
    } else {
        set[pos] = new_token;
    }
    tl_update(&d, &set);
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
    Ok(Value::Int(tl_set(&tl(this)).len() as i64))
}

pub fn tl_length(_: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    Ok(Some(Value::Int(tl_set(d).len() as i64)))
}

pub fn tl_value(_: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let el = tl_element(d);
    if el.is_null() {
        return Ok(Some(Value::str("")));
    }
    Ok(Some(Value::str(class_attr(el).unwrap_or_default())))
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

/// One step of the token list's iterator: `[index, token]` or null.
pub fn tl_iterate(d: &DomObject, index: i64) -> Value {
    match tl_item(d, index) {
        Value::Null => Value::Null,
        v => {
            let mut a = rhp_values::PhpArray::new();
            a.push(Value::Int(index));
            a.push(v);
            Value::Array(a)
        }
    }
}

fn read_tl(
    host: &mut dyn Host,
    h: &ObjectHandle,
    offset: Option<&Value>,
    ty: DimType,
) -> Result<Option<Value>, Thrown> {
    if matches!(ty, DimType::W | DimType::Rw | DimType::Unset) {
        return Err(not_array(host, h));
    }
    let Some(offset) = offset else {
        return Err(Thrown::error("Error", "Cannot append to Dom\\TokenList"));
    };
    let Some(i) = tl_offset(offset) else {
        return Err(illegal(host, h, offset, false));
    };
    let d = tl(&Value::Object(h.clone()));
    Ok(match (ty, tl_item(&d, i)) {
        (DimType::Is, Value::Null) => None,
        (_, v) => Some(v),
    })
}

/// dom_token_list_offset_convert_to_long
fn tl_offset(v: &Value) -> Option<i64> {
    match super::deref(v) {
        Value::Str(s) => handle_numeric(s.as_bytes()),
        Value::Float(f) => Some(dval_to_lval(f)),
        Value::Int(n) => Some(n),
        Value::Bool(b) => Some(b as i64),
        _ => None,
    }
}

fn has_tl(
    host: &mut dyn Host,
    h: &ObjectHandle,
    offset: &Value,
    check_empty: bool,
) -> Result<bool, Thrown> {
    let Some(i) = tl_offset(offset) else {
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
