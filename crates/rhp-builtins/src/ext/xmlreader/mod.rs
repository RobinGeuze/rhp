//! The `xmlreader` extension on libxml2's xmlTextReader, ported from
//! php-src ext/xmlreader/php_xmlreader.c (PHP License 3.01).
//!
//! `XMLReader` is PHP source (`PRELUDE_PHP`): methods call
//! `__xr("XMLReader::method", $this, ...)`, the virtual properties go
//! through the magic accessors to `__xr_prop`. A reader's native state is
//! its payload ([`Reader`]).

use std::cell::Cell;
use std::ffi::{c_char, c_int, c_void};
use std::ptr;
use std::rc::Rc;

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, ObjectHandle, PhpArray, PhpStr, Value};

use crate::ext::libxml::ffi;
use crate::ext::libxml::sys::{self, xmlTextReader};
use crate::zend::object_handlers;
use crate::zend::object_handlers::{ClassHandlers, CloneObj, ObjectHandlers, PropPurpose};
use crate::{Host, Thrown};

/// php's `xmlreader_object`.
pub(crate) struct Reader {
    ptr: Cell<*mut xmlTextReader>,
    input: Cell<*mut sys::xmlParserInputBuffer>,
    schema: Cell<*mut sys::xmlRelaxNG>,
}

impl Reader {
    fn new() -> Reader {
        Reader {
            ptr: Cell::new(ptr::null_mut()),
            input: Cell::new(ptr::null_mut()),
            schema: Cell::new(ptr::null_mut()),
        }
    }

    /// xmlreader_free_resources
    fn free_resources(&self) {
        unsafe {
            let input = self.input.replace(ptr::null_mut());
            if !input.is_null() {
                sys::xmlFreeParserInputBuffer(input);
            }
            let r = self.ptr.replace(ptr::null_mut());
            if !r.is_null() {
                sys::xmlFreeTextReader(r);
            }
            let s = self.schema.replace(ptr::null_mut());
            if !s.is_null() {
                sys::xmlRelaxNGFree(s);
            }
        }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.free_resources();
    }
}

static XMLREADER_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(get_properties_for),
    clone_obj: CloneObj::None,
    ..ObjectHandlers::STD
};

pub(crate) static CLASSES: &[ClassHandlers] = &[ClassHandlers {
    class: "XMLReader",
    handlers: &XMLREADER_HANDLERS,
    create_object: true,
}];

/// The payload of `this`, created on first use.
fn intern(v: &Value) -> Option<Rc<Reader>> {
    if let Some(r) = object_handlers::payload::<Reader>(v) {
        return Some(r);
    }
    let h = object_handlers::object_of(v)?;
    let r = Rc::new(Reader::new());
    object_handlers::set_payload(&h, r.clone());
    Some(r)
}

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

fn cs(s: &[u8]) -> Vec<u8> {
    sys::cbytes(s)
}

/// A string the reader allocated, freed after copying.
fn take(p: *mut u8) -> Option<Value> {
    unsafe { sys::take_string(p) }.map(Value::str)
}

struct Cx<'h> {
    host: &'h mut dyn Host,
    fname: String,
}

impl Cx<'_> {
    fn warn(&mut self, msg: &str) -> Result<(), Thrown> {
        crate::warn(self.host, &self.fname, msg)
    }

    fn notice(&mut self, msg: &str) -> Result<(), Thrown> {
        crate::notice(self.host, &self.fname, msg)
    }

    fn with_libxml<R>(&mut self, f: impl FnOnce() -> R) -> Result<R, Thrown> {
        ffi::with_host(self.host, &self.fname, f)
    }

    /// [`Self::with_libxml`] for a call whose result owns memory.
    fn with_libxml_owned<R>(
        &mut self,
        f: impl FnOnce() -> R,
        free: impl FnOnce(R),
    ) -> Result<R, Thrown> {
        match ffi::with_host_keep(self.host, &self.fname, f) {
            (r, Some(t)) => {
                free(r);
                Err(t)
            }
            (r, None) => Ok(r),
        }
    }

    fn arg_error(&self, class: &'static str, n: usize, pname: &str, msg: &str) -> Thrown {
        Thrown::Error {
            class,
            message: format!("{}(): Argument #{n} (${pname}) {msg}", self.fname),
        }
    }
}

fn free_string(p: *mut u8) {
    unsafe { sys::xml_free(p) }
}

// ---- properties ----

/// One of php's `xmlreader_prop_handler`s.
#[derive(Clone, Copy)]
enum Prop {
    Int(unsafe extern "C" fn(*mut xmlTextReader) -> c_int),
    Bool(unsafe extern "C" fn(*mut xmlTextReader) -> c_int),
    Str(unsafe extern "C" fn(*mut xmlTextReader) -> *const u8),
}

