//! DOMXPath over libxml2's XPath engine, and the PHP functions it can call
//! (`php:function`, `php:functionString`, registerPhpFunctionNS), ported
//! from php-src ext/dom/xpath.c and xpath_callbacks.c. A callback reaches
//! PHP through the libxml FFI layer: what it throws is rethrown once the
//! evaluation returns.

use std::cell::RefCell;
use std::ffi::c_int;
use std::ptr;
use std::rc::Rc;

use rhp_values::{ArrayKey, PhpArray, Value};

use super::util::*;
use super::{
    arg, bytes, class_of, create_object, ensure_intern, get_obj, intern_of, lists, throw, xs, Cx,
    DomObject, Kind, INVALID_STATE_ERR,
};
use crate::ext::libxml::ffi;
use crate::ext::libxml::node as lxn;
use crate::ext::libxml::sys::{self, XmlNode, XmlXPathObject};
use crate::Thrown;

const PHP_XPATH_NS: &[u8] = b"http://php.net/xpath";

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    None,
    All,
    Set,
}

/// php_dom_xpath_callback_ns
struct CallbackNs {
    functions: Vec<(Vec<u8>, Value)>,
    mode: Mode,
}

impl CallbackNs {
    fn new() -> Self {
        CallbackNs {
            functions: Vec::new(),
            mode: Mode::None,
        }
    }

    fn set(&mut self, name: Vec<u8>, cb: Value) {
        match self.functions.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = cb,
            None => self.functions.push((name, cb)),
        }
    }
}

/// php_dom_xpath_callbacks
#[derive(Default)]
pub struct Callbacks {
    php_ns: Option<CallbackNs>,
    namespaces: Vec<(Vec<u8>, CallbackNs)>,
    /// Nodes and arrays handed to or returned by callbacks, kept alive
    /// for the context's lifetime.
    node_list: Vec<Value>,
}

/// The DOMXPath part of `dom_xpath_object`.
pub struct XPathState {
    pub ctx: *mut sys::xmlXPathContext,
    pub register_node_ns: bool,
    pub evaluation_depth: u32,
    callbacks: Callbacks,
}

impl XPathState {
    pub fn new() -> Self {
        XPathState {
            ctx: ptr::null_mut(),
            register_node_ns: true,
            evaluation_depth: 0,
            callbacks: Callbacks::default(),
        }
    }

    /// dom_xpath_objects_free_storage's context part.
    pub fn free_context(&mut self) {
        if !self.ctx.is_null() {
            unsafe { sys::xmlXPathFreeContext(self.ctx) };
            self.ctx = ptr::null_mut();
        }
        self.callbacks = Callbacks::default();
    }
}

impl Default for XPathState {
    fn default() -> Self {
        XPathState::new()
    }
}

fn state(d: &DomObject) -> &RefCell<XPathState> {
    match &d.kind {
        Kind::XPath(x) => x,
        _ => unreachable!("not a DOMXPath"),
    }
}

fn this_xpath(this: &Value) -> Rc<DomObject> {
    ensure_intern(this, || Kind::XPath(RefCell::new(XPathState::new()))).expect("a DOMXPath")
}

pub fn construct(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let docv = arg(args, 0);
    let register_node_ns = super::boolean(args, 1);
    let intern = this_xpath(this);
    if state(&intern).borrow().evaluation_depth > 0 {
        return Err(Thrown::error(
            "Error",
            format!(
                "Cannot call {}::__construct() while an XPath evaluation is in progress",
                class_of(cx.host, this)
            ),
        ));
    }
    let (docobj, docn) = get_obj(cx, &docv)?;
    let ctx = unsafe { sys::xmlXPathNewContext(docn.cast()) };
    if ctx.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    {
        let mut st = state(&intern).borrow_mut();
        if !st.ctx.is_null() {
            lxn::decrement_doc_ref(&intern.base);
            st.free_context();
        }
        unsafe {
            sys::xmlXPathRegisterFuncNS(
                ctx,
                c"functionString".as_ptr().cast(),
                c"http://php.net/xpath".as_ptr().cast(),
                Some(ext_function_string_php),
            );
            sys::xmlXPathRegisterFuncNS(
                ctx,
                c"function".as_ptr().cast(),
                c"http://php.net/xpath".as_ptr().cast(),
                Some(ext_function_object_php),
            );
            sys::rhp_xml_xpath_set_user_data(ctx, Rc::as_ptr(&intern) as *mut _);
        }
        st.ctx = ctx;
        st.register_node_ns = register_node_ns;
    }
    intern.base.document.set(docobj.document());
    lxn::increment_doc_ref(&intern.base, docn.cast());
    Ok(Value::Null)
}