/// The property handlers, in registration order.
static PROPS: &[(&str, Prop)] = &[
    (
        "attributeCount",
        Prop::Int(sys::xmlTextReaderAttributeCount),
    ),
    ("baseURI", Prop::Str(sys::xmlTextReaderConstBaseUri)),
    ("depth", Prop::Int(sys::xmlTextReaderDepth)),
    ("hasAttributes", Prop::Bool(sys::xmlTextReaderHasAttributes)),
    ("hasValue", Prop::Bool(sys::xmlTextReaderHasValue)),
    ("isDefault", Prop::Bool(sys::xmlTextReaderIsDefault)),
    (
        "isEmptyElement",
        Prop::Bool(sys::xmlTextReaderIsEmptyElement),
    ),
    ("localName", Prop::Str(sys::xmlTextReaderConstLocalName)),
    ("name", Prop::Str(sys::xmlTextReaderConstName)),
    (
        "namespaceURI",
        Prop::Str(sys::xmlTextReaderConstNamespaceUri),
    ),
    ("nodeType", Prop::Int(sys::xmlTextReaderNodeType)),
    ("prefix", Prop::Str(sys::xmlTextReaderConstPrefix)),
    ("value", Prop::Str(sys::xmlTextReaderConstValue)),
    ("xmlLang", Prop::Str(sys::xmlTextReaderConstXmlLang)),
];

/// Whether `class` is (or extends) XMLReader and `name` one of its declared
/// properties.
pub(crate) fn has_declared_property(host: &dyn Host, class: u32, name: &[u8]) -> bool {
    prop_handler(name).is_some()
        && host
            .class_name(class)
            .is_some_and(|c| host.reflect_is_a(&c, b"XMLReader"))
}

fn prop_handler(name: &[u8]) -> Option<Prop> {
    PROPS
        .iter()
        .find(|(n, _)| n.as_bytes() == name)
        .map(|(_, p)| *p)
}

/// xmlreader_property_reader
fn property_reader(r: &Reader, hnd: Prop) -> Result<Value, Thrown> {
    let p = r.ptr.get();
    Ok(match hnd {
        Prop::Str(f) => {
            let s = if p.is_null() {
                ptr::null()
            } else {
                unsafe { f(p) }
            };
            Value::str(unsafe { sys::cstr(s) })
        }
        Prop::Int(f) | Prop::Bool(f) => {
            let mut n = 0;
            if !p.is_null() {
                n = unsafe { f(p) };
                if n == -1 {
                    return Err(Thrown::error(
                        "Error",
                        "Failed to read property because no XML data has been read yet",
                    ));
                }
            }
            if matches!(hnd, Prop::Bool(_)) {
                Value::Bool(n != 0)
            } else {
                Value::Int(n as i64)
            }
        }
    })
}

/// "not a virtual property": the prelude then uses the object's own.
fn not_virtual() -> Value {
    Value::Array(PhpArray::new())
}

/// `__xr_prop($op, $this, $name [, $value])`: php's read_property (0),
/// write_property (1), has_property (2, 3 for empty) and unset_property
/// (4) for the virtual properties.
fn fn_xr_prop(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = conv::to_int(&arg(args, 0));
    let this = arg(args, 1);
    let name = bytes(args, 2);
    let Some(hnd) = prop_handler(&name) else {
        return Ok(not_virtual());
    };
    let class =
        || String::from_utf8_lossy(&host.object_class_name(&this).unwrap_or_default()).into_owned();
    match op {
        0 => {
            let r = intern(&this).expect("an XMLReader");
            property_reader(&r, hnd)
        }
        1 => Err(Thrown::error(
            "Error",
            format!(
                "Cannot modify readonly property {}::${}",
                class(),
                String::from_utf8_lossy(&name)
            ),
        )),
        2 | 3 => {
            let r = intern(&this).expect("an XMLReader");
            let v = property_reader(&r, hnd)?;
            Ok(Value::Bool(if op == 3 {
                conv::to_bool(&v)
            } else {
                !matches!(v, Value::Null)
            }))
        }
        _ => Err(Thrown::error(
            "Error",
            format!(
                "Cannot unset {}::${}",
                class(),
                String::from_utf8_lossy(&name)
            ),
        )),
    }
}

/// xmlreader_get_debug_info: the properties, then every virtual property
/// that reads.
fn get_properties_for(
    _: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    if purpose != PropPurpose::Debug {
        return Ok(None);
    }
    let v = Value::Object(h.clone());
    let mut out = PhpArray::new();
    for (k, val) in h.borrow().properties.iter() {
        out.insert(ArrayKey::Str(k.clone()), val.clone());
    }
    let r = intern(&v).expect("an XMLReader");
    for (name, hnd) in PROPS {
        if let Ok(val) = property_reader(&r, *hnd) {
            out.insert(ArrayKey::Str(PhpStr::from_bytes(name.as_bytes())), val);
        }
    }
    Ok(Some(out))
}

// ---- methods ----

type Method = fn(&mut Cx, &Rc<Reader>, &Value, &[Value]) -> Result<Value, Thrown>;

fn method(name: &[u8]) -> Option<Method> {
    Some(match name {
        b"XMLReader::close" => m_close,
        b"XMLReader::getAttribute" => m_get_attribute,
        b"XMLReader::getAttributeNo" => m_get_attribute_no,
        b"XMLReader::getAttributeNs" => m_get_attribute_ns,
        b"XMLReader::getParserProperty" => m_get_parser_property,
        b"XMLReader::isValid" => m_is_valid,
        b"XMLReader::lookupNamespace" => m_lookup_namespace,
        b"XMLReader::moveToAttribute" => m_move_to_attribute,
        b"XMLReader::moveToAttributeNo" => m_move_to_attribute_no,
        b"XMLReader::moveToAttributeNs" => m_move_to_attribute_ns,
        b"XMLReader::moveToElement" => m_move_to_element,
        b"XMLReader::moveToFirstAttribute" => m_move_to_first_attribute,
        b"XMLReader::moveToNextAttribute" => m_move_to_next_attribute,
        b"XMLReader::read" => m_read,
        b"XMLReader::next" => m_next,
        b"XMLReader::open" => m_open,
        b"XMLReader::fromUri" => m_from_uri,
        b"XMLReader::fromStream" => m_from_stream,
        b"XMLReader::readInnerXml" => m_read_inner_xml,
        b"XMLReader::readOuterXml" => m_read_outer_xml,
        b"XMLReader::readString" => m_read_string,
        b"XMLReader::setSchema" => m_set_schema,
        b"XMLReader::setParserProperty" => m_set_parser_property,
        b"XMLReader::setRelaxNGSchema" => m_set_relaxng_schema,
        b"XMLReader::setRelaxNGSchemaSource" => m_set_relaxng_schema_source,
        b"XMLReader::XML" => m_xml,
        b"XMLReader::fromString" => m_from_string,
        b"XMLReader::expand" => m_expand,
        _ => return None,
    })
}

fn m_close(_: &mut Cx, r: &Rc<Reader>, _: &Value, _: &[Value]) -> Result<Value, Thrown> {
    r.free_resources();
    Ok(Value::Bool(true))
}

/// php_xmlreader_string_arg
fn string_arg(
    cx: &mut Cx,
    r: &Reader,
    args: &[Value],
    f: unsafe extern "C" fn(*mut xmlTextReader, *const u8) -> *mut u8,
) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    if name.is_empty() {
        return Err(cx.arg_error("ValueError", 1, "name", "must not be empty"));
    }
    let p = r.ptr.get();
    if p.is_null() {
        return Ok(Value::Null);
    }
    let cn = cs(&name);
    let s = cx.with_libxml_owned(|| unsafe { f(p, cn.as_ptr()) }, free_string)?;
    Ok(take(s).unwrap_or(Value::Null))
}

/// php_xmlreader_no_arg
fn no_arg(
    cx: &mut Cx,
    r: &Reader,
    f: unsafe extern "C" fn(*mut xmlTextReader) -> c_int,
) -> Result<Value, Thrown> {
    let p = r.ptr.get();
    if p.is_null() {
        return Ok(Value::Bool(false));
    }
    let v = cx.with_libxml(|| unsafe { f(p) })?;
    Ok(Value::Bool(v == 1))
}

/// php_xmlreader_no_arg_string
fn no_arg_string(
    cx: &mut Cx,
    r: &Reader,
    f: unsafe extern "C" fn(*mut xmlTextReader) -> *mut u8,
) -> Result<Value, Thrown> {
    let p = r.ptr.get();
    if p.is_null() {
        return Ok(Value::str(""));
    }
    let s = cx.with_libxml_owned(|| unsafe { f(p) }, free_string)?;
    Ok(take(s).unwrap_or_else(|| Value::str("")))
}

fn m_get_attribute(
    cx: &mut Cx,
    r: &Rc<Reader>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    string_arg(cx, r, args, sys::xmlTextReaderGetAttribute)
}

fn m_get_attribute_no(
    cx: &mut Cx,
    r: &Rc<Reader>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let pos = conv::to_int(&arg(args, 0));
    let p = r.ptr.get();
    if p.is_null() {
        return Ok(Value::Null);
    }
    let s = cx.with_libxml_owned(
        || unsafe { sys::xmlTextReaderGetAttributeNo(p, pos as c_int) },
        free_string,
    )?;
    Ok(take(s).unwrap_or(Value::Null))
}

fn m_get_attribute_ns(
    cx: &mut Cx,
    r: &Rc<Reader>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let ns = bytes(args, 1);
    if name.is_empty() {
        return Err(cx.arg_error("ValueError", 1, "name", "must not be empty"));
    }
    if ns.is_empty() {
        return Err(cx.arg_error("ValueError", 2, "namespace", "must not be empty"));
    }
    let p = r.ptr.get();
    if p.is_null() {
        return Ok(Value::Null);
    }
    let (cn, cu) = (cs(&name), cs(&ns));
    let s = cx.with_libxml_owned(
        || unsafe { sys::xmlTextReaderGetAttributeNs(p, cn.as_ptr(), cu.as_ptr()) },
        free_string,
    )?;
    Ok(take(s).unwrap_or(Value::Null))
}