pub fn register_namespace(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let prefix = c(&bytes(args, 0));
    let uri = c(&bytes(args, 1));
    let intern = this_xpath(this);
    let ctx = state(&intern).borrow().ctx;
    if ctx.is_null() {
        return Err(Thrown::error("Error", "Invalid XPath Context"));
    }
    let _ = cx;
    Ok(Value::Bool(
        unsafe { sys::xmlXPathRegisterNs(ctx, prefix.as_ptr(), uri.as_ptr()) } == 0,
    ))
}

/// dom_xpath_intern_for_doc: the object whose document a found node gets
/// (a callback may return nodes of another document).
fn intern_for_doc(xpath: &Rc<DomObject>, doc: *mut sys::XmlDoc) -> Rc<DomObject> {
    let ptr_of = |d: &DomObject| unsafe { d.document().as_ref().map(|r| r.ptr) };
    if ptr_of(xpath) == Some(doc) {
        return xpath.clone();
    }
    fn search(v: &Value, doc: *mut sys::XmlDoc) -> Option<Rc<DomObject>> {
        match v {
            Value::Object(_) => {
                let d = intern_of(v)?;
                (unsafe { d.document().as_ref() }.map(|r| r.ptr) == Some(doc)).then_some(d)
            }
            Value::Array(a) => a.iter().find_map(|(_, x)| search(x, doc)),
            _ => None,
        }
    }
    let st = state(xpath).borrow();
    st.callbacks
        .node_list
        .iter()
        .find_map(|v| search(v, doc))
        .unwrap_or_else(|| xpath.clone())
}

/// A DOMNameSpaceNode for a namespace node of an XPath result, whose
/// parent element libxml2 keeps in the copy's `next`.
fn namespace_result(cx: &mut Cx, node: *mut XmlNode, domobj: &DomObject) -> Result<Value, Thrown> {
    let original = node as *mut sys::XmlNs;
    let nsparent = unsafe { (*original).next } as *mut XmlNode;
    let parent = create_object(cx, nsparent, Some(domobj))?;
    let parent_intern = intern_of(&parent).expect("a node object");
    super::node::create_fake_namespace_decl(cx, nsparent, original, &parent_intern)
}

fn eval(cx: &mut Cx, this: &Value, args: &[Value], query: bool) -> Result<Value, Thrown> {
    let expr = bytes(args, 0);
    let context = arg(args, 1);
    let intern = this_xpath(this);
    let register_node_ns = if args.len() >= 3 {
        super::boolean(args, 2)
    } else {
        state(&intern).borrow().register_node_ns
    };
    let ctxp = state(&intern).borrow().ctx;
    if ctxp.is_null() {
        return Err(Thrown::error("Error", "Invalid XPath Context"));
    }
    let docp = unsafe { sys::rhp_xml_xpath_doc(ctxp) };
    if docp.is_null() {
        cx.warn("Invalid XPath Document Pointer")?;
        return Ok(Value::Bool(false));
    }
    let mut nodep: *mut XmlNode = ptr::null_mut();
    if !matches!(context, Value::Null) {
        nodep = get_obj(cx, &context)?.1;
    }
    if nodep.is_null() {
        nodep = unsafe { sys::xmlDocGetRootElement(docp) };
    }
    if !nodep.is_null() && unsafe { (*nodep).doc } != docp {
        return Err(Thrown::error("Error", "Node from wrong document"));
    }
    let ce = c(&expr);
    let xpathobjp = unsafe {
        let old_node = sys::rhp_xml_xpath_node(ctxp);
        let mut old_nr = 0;
        let old_ns = sys::rhp_xml_xpath_namespaces(ctxp, &mut old_nr);
        sys::rhp_xml_xpath_set_node(ctxp, nodep);
        let mut list: *mut *mut sys::XmlNs = ptr::null_mut();
        if register_node_ns && !nodep.is_null() {
            list = sys::xmlGetNsList((*nodep).doc, nodep);
            let mut count = 0;
            if !list.is_null() {
                while !(*list.add(count)).is_null() {
                    count += 1;
                }
            }
            sys::rhp_xml_xpath_set_namespaces(ctxp, list, count as c_int);
        }
        state(&intern).borrow_mut().evaluation_depth += 1;
        let r = cx.with_libxml_owned(
            || sys::xmlXPathEvalExpression(ce.as_ptr(), ctxp),
            |o| {
                if !o.is_null() {
                    sys::xmlXPathFreeObject(o)
                }
            },
        );
        state(&intern).borrow_mut().evaluation_depth -= 1;
        sys::rhp_xml_xpath_set_node(ctxp, old_node);
        sys::rhp_xml_xpath_set_namespaces(ctxp, old_ns, old_nr);
        if register_node_ns && !nodep.is_null() {
            sys::xml_free(list);
        }
        match r {
            Ok(o) => o,
            Err(t) => {
                // An exception from a callback: php returns its result
                // (if any) with the exception pending; the exception wins.
                return Err(t);
            }
        }
    };
    if xpathobjp.is_null() {
        return Ok(Value::Bool(false));
    }
    let result = (|| -> Result<Value, Thrown> {
        unsafe {
            let t = (*xpathobjp).type_;
            if query || t == sys::XPATH_NODESET {
                let mut nodes = Vec::new();
                let set = (*xpathobjp).nodesetval;
                if t == sys::XPATH_NODESET && !set.is_null() && (*set).nodeNr > 0 {
                    for i in 0..(*set).nodeNr as usize {
                        let node = *(*set).nodeTab.add(i);
                        let child = if (*node).type_ == sys::XML_NAMESPACE_DECL {
                            namespace_result(cx, node, &intern)?
                        } else {
                            let parent = intern_for_doc(&intern, (*node).doc);
                            create_object(cx, node, Some(&parent))?
                        };
                        nodes.push(child);
                    }
                }
                return lists::nodeset_list(cx, nodes);
            }
            Ok(match t {
                sys::XPATH_BOOLEAN => Value::Bool((*xpathobjp).boolval != 0),
                sys::XPATH_NUMBER => Value::Float((*xpathobjp).floatval),
                sys::XPATH_STRING => xs((*xpathobjp).stringval),
                _ => Value::Null,
            })
        }
    })();
    unsafe { sys::xmlXPathFreeObject(xpathobjp) };
    result
}

pub fn query(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    eval(cx, this, args, true)
}

pub fn evaluate(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    eval(cx, this, args, false)
}

/// php_dom_xpath_is_callback_name_valid
fn valid_callback_name(name: &[u8], ncname: bool) -> bool {
    if name.is_empty() || name.contains(&0) {
        return false;
    }
    if ncname {
        let cn = c(name);
        return unsafe { sys::xmlValidateNCName(cn.as_ptr(), 0) } == 0;
    }
    true
}

pub fn register_php_functions(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let restrict = arg(args, 0);
    let intern = this_xpath(this);
    match restrict {
        Value::Array(ht) => {
            let mut entries = Vec::new();
            for (k, entry) in ht.iter() {
                let entry = super::deref(entry);
                if let Some(why) = cx.host.check_callable(&entry)? {
                    return Err(Thrown::type_error(format!(
                        "{}(): Argument #1 ($restrict) must be an array with valid callbacks as values, {why}",
                        cx.fname
                    )));
                }
                let name = match k {
                    ArrayKey::Str(s) => s.as_bytes().to_vec(),
                    ArrayKey::Int(_) => crate::ext::libxml::io::to_string(cx.host, &entry)?,
                };
                if !valid_callback_name(&name, false) {
                    return Err(crate::ext::zpp::value_error(
                        &cx.fname,
                        1,
                        "restrict",
                        "must be an array containing valid callback names",
                    ));
                }
                entries.push((name, entry));
            }
            let mut st = state(&intern).borrow_mut();
            let ns = st.callbacks.php_ns.get_or_insert_with(CallbackNs::new);
            for (n, e) in entries {
                ns.set(n, e);
            }
            ns.mode = Mode::Set;
        }
        Value::Null => {
            let mut st = state(&intern).borrow_mut();
            st.callbacks.php_ns.get_or_insert_with(CallbackNs::new).mode = Mode::All;
        }
        v => {
            // Z_PARAM_ARRAY_HT_OR_STR_OR_NULL: scalars and Stringable
            // objects become strings, anything else is a TypeError.
            let ok = match &v {
                Value::Object(_) => cx
                    .host
                    .object_class_name(&v)
                    .and_then(|c| cx.host.resolve_class(&c))
                    .is_some_and(|id| cx.host.class_has_method(id, b"__tostring")),
                Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Str(_) => true,
                _ => false,
            };
            if !ok {
                return Err(Thrown::type_error(format!(
                    "{}(): Argument #1 ($restrict) must be of type array|string|null, {} given",
                    cx.fname,
                    crate::argparse::zval_value_name(cx.host, &v)
                )));
            }
            let name = crate::ext::libxml::io::to_string(cx.host, &v)?;
            if !valid_callback_name(&name, false) {
                return Err(crate::ext::zpp::value_error(
                    &cx.fname,
                    1,
                    "restrict",
                    "must be a valid callback name",
                ));
            }
            let cb = Value::str(name.clone());
            if let Some(why) = cx.host.check_callable(&cb)? {
                return Err(Thrown::type_error(format!(
                    "{}(): Argument #1 ($restrict) must be a callable, {why}",
                    cx.fname
                )));
            }
            let mut st = state(&intern).borrow_mut();
            let ns = st.callbacks.php_ns.get_or_insert_with(CallbackNs::new);
            ns.set(name, cb);
            ns.mode = Mode::Set;
        }
    }
    Ok(Value::Null)
}