fn m_get_parser_property(
    cx: &mut Cx,
    r: &Rc<Reader>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let prop = conv::to_int(&arg(args, 0));
    let p = r.ptr.get();
    if p.is_null() {
        return Err(Thrown::error(
            "Error",
            "Cannot access parser properties before loading data",
        ));
    }
    let v = unsafe { sys::xmlTextReaderGetParserProp(p, prop as c_int) };
    if v == -1 {
        return Err(cx.arg_error(
            "ValueError",
            1,
            "property",
            "must be a valid parser property",
        ));
    }
    Ok(Value::Bool(v != 0))
}

fn m_is_valid(cx: &mut Cx, r: &Rc<Reader>, _: &Value, _: &[Value]) -> Result<Value, Thrown> {
    no_arg(cx, r, sys::xmlTextReaderIsValid)
}

fn m_lookup_namespace(
    cx: &mut Cx,
    r: &Rc<Reader>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    if name.is_empty() {
        return Err(cx.arg_error("ValueError", 1, "prefix", "must not be empty"));
    }
    let p = r.ptr.get();
    if p.is_null() {
        return Ok(Value::Null);
    }
    let cn = cs(&name);
    let s = cx.with_libxml_owned(
        || unsafe { sys::xmlTextReaderLookupNamespace(p, cn.as_ptr()) },
        free_string,
    )?;
    Ok(take(s).unwrap_or(Value::Null))
}

fn m_move_to_attribute(
    cx: &mut Cx,
    r: &Rc<Reader>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    if name.is_empty() {
        return Err(cx.arg_error("ValueError", 1, "name", "must not be empty"));
    }
    let p = r.ptr.get();
    if p.is_null() {
        return Ok(Value::Bool(false));
    }
    let cn = cs(&name);
    let v = cx.with_libxml(|| unsafe { sys::xmlTextReaderMoveToAttribute(p, cn.as_ptr()) })?;
    Ok(Value::Bool(v == 1))
}

fn m_move_to_attribute_no(
    cx: &mut Cx,
    r: &Rc<Reader>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let pos = conv::to_int(&arg(args, 0));
    let p = r.ptr.get();
    if p.is_null() {
        return Ok(Value::Bool(false));
    }
    let v = cx.with_libxml(|| unsafe { sys::xmlTextReaderMoveToAttributeNo(p, pos as c_int) })?;
    Ok(Value::Bool(v == 1))
}

fn m_move_to_attribute_ns(
    cx: &mut Cx,
    r: &Rc<Reader>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let ns = bytes(args, 1);
    if name.is_empty() {
        return Err(cx.arg_error("ValueError", 1, "name", "must not be empty"));
    }
    if ns.is_empty() {
        return Err(cx.arg_error("ValueError", 2, "namespace", "must not be empty"));
    }
    let p = r.ptr.get();
    if p.is_null() {
        return Ok(Value::Bool(false));
    }
    let (cn, cu) = (cs(&name), cs(&ns));
    let v = cx.with_libxml(|| unsafe {
        sys::xmlTextReaderMoveToAttributeNs(p, cn.as_ptr(), cu.as_ptr())
    })?;
    Ok(Value::Bool(v == 1))
}

fn m_move_to_element(cx: &mut Cx, r: &Rc<Reader>, _: &Value, _: &[Value]) -> Result<Value, Thrown> {
    no_arg(cx, r, sys::xmlTextReaderMoveToElement)
}

fn m_move_to_first_attribute(
    cx: &mut Cx,
    r: &Rc<Reader>,
    _: &Value,
    _: &[Value],
) -> Result<Value, Thrown> {
    no_arg(cx, r, sys::xmlTextReaderMoveToFirstAttribute)
}

fn m_move_to_next_attribute(
    cx: &mut Cx,
    r: &Rc<Reader>,
    _: &Value,
    _: &[Value],
) -> Result<Value, Thrown> {
    no_arg(cx, r, sys::xmlTextReaderMoveToNextAttribute)
}

fn not_loaded() -> Thrown {
    Thrown::error("Error", "Data must be loaded before reading")
}

fn m_read(cx: &mut Cx, r: &Rc<Reader>, _: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let p = r.ptr.get();
    if p.is_null() {
        return Err(not_loaded());
    }
    let v = cx.with_libxml(|| unsafe { sys::xmlTextReaderRead(p) })?;
    Ok(Value::Bool(v == 1))
}

fn m_next(cx: &mut Cx, r: &Rc<Reader>, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = opt_bytes(args, 0);
    let p = r.ptr.get();
    if p.is_null() {
        return Err(not_loaded());
    }
    let mut v = cx.with_libxml(|| unsafe { sys::xmlTextReaderNext(p) })?;
    if let Some(n) = &name {
        while v == 1 {
            let local = unsafe { sys::cstr(sys::xmlTextReaderConstLocalName(p)) };
            let want = &n[..n.iter().position(|&b| b == 0).unwrap_or(n.len())];
            if local == want {
                return Ok(Value::Bool(true));
            }
            v = cx.with_libxml(|| unsafe { sys::xmlTextReaderNext(p) })?;
        }
    }
    Ok(Value::Bool(v == 1))
}