pub fn register_php_function_ns(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let ns = bytes(args, 0);
    let name = bytes(args, 1);
    let callable = arg(args, 2);
    for (i, (s, p)) in [(&ns, "namespaceURI"), (&name, "name")].iter().enumerate() {
        if s.contains(&0) {
            return Err(crate::ext::zpp::value_error(
                &cx.fname,
                i + 1,
                p,
                "must not contain any null bytes",
            ));
        }
    }
    if let Some(why) = cx.host.check_callable(&callable)? {
        return Err(Thrown::type_error(format!(
            "{}(): Argument #3 ($callable) must be a valid callback, {why}",
            cx.fname
        )));
    }
    if ns == PHP_XPATH_NS {
        return Err(crate::ext::zpp::value_error(
            &cx.fname,
            1,
            "namespaceURI",
            "must not be \"http://php.net/xpath\" because it is reserved by PHP",
        ));
    }
    if !valid_callback_name(&name, true) {
        return Err(crate::ext::zpp::value_error(
            &cx.fname,
            2,
            "name",
            "must be a valid callback name",
        ));
    }
    let intern = this_xpath(this);
    let mut st = state(&intern).borrow_mut();
    let idx = match st.callbacks.namespaces.iter().position(|(n, _)| *n == ns) {
        Some(i) => i,
        None => {
            st.callbacks
                .namespaces
                .push((ns.clone(), CallbackNs::new()));
            st.callbacks.namespaces.len() - 1
        }
    };
    let entry = &mut st.callbacks.namespaces[idx].1;
    entry.set(name.clone(), callable);
    entry.mode = Mode::Set;
    let ctx = st.ctx;
    if !ctx.is_null() {
        let cn = c(&name);
        let cns = c(&ns);
        unsafe {
            sys::xmlXPathRegisterFuncNS(
                ctx,
                cn.as_ptr(),
                cns.as_ptr(),
                Some(ext_function_trampoline),
            )
        };
    }
    Ok(Value::Null)
}

pub fn quote(cx: &mut Cx, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let input = bytes(args, 0);
    if input.contains(&0) {
        return Err(crate::ext::zpp::value_error(
            &cx.fname,
            1,
            "str",
            "must not contain any null bytes",
        ));
    }
    if !rhp_values::mem::reserve_str(input.len() + 2) {
        return Ok(Value::str(""));
    }
    if !input.contains(&b'\'') {
        return Ok(Value::str([&b"'"[..], &input, b"'"].concat()));
    }
    if !input.contains(&b'"') {
        return Ok(Value::str([&b"\""[..], &input, b"\""].concat()));
    }
    let mut out = b"concat(".to_vec();
    let mut i = 0;
    while i < input.len() {
        let rest = &input[i..];
        let ds = rest.iter().position(|&b| b == b'\'').unwrap_or(rest.len());
        let dd = rest.iter().position(|&b| b == b'"').unwrap_or(rest.len());
        let n = ds.max(dd);
        let q = if ds > dd { b'\'' } else { b'"' };
        out.push(q);
        out.extend_from_slice(&rest[..n]);
        out.push(q);
        out.push(b',');
        i += n;
    }
    let last = out.len() - 1;
    out[last] = b')';
    Ok(Value::str(out))
}

// ---- the callbacks libxml2 calls ----

#[derive(Clone, Copy, PartialEq)]
enum NodesetMode {
    ToString,
    ToNodeset,
}

/// The DOMXPath object of a parser context (dom_xpath_ext_fetch_intern).
fn fetch_intern(ctxt: *mut sys::xmlXPathParserContext) -> Option<Rc<DomObject>> {
    unsafe {
        let ctx = sys::rhp_xml_xpath_parser_context(ctxt);
        let p = sys::rhp_xml_xpath_user_data(ctx) as *const DomObject;
        if p.is_null() {
            sys::rhp_xml_generic_error_msg(
                c"xmlExtFunctionTest: failed to get the internal object\n".as_ptr(),
            );
            return None;
        }
        // The evaluating builtin holds a reference to the payload.
        Rc::increment_strong_count(p);
        Some(Rc::from_raw(p))
    }
}

fn pop(ctxt: *mut sys::xmlXPathParserContext) -> *mut XmlXPathObject {
    unsafe { sys::rhp_xml_xpath_value_pop(ctxt) }
}

fn push(ctxt: *mut sys::xmlXPathParserContext, obj: *mut XmlXPathObject) {
    unsafe { sys::rhp_xml_xpath_value_push(ctxt, obj) }
}

/// php_dom_xpath_callbacks_clean_argument_stack
fn clean_argument_stack(ctxt: *mut sys::xmlXPathParserContext, n: i32) {
    for _ in 0..n {
        unsafe { sys::xmlXPathFreeObject(pop(ctxt)) };
    }
}

/// php_dom_xpath_callback_fetch_args
fn fetch_args(
    cx: &mut Cx,
    ctxt: *mut sys::xmlXPathParserContext,
    count: usize,
    mode: NodesetMode,
    intern: &Rc<DomObject>,
) -> Result<Vec<Value>, Thrown> {
    let mut params = vec![Value::Null; count];
    for slot in params.iter_mut().rev() {
        let obj = pop(ctxt);
        if obj.is_null() {
            continue;
        }
        let v = unsafe {
            match (*obj).type_ {
                sys::XPATH_STRING => xs((*obj).stringval),
                sys::XPATH_BOOLEAN => Value::Bool((*obj).boolval != 0),
                sys::XPATH_NUMBER => Value::Float((*obj).floatval),
                sys::XPATH_NODESET if mode == NodesetMode::ToNodeset => {
                    let set = (*obj).nodesetval;
                    let mut arr = PhpArray::new();
                    if !set.is_null() {
                        for i in 0..(*set).nodeNr.max(0) as usize {
                            let node = *(*set).nodeTab.add(i);
                            let child = if (*node).type_ == sys::XML_NAMESPACE_DECL {
                                namespace_result(cx, node, intern)
                            } else {
                                create_object(cx, node, Some(intern))
                            };
                            match child {
                                Ok(ch) => arr.push(ch),
                                Err(e) => {
                                    sys::xmlXPathFreeObject(obj);
                                    return Err(e);
                                }
                            }
                        }
                    }
                    Value::Array(arr)
                }
                _ => {
                    Value::str(sys::take_string(sys::xmlXPathCastToString(obj)).unwrap_or_default())
                }
            }
        };
        unsafe { sys::xmlXPathFreeObject(obj) };
        *slot = v;
    }
    Ok(params)
}

/// php_dom_xpath_callback_cleanup_args: objects and arrays stay alive.
fn keep_args(intern: &DomObject, params: Vec<Value>) {
    let mut st = state(intern).borrow_mut();
    for p in params {
        if matches!(p, Value::Object(_) | Value::Array(_)) {
            st.callbacks.node_list.push(p);
        }
    }
}