/// xmlreader_valid_encoding
fn valid_encoding(encoding: Option<&[u8]>) -> bool {
    let Some(e) = encoding else {
        return true;
    };
    let ce = cs(e);
    unsafe {
        let h = sys::xmlFindCharEncodingHandler(ce.as_ptr().cast());
        if h.is_null() {
            return false;
        }
        sys::xmlCharEncCloseFunc(h);
    }
    true
}

/// `p` zpp: no NUL bytes.
fn check_path(cx: &Cx, n: usize, pname: &str, s: &[u8]) -> Result<(), Thrown> {
    if s.contains(&0) {
        return Err(cx.arg_error("ValueError", n, pname, "must not contain any null bytes"));
    }
    Ok(())
}

/// The new reader for the static `open`/`XML`/`from*`: an object of
/// `class`, constructed.
fn new_reader(cx: &mut Cx, class: &[u8]) -> Result<(Value, Rc<Reader>), Thrown> {
    let v = cx.host.reflect_new_instance(class, Vec::new(), true)?;
    let r = intern(&v).expect("an XMLReader");
    Ok((v, r))
}

/// xml_reader_from_uri: `this` is null for the static form.
fn from_uri(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
    class: Option<&[u8]>,
    use_exceptions: bool,
) -> Result<Value, Thrown> {
    let source = bytes(args, 0);
    let encoding = opt_bytes(args, 1);
    let options = conv::to_int(&arg(args, 2));
    check_path(cx, 1, "uri", &source)?;
    if let Some(e) = &encoding {
        check_path(cx, 2, "encoding", e)?;
    }
    let target = if class.is_none() {
        let r = intern(this).expect("an XMLReader");
        r.free_resources();
        Some(r)
    } else {
        None
    };
    if source.is_empty() {
        return Err(cx.arg_error("ValueError", 1, "uri", "must not be empty"));
    }
    if !valid_encoding(encoding.as_deref()) {
        return Err(cx.arg_error(
            "ValueError",
            2,
            "encoding",
            "must be a valid character encoding",
        ));
    }
    let mut reader: *mut xmlTextReader = ptr::null_mut();
    if let Some(file) = crate::ext::dom::valid_file_path(&source) {
        let cf = cs(&file);
        let ce = encoding.as_deref().map(cs);
        reader = cx.with_libxml_owned(
            || unsafe {
                let mut saved = [0 as c_int; 6];
                sys::rhp_xml_sanitize_globals(saved.as_mut_ptr());
                let r = sys::xmlReaderForFile(
                    cf.as_ptr().cast(),
                    ce.as_ref().map_or(ptr::null(), |e| e.as_ptr().cast()),
                    options as c_int,
                );
                sys::rhp_xml_restore_globals(saved.as_ptr());
                r
            },
            |r| unsafe {
                if !r.is_null() {
                    sys::xmlFreeTextReader(r)
                }
            },
        )?;
    }
    if reader.is_null() {
        if use_exceptions {
            return Err(Thrown::error("Error", "Unable to open source data"));
        }
        cx.warn("Unable to open source data")?;
        return Ok(Value::Bool(false));
    }
    match (target, class) {
        (Some(r), _) => {
            r.ptr.set(reader);
            Ok(Value::Bool(true))
        }
        (None, Some(class)) => match new_reader(cx, class) {
            Ok((v, r)) => {
                r.ptr.set(reader);
                Ok(v)
            }
            Err(t) => {
                unsafe { sys::xmlFreeTextReader(reader) };
                Err(t)
            }
        },
        (None, None) => unreachable!(),
    }
}

/// The class a static form instantiates: XMLReader for `open`/`XML`, the
/// called class for the `from*` factories.
fn static_class(args: &[Value]) -> Vec<u8> {
    bytes(args, args.len().saturating_sub(1))
}

fn m_open(cx: &mut Cx, _: &Rc<Reader>, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    if matches!(this, Value::Object(_)) {
        from_uri(cx, this, args, None, false)
    } else {
        from_uri(cx, this, args, Some(b"XMLReader"), false)
    }
}

fn m_from_uri(cx: &mut Cx, _: &Rc<Reader>, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let class = static_class(args);
    from_uri(
        cx,
        this,
        &args[..args.len().saturating_sub(1)],
        Some(&class),
        true,
    )
}

/// A php stream as xmlReaderForIO's context: the reader holds a reference
/// to the resource until it closes its input.
struct StreamCtx {
    res: Value,
}

unsafe extern "C" fn stream_read(ctx: *mut c_void, buf: *mut c_char, len: c_int) -> c_int {
    ffi::guard(-1, || {
        let s = unsafe { &*(ctx as *const StreamCtx) };
        let want = len.max(0) as usize;
        match ffi::call_host(|h| Ok(h.stream_read(&s.res, want as i64))) {
            Some(Some(data)) => {
                let n = data.len().min(want);
                unsafe { ptr::copy_nonoverlapping(data.as_ptr(), buf.cast::<u8>(), n) };
                n as c_int
            }
            _ => -1,
        }
    })
}

unsafe extern "C" fn stream_close(ctx: *mut c_void) -> c_int {
    // Drops the reference; the stream stays open for its other holders.
    drop(unsafe { Box::from_raw(ctx as *mut StreamCtx) });
    0
}

fn m_from_stream(cx: &mut Cx, _: &Rc<Reader>, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let class = static_class(args);
    let args = &args[..args.len().saturating_sub(1)];
    let stream = arg(args, 0);
    let encoding = opt_bytes(args, 1);
    let flags = conv::to_int(&arg(args, 2));
    let uri = opt_bytes(args, 3);
    if let Some(e) = &encoding {
        check_path(cx, 2, "encoding", e)?;
    }
    if let Some(u) = &uri {
        check_path(cx, 4, "documentUri", u)?;
    }
    match &stream {
        Value::Resource(r) if r.type_name == "stream" && !cx.host.resource_closed(&stream) => {}
        Value::Resource(_) => {
            return Err(Thrown::type_error(format!(
                "{}(): supplied resource is not a valid stream resource",
                cx.fname
            )));
        }
        other => {
            return Err(Thrown::type_error(format!(
                "{}(): Argument #1 ($stream) must be of type resource, {} given",
                cx.fname,
                crate::zend::zpp::zval_value_name(cx.host, other)
            )));
        }
    }
    if !valid_encoding(encoding.as_deref()) {
        return Err(cx.arg_error(
            "ValueError",
            2,
            "encoding",
            "must be a valid character encoding",
        ));
    }
    let ctx = Box::into_raw(Box::new(StreamCtx { res: stream })).cast::<c_void>();
    let ce = encoding.as_deref().map(cs);
    let cu = uri.as_deref().map(cs);
    let reader = cx.with_libxml_owned(
        || unsafe {
            let mut saved = [0 as c_int; 6];
            sys::rhp_xml_sanitize_globals(saved.as_mut_ptr());
            let r = sys::xmlReaderForIO(
                Some(stream_read),
                Some(stream_close),
                ctx,
                cu.as_ref().map_or(ptr::null(), |u| u.as_ptr().cast()),
                ce.as_ref().map_or(ptr::null(), |e| e.as_ptr().cast()),
                flags as c_int,
            );
            sys::rhp_xml_restore_globals(saved.as_ptr());
            r
        },
        |r| unsafe {
            if !r.is_null() {
                sys::xmlFreeTextReader(r)
            }
        },
    )?;
    if reader.is_null() {
        return Err(Thrown::error("Error", "Could not construct libxml reader"));
    }
    match new_reader(cx, &class) {
        Ok((v, r)) => {
            r.ptr.set(reader);
            Ok(v)
        }
        Err(t) => {
            unsafe { sys::xmlFreeTextReader(reader) };
            Err(t)
        }
    }
}

fn m_read_inner_xml(cx: &mut Cx, r: &Rc<Reader>, _: &Value, _: &[Value]) -> Result<Value, Thrown> {
    no_arg_string(cx, r, sys::xmlTextReaderReadInnerXml)
}

fn m_read_outer_xml(cx: &mut Cx, r: &Rc<Reader>, _: &Value, _: &[Value]) -> Result<Value, Thrown> {
    no_arg_string(cx, r, sys::xmlTextReaderReadOuterXml)
}

fn m_read_string(cx: &mut Cx, r: &Rc<Reader>, _: &Value, _: &[Value]) -> Result<Value, Thrown> {
    no_arg_string(cx, r, sys::xmlTextReaderReadString)
}

fn m_set_schema(cx: &mut Cx, r: &Rc<Reader>, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let source = opt_bytes(args, 0);
    if let Some(s) = &source {
        check_path(cx, 1, "filename", s)?;
        if s.is_empty() {
            return Err(cx.arg_error("ValueError", 1, "filename", "must not be empty"));
        }
    }
    let p = r.ptr.get();
    if p.is_null() {
        return Err(Thrown::error(
            "Error",
            "Schema must be set prior to reading",
        ));
    }
    let cs_ = source.as_deref().map(cs);
    let v = cx.with_libxml(|| unsafe {
        let mut saved = [0 as c_int; 6];
        sys::rhp_xml_sanitize_globals(saved.as_mut_ptr());
        let v = sys::xmlTextReaderSchemaValidate(
            p,
            cs_.as_ref().map_or(ptr::null(), |s| s.as_ptr().cast()),
        );
        sys::rhp_xml_restore_globals(saved.as_ptr());
        v
    })?;
    if v == 0 {
        return Ok(Value::Bool(true));
    }
    cx.warn("Schema contains errors")?;
    Ok(Value::Bool(false))
}