/// php_dom_xpath_callback_dispatch
fn dispatch(
    cx: &mut Cx,
    intern: &Rc<DomObject>,
    ctxt: *mut sys::xmlXPathParserContext,
    ns: Option<&[u8]>,
    params: &[Value],
    function: &[u8],
) -> Result<(), Thrown> {
    let (mode, cb) = {
        let st = state(intern).borrow();
        let cbns = match ns {
            None => st.callbacks.php_ns.as_ref(),
            Some(n) => st
                .callbacks
                .namespaces
                .iter()
                .find(|(k, _)| k.as_slice() == n)
                .map(|(_, v)| v),
        };
        match cbns {
            None => (Mode::None, None),
            Some(c) => (
                c.mode,
                c.functions
                    .iter()
                    .find(|(k, _)| k.as_slice() == function)
                    .map(|(_, v)| v.clone()),
            ),
        }
    };
    let retval = match mode {
        Mode::None => return Err(Thrown::error("Error", "No callbacks were registered")),
        Mode::All => {
            let f = Value::str(function.to_vec());
            if let Some(why) = cx.host.check_callable(&f)? {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Invalid callback {}, {why}",
                        String::from_utf8_lossy(function)
                    ),
                ));
            }
            cx.host.call_callable(&f, params)?
        }
        Mode::Set => match cb {
            Some(cb) => cx.host.call_callable(&cb, params)?,
            None => {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "No callback handler \"{}\" registered",
                        String::from_utf8_lossy(function)
                    ),
                ))
            }
        },
    };
    let obj = match &retval {
        Value::Object(_) => {
            let is_node = cx
                .host
                .object_class_name(&retval)
                .is_some_and(|c| cx.host.reflect_is_a(&c, b"DOMNode"));
            if !is_node {
                return Err(Thrown::type_error(
                    "Only objects that are instances of DOM nodes can be converted to an XPath expression",
                ));
            }
            state(intern)
                .borrow_mut()
                .callbacks
                .node_list
                .push(retval.clone());
            let n = intern_of(&retval).map_or(ptr::null_mut(), |d| d.node());
            unsafe { sys::xmlXPathNewNodeSet(n) }
        }
        Value::Bool(b) => unsafe { sys::xmlXPathNewBoolean(*b as c_int) },
        v => {
            let s = crate::ext::libxml::io::to_string(cx.host, v)?;
            let cs = c(&s);
            unsafe { sys::xmlXPathNewString(cs.as_ptr()) }
        }
    };
    push(ctxt, obj);
    Ok(())
}

/// Run one PHP function call for libxml2: on failure (or once something
/// was thrown) a "" sentinel keeps the stack balanced.
fn call_function(
    ctxt: *mut sys::xmlXPathParserContext,
    nargs: c_int,
    mode: NodesetMode,
    custom: bool,
) {
    ffi::guard((), || {
        let Some(intern) = fetch_intern(ctxt) else {
            clean_argument_stack(ctxt, nargs);
            return;
        };
        if ffi::aborted() {
            clean_argument_stack(ctxt, nargs);
            push(ctxt, unsafe { sys::xmlXPathNewString(c"".as_ptr().cast()) });
            return;
        }
        let ok = ffi::call_host(|host| {
            let mut cx = Cx {
                host,
                fname: ffi::fname(),
                quiet: false,
                lazy_fname: false,
            };
            if custom {
                let (function, uri) = unsafe {
                    let ctx = sys::rhp_xml_xpath_parser_context(ctxt);
                    let mut uri = ptr::null();
                    let f = sys::rhp_xml_xpath_function(ctx, &mut uri);
                    (sys::cstr(f).to_vec(), sys::cstr(uri).to_vec())
                };
                let params = fetch_args(&mut cx, ctxt, nargs.max(0) as usize, mode, &intern)?;
                let r = dispatch(&mut cx, &intern, ctxt, Some(&uri), &params, &function);
                keep_args(&intern, params);
                r
            } else {
                if nargs == 0 {
                    return Err(Thrown::error(
                        "Error",
                        "Function name must be passed as the first argument",
                    ));
                }
                let params = fetch_args(&mut cx, ctxt, (nargs - 1) as usize, mode, &intern)?;
                let obj = pop(ctxt);
                let r = unsafe {
                    if obj.is_null() || (*obj).stringval.is_null() {
                        Err(Thrown::type_error("Handler name must be a string"))
                    } else {
                        let name = sys::cstr((*obj).stringval).to_vec();
                        dispatch(&mut cx, &intern, ctxt, None, &params, &name)
                    }
                };
                unsafe { sys::xmlXPathFreeObject(obj) };
                keep_args(&intern, params);
                r
            }
        });
        if ok.is_none() {
            push(ctxt, unsafe { sys::xmlXPathNewString(c"".as_ptr().cast()) });
        }
    })
}

unsafe extern "C" fn ext_function_string_php(ctxt: *mut sys::xmlXPathParserContext, nargs: c_int) {
    call_function(ctxt, nargs, NodesetMode::ToString, false);
}

unsafe extern "C" fn ext_function_object_php(ctxt: *mut sys::xmlXPathParserContext, nargs: c_int) {
    call_function(ctxt, nargs, NodesetMode::ToNodeset, false);
}

unsafe extern "C" fn ext_function_trampoline(ctxt: *mut sys::xmlXPathParserContext, nargs: c_int) {
    call_function(ctxt, nargs, NodesetMode::ToNodeset, true);
}