fn m_set_parser_property(
    cx: &mut Cx,
    r: &Rc<Reader>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let prop = conv::to_int(&arg(args, 0));
    let value = conv::to_bool(&arg(args, 1));
    let p = r.ptr.get();
    if p.is_null() {
        return Err(Thrown::error(
            "Error",
            "Cannot access parser properties before loading data",
        ));
    }
    let v = unsafe { sys::xmlTextReaderSetParserProp(p, prop as c_int, value as c_int) };
    if v == -1 {
        return Err(cx.arg_error(
            "ValueError",
            1,
            "property",
            "must be a valid parser property",
        ));
    }
    Ok(Value::Bool(true))
}

/// _xmlreader_get_relaxNG
fn get_relaxng(
    cx: &mut Cx,
    source: &[u8],
    from_file: bool,
) -> Result<*mut sys::xmlRelaxNG, Thrown> {
    let parser = if from_file {
        let Some(file) = crate::ext::dom::valid_file_path(source) else {
            return Ok(ptr::null_mut());
        };
        let cf = cs(&file);
        unsafe { sys::xmlRelaxNGNewParserCtxt(cf.as_ptr().cast()) }
    } else {
        unsafe { sys::xmlRelaxNGNewMemParserCtxt(source.as_ptr().cast(), source.len() as c_int) }
    };
    if parser.is_null() {
        return Ok(ptr::null_mut());
    }
    cx.with_libxml_owned(
        || unsafe {
            let mut saved = [0 as c_int; 6];
            sys::rhp_xml_sanitize_globals(saved.as_mut_ptr());
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
    )
}

/// php_xmlreader_set_relaxng_schema
fn set_relaxng(cx: &mut Cx, r: &Reader, args: &[Value], from_file: bool) -> Result<Value, Thrown> {
    let pname = if from_file { "filename" } else { "source" };
    let source = opt_bytes(args, 0);
    if let Some(s) = &source {
        check_path(cx, 1, pname, s)?;
        if s.is_empty() {
            return Err(cx.arg_error("ValueError", 1, pname, "must not be empty"));
        }
    }
    let p = r.ptr.get();
    if p.is_null() {
        return Err(Thrown::error(
            "Error",
            "Schema must be set prior to reading",
        ));
    }
    let mut schema = ptr::null_mut();
    let mut retval = -1;
    match &source {
        Some(s) => {
            schema = get_relaxng(cx, s, from_file)?;
            if !schema.is_null() {
                retval =
                    cx.with_libxml(|| unsafe { sys::xmlTextReaderRelaxNGSetSchema(p, schema) })?;
            }
        }
        None => {
            retval = cx.with_libxml(|| unsafe {
                sys::xmlTextReaderRelaxNGSetSchema(p, ptr::null_mut())
            })?;
        }
    }
    if retval == 0 {
        let old = r.schema.replace(schema);
        if !old.is_null() {
            unsafe { sys::xmlRelaxNGFree(old) };
        }
        return Ok(Value::Bool(true));
    }
    if !schema.is_null() {
        unsafe { sys::xmlRelaxNGFree(schema) };
    }
    cx.warn("Schema contains errors")?;
    Ok(Value::Bool(false))
}

fn m_set_relaxng_schema(
    cx: &mut Cx,
    r: &Rc<Reader>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    set_relaxng(cx, r, args, true)
}

fn m_set_relaxng_schema_source(
    cx: &mut Cx,
    r: &Rc<Reader>,
    _: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    set_relaxng(cx, r, args, false)
}

/// The script's directory as a base URI (`getcwd()` with a trailing
/// slash, canonicalized).
fn cwd_uri() -> Option<Vec<u8>> {
    let mut d = crate::zend::virtual_cwd::getcwd()?;
    if d.last() != Some(&b'/') {
        d.push(b'/');
    }
    let cd = cs(&d);
    unsafe { sys::take_string(sys::xmlCanonicPath(cd.as_ptr())) }
}

/// xml_reader_from_string
fn from_string(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
    class: Option<&[u8]>,
    throw: bool,
) -> Result<Value, Thrown> {
    let source = crate::ext::libxml::str_arg(args, 0);
    let source = source.as_bytes();
    let encoding = opt_bytes(args, 1);
    let options = conv::to_int(&arg(args, 2));
    if let Some(e) = &encoding {
        check_path(cx, 2, "encoding", e)?;
    }
    let target = if class.is_none() {
        let r = intern(this).expect("an XMLReader");
        r.free_resources();
        Some(r)
    } else {
        None
    };
    if source.is_empty() {
        return Err(cx.arg_error("ValueError", 1, "source", "must not be empty"));
    }
    if !valid_encoding(encoding.as_deref()) {
        return Err(cx.arg_error(
            "ValueError",
            2,
            "encoding",
            "must be a valid character encoding",
        ));
    }
    let inputbfr = unsafe {
        sys::xmlParserInputBufferCreateMem(sys::mem_ptr(source), source.len() as c_int, 0)
    };
    if !inputbfr.is_null() {
        let uri = cwd_uri().map(|u| cs(&u));
        let up = uri.as_ref().map_or(ptr::null(), |u| u.as_ptr().cast());
        let ce = encoding.as_deref().map(cs);
        let ep = ce.as_ref().map_or(ptr::null(), |e| e.as_ptr().cast());
        let reader = cx.with_libxml_owned(
            || unsafe {
                let mut saved = [0 as c_int; 6];
                sys::rhp_xml_sanitize_globals(saved.as_mut_ptr());
                let mut reader = sys::xmlNewTextReader(inputbfr, up);
                if !reader.is_null()
                    && sys::xmlTextReaderSetup(reader, ptr::null_mut(), up, ep, options as c_int)
                        != 0
                {
                    sys::xmlFreeTextReader(reader);
                    reader = ptr::null_mut();
                }
                sys::rhp_xml_restore_globals(saved.as_ptr());
                reader
            },
            |r| unsafe {
                if !r.is_null() {
                    sys::xmlFreeTextReader(r)
                }
            },
        );
        let reader = match reader {
            Ok(r) => r,
            Err(t) => {
                unsafe { sys::xmlFreeParserInputBuffer(inputbfr) };
                return Err(t);
            }
        };
        if !reader.is_null() {
            let r = match (target, class) {
                (Some(r), _) => r,
                (None, Some(class)) => match new_reader(cx, class) {
                    Ok((v, r)) => {
                        r.input.set(inputbfr);
                        r.ptr.set(reader);
                        return Ok(v);
                    }
                    Err(t) => {
                        unsafe {
                            sys::xmlFreeParserInputBuffer(inputbfr);
                            sys::xmlFreeTextReader(reader);
                        }
                        return Err(t);
                    }
                },
                (None, None) => unreachable!(),
            };
            r.input.set(inputbfr);
            r.ptr.set(reader);
            return Ok(Value::Bool(true));
        }
        unsafe { sys::xmlFreeParserInputBuffer(inputbfr) };
    }
    if throw {
        return Err(Thrown::error("Error", "Unable to load source data"));
    }
    cx.warn("Unable to load source data")?;
    Ok(Value::Bool(false))
}

fn m_xml(cx: &mut Cx, _: &Rc<Reader>, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    if matches!(this, Value::Object(_)) {
        from_string(cx, this, args, None, false)
    } else {
        from_string(cx, this, args, Some(b"XMLReader"), false)
    }
}

fn m_from_string(
    cx: &mut Cx,
    _: &Rc<Reader>,
    this: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let class = static_class(args);
    from_string(
        cx,
        this,
        &args[..args.len().saturating_sub(1)],
        Some(&class),
        true,
    )
}

fn m_expand(cx: &mut Cx, r: &Rc<Reader>, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let basenode = arg(args, 0);
    let mut docp = ptr::null_mut();
    let mut document = ptr::null_mut();
    if !matches!(basenode, Value::Null) {
        let Some((node, doc)) = crate::ext::dom::export_node(&basenode) else {
            let class =
                String::from_utf8_lossy(&cx.host.object_class_name(&basenode).unwrap_or_default())
                    .into_owned();
            cx.warn(&format!("Couldn't fetch {class}"))?;
            return Ok(Value::Bool(false));
        };
        docp = unsafe { (*node).doc };
        document = doc;
    }
    let p = r.ptr.get();
    if p.is_null() {
        return Err(Thrown::error(
            "Error",
            "Data must be loaded before expanding",
        ));
    }
    let node = cx.with_libxml(|| unsafe { sys::xmlTextReaderExpand(p) })?;
    if node.is_null() {
        cx.warn("An Error Occurred while expanding")?;
        return Ok(Value::Bool(false));
    }
    let nodec = unsafe { sys::xmlDocCopyNode(node, docp, 1) };
    if nodec.is_null() {
        cx.notice("Cannot expand this node type")?;
        return Ok(Value::Bool(false));
    }
    let mut dcx = crate::ext::dom::Cx {
        host: cx.host,
        fname: cx.fname.clone(),
        quiet: false,
        lazy_fname: false,
    };
    crate::ext::dom::create_object_in(&mut dcx, nodec, document)
}

/// `__xr("XMLReader::method", $this, ...$args)`: `$this` is null for a
/// static call, whose last argument is the called class.
fn fn_xr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
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
    let r = intern(&this).unwrap_or_else(|| Rc::new(Reader::new()));
    let mut cx = Cx {
        host,
        fname: String::from_utf8_lossy(&name).into_owned(),
    };
    m(&mut cx, &r, &this, &args[2.min(args.len())..])
}

pub fn register(m: &mut crate::Registry) {
    crate::register_helpers(
        m,
        &[
            ("__xr", fn_xr, 2, None),
            ("__xr_prop", fn_xr_prop, 3, Some(4)),
        ],
    );
}

pub const PRELUDE_PHP: &str = include_str!("prelude.php");
