//! Native PHP serialization builtins: `serialize` / `unserialize`
//! (php's `ext/standard/var.c` + `var_unserializer.re`).
//!
//! Format notes (php 8.5, `serialize_precision = -1`):
//! - `N;`                       null
//! - `b:0;` / `b:1;`            bool
//! - `i:42;`                    int
//! - `d:3.14;`                  float (shortest round-trip via `fmt::float_serialize`)
//! - `s:5:"hello";`             string (BYTE length, raw bytes, quotes NOT escaped)
//! - `a:2:{i:0;i:1;i:1;i:2;}`   array (count pairs of key/value)
//! - `O:1:"A":1:{s:1:"p";i:1;}` object (property keys visibility-mangled:
//!   protected `"\0*\0p"`, private `"\0A\0p"`); `__serialize()` /
//!   `__sleep()` shape the pairs, `__unserialize()` / `__wakeup()` run after
//!   the whole value is rebuilt, in order of appearance
//! - `C:1:"A":3:{abc}`          `Serializable::serialize()` payload
//! - `E:5:"S:Foo";`             enum case
//! - `r:2;` / `R:2;`            back-reference to the 2nd value (object
//!   identity / PHP reference)

use std::collections::HashMap;
use std::rc::Rc;

use rhp_values::{conv, fmt, ArrayKey, ObjectHandle, PhpArray, PhpStr, Value};

use crate::{Builtin, Host, Thrown};

/// Deref a (possibly `Ref`) value to an owned value.
fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

/// `[$obj, 'method']` as a callable value.
fn method_callable(obj: &ObjectHandle, method: &str) -> Value {
    let mut pair = PhpArray::new();
    pair.push(Value::Object(obj.clone()));
    pair.push(Value::str(method.as_bytes().to_vec()));
    Value::Array(pair)
}

const INCOMPLETE_CLASS: &[u8] = b"__PHP_Incomplete_Class";
const INCOMPLETE_NAME: &str = "__PHP_Incomplete_Class_Name";

// ---------------------------------------------------------------------------
// serialize
// ---------------------------------------------------------------------------

struct Ser<'h> {
    host: &'h mut dyn Host,
    out: Vec<u8>,
    /// php's `var_hash`: values seen so far (objects by identity, shared PHP
    /// references by cell) -> their 1-based position.
    n: i64,
    seen: HashMap<usize, i64>,
}

fn put_str(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(format!("s:{}:\"", bytes.len()).as_bytes());
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\";");
}

fn put_key(out: &mut Vec<u8>, k: &ArrayKey) {
    match k {
        ArrayKey::Int(n) => out.extend_from_slice(format!("i:{n};").as_bytes()),
        ArrayKey::Str(s) => put_str(out, s.as_bytes()),
    }
}

impl Ser<'_> {
    /// php's `php_add_var_hash`: count this value and, for an object or a
    /// shared reference, return the position it was first seen at.
    fn add_var_hash(&mut self, v: &Value) -> Option<i64> {
        self.n += 1;
        let key = match v {
            Value::Ref(r) => match &*r.borrow() {
                // A reference to an object is keyed by the object.
                Value::Object(h) => Rc::as_ptr(h) as *const u8 as usize,
                _ => Rc::as_ptr(r) as *const u8 as usize,
            },
            Value::Object(h) => Rc::as_ptr(h) as *const u8 as usize,
            _ => return None,
        };
        if let Some(&idx) = self.seen.get(&key) {
            if matches!(v, Value::Ref(_)) {
                self.n -= 1; // references are only counted once
            }
            return Some(idx);
        }
        self.seen.insert(key, self.n);
        None
    }

    /// Serialize one element of an array / property table: a reference
    /// nobody else holds is serialized as its value.
    fn element(&mut self, v: &Value) -> Result<(), Thrown> {
        match v {
            Value::Ref(r) if Rc::strong_count(r) == 1 => {
                let inner = r.borrow().clone();
                self.value(&inner)
            }
            other => self.value(other),
        }
    }

    fn value(&mut self, v: &Value) -> Result<(), Thrown> {
        if let Some(idx) = self.add_var_hash(v) {
            if idx == -1 {
                self.out.extend_from_slice(b"N;");
            } else if matches!(v, Value::Ref(_)) {
                self.out.extend_from_slice(format!("R:{idx};").as_bytes());
            } else {
                self.out.extend_from_slice(format!("r:{idx};").as_bytes());
            }
            return Ok(());
        }
        match deref(v) {
            Value::Null => self.out.extend_from_slice(b"N;"),
            Value::Bool(b) => self
                .out
                .extend_from_slice(if b { b"b:1;" } else { b"b:0;" }),
            Value::Int(n) => self.out.extend_from_slice(format!("i:{n};").as_bytes()),
            Value::Float(f) => {
                self.out.extend_from_slice(b"d:");
                self.out.extend_from_slice(&fmt::float_serialize(f));
                self.out.push(b';');
            }
            Value::Str(s) => put_str(&mut self.out, s.as_bytes()),
            Value::Array(a) => {
                self.out
                    .extend_from_slice(format!("a:{}:{{", a.len()).as_bytes());
                for (k, val) in a.iter() {
                    put_key(&mut self.out, k);
                    self.element(val)?;
                }
                self.out.push(b'}');
            }
            Value::Object(h) => self.object(&h)?,
            Value::Closure(_) => {
                return Err(Thrown::error(
                    "Exception",
                    "Serialization of 'Closure' is not allowed",
                ))
            }
            // php serializes a resource as the integer 0.
            Value::Resource(_) => self.out.extend_from_slice(b"i:0;"),
            Value::Ref(_) => unreachable!("deref'd above"),
        }
        Ok(())
    }

    fn class_header(&mut self, tag: u8, name: &[u8]) {
        self.out.push(tag);
        self.out
            .extend_from_slice(format!(":{}:\"", name.len()).as_bytes());
        self.out.extend_from_slice(name);
        self.out.extend_from_slice(b"\":");
    }

    fn pairs(&mut self, props: &[(ArrayKey, Value)]) -> Result<(), Thrown> {
        self.out
            .extend_from_slice(format!("{}:{{", props.len()).as_bytes());
        for (k, val) in props {
            put_key(&mut self.out, k);
            self.element(val)?;
        }
        self.out.push(b'}');
        Ok(())
    }

    fn object(&mut self, h: &ObjectHandle) -> Result<(), Thrown> {
        let class_id = h.borrow().class;
        let name = self.host.class_name(class_id).unwrap_or_default();
        if name.eq_ignore_ascii_case(b"Generator") {
            return Err(Thrown::error(
                "Exception",
                "Serialization of 'Generator' is not allowed",
            ));
        }
        if self.host.is_enum_class(class_id) {
            let case = match h.borrow().properties.get(&PhpStr::from("name")) {
                Some(Value::Str(c)) => c.as_bytes().to_vec(),
                _ => Vec::new(),
            };
            self.out
                .extend_from_slice(format!("E:{}:\"", name.len() + 1 + case.len()).as_bytes());
            self.out.extend_from_slice(&name);
            self.out.push(b':');
            self.out.extend_from_slice(&case);
            self.out.extend_from_slice(b"\";");
            return Ok(());
        }
        if self.host.class_has_method(class_id, b"__serialize") {
            let r = deref(
                &self
                    .host
                    .call_callable(&method_callable(h, "__serialize"), &[])?,
            );
            let Value::Array(a) = r else {
                return Err(Thrown::type_error(format!(
                    "{}::__serialize() must return an array",
                    String::from_utf8_lossy(&name)
                )));
            };
            self.class_header(b'O', &name);
            let props: Vec<(ArrayKey, Value)> =
                a.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            return self.pairs(&props);
        }
        if self.host.class_has_method(class_id, b"serialize")
            && self.host.reflect_is_a(&name, b"Serializable")
        {
            let r = deref(
                &self
                    .host
                    .call_callable(&method_callable(h, "serialize"), &[])?,
            );
            match r {
                Value::Str(data) => {
                    self.class_header(b'C', &name);
                    self.out
                        .extend_from_slice(format!("{}:{{", data.len()).as_bytes());
                    self.out.extend_from_slice(data.as_bytes());
                    self.out.push(b'}');
                }
                Value::Null => {
                    // Never back-reference an object that failed to serialize.
                    if let Some(e) = self.seen.get_mut(&(Rc::as_ptr(h) as *const u8 as usize)) {
                        *e = -1;
                    }
                    self.out.extend_from_slice(b"N;");
                }
                _ => {
                    return Err(Thrown::error(
                        "Exception",
                        format!(
                            "{}::serialize() must return a string or NULL",
                            String::from_utf8_lossy(&name)
                        ),
                    ))
                }
            }
            return Ok(());
        }
        let mangled = self.host.object_props_mangled(h);
        let incomplete = name == INCOMPLETE_CLASS;
        if !incomplete && self.host.class_has_method(class_id, b"__sleep") {
            let r = deref(
                &self
                    .host
                    .call_callable(&method_callable(h, "__sleep"), &[])?,
            );
            let Value::Array(names) = r else {
                // php warns "__sleep() should return an array only containing
                // the names of instance-variables to serialize".
                self.out.extend_from_slice(b"N;");
                return Ok(());
            };
            let mut props: Vec<(ArrayKey, Value)> = Vec::new();
            for (_, pn) in names.iter() {
                let pn = conv::to_string_bytes(&deref(pn));
                let mut priv_name = vec![0u8];
                priv_name.extend_from_slice(&name);
                priv_name.push(0);
                priv_name.extend_from_slice(&pn);
                let mut prot_name = b"\0*\0".to_vec();
                prot_name.extend_from_slice(&pn);
                for cand in [pn.clone(), priv_name, prot_name] {
                    let k = ArrayKey::Str(PhpStr::from_bytes(cand));
                    if let Some(v) = mangled.get(&k) {
                        // A name returned twice is only serialized once.
                        if !props.iter().any(|(pk, _)| pk == &k) {
                            props.push((k, v.clone()));
                        }
                        break;
                    }
                }
                // (A missing name only warns in php.)
            }
            self.class_header(b'O', &name);
            return self.pairs(&props);
        }
        let mut real_name = name.clone();
        let mut props: Vec<(ArrayKey, Value)> = Vec::new();
        for (k, v) in mangled.iter() {
            if incomplete {
                if let ArrayKey::Str(s) = k {
                    if s.as_bytes() == INCOMPLETE_NAME.as_bytes() {
                        if let Value::Str(n) = deref(v) {
                            real_name = n.as_bytes().to_vec();
                        }
                        continue;
                    }
                }
            }
            props.push((k.clone(), v.clone()));
        }
        self.class_header(b'O', &real_name);
        self.pairs(&props)
    }
}

fn fn_serialize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut s = Ser {
        host,
        out: Vec::new(),
        n: 0,
        seen: HashMap::new(),
    };
    s.value(&args[0])?;
    Ok(Value::str(s.out))
}

// ---------------------------------------------------------------------------
// unserialize
// ---------------------------------------------------------------------------

/// The `allowed_classes` option.
enum Allowed {
    All,
    None,
    Some(Vec<Vec<u8>>),
}

/// A post-processing call php delays until the whole value is rebuilt.
enum Delayed {
    Wakeup,
    Unserialize(PhpArray),
}

struct Parser<'a, 'h> {
    b: &'a [u8],
    pos: usize,
    host: &'h mut dyn Host,
    /// php's `var_hash`: every value except `R:` gets a 1-based slot, for
    /// `r:`/`R:` back-references.
    vars: Vec<Value>,
    /// Delayed `__wakeup`/`__unserialize` calls, in the order the objects
    /// were completed (innermost first).
    delayed: Vec<(ObjectHandle, Delayed)>,
    allowed: Allowed,
    /// A Thrown raised while parsing (autoloader, `Serializable::unserialize`).
    err: Option<Thrown>,
}

impl<'a> Parser<'a, '_> {
    fn expect(&mut self, c: u8) -> Option<()> {
        if self.pos < self.b.len() && self.b[self.pos] == c {
            self.pos += 1;
            Some(())
        } else {
            None
        }
    }

    /// Read bytes up to (not including) `term`, consuming `term`. `None` if
    /// `term` is never found.
    fn read_until(&mut self, term: u8) -> Option<&'a [u8]> {
        let start = self.pos;
        while self.pos < self.b.len() && self.b[self.pos] != term {
            self.pos += 1;
        }
        if self.pos >= self.b.len() {
            return None;
        }
        let slice = &self.b[start..self.pos];
        self.pos += 1; // consume terminator
        Some(slice)
    }

    /// `<len>:"<bytes>"` (the opening tag and its `:` already consumed).
    fn quoted(&mut self) -> Option<&'a [u8]> {
        let len = parse_usize(self.read_until(b':')?)?;
        self.expect(b'"')?;
        if self.pos + len > self.b.len() {
            return None;
        }
        let content = &self.b[self.pos..self.pos + len];
        self.pos += len;
        self.expect(b'"')?;
        Some(content)
    }

    /// A value, recorded in the back-reference table.
    fn parse_value(&mut self) -> Option<Value> {
        if self.b.get(self.pos) == Some(&b'R') {
            return self.parse_inner(None);
        }
        let slot = self.vars.len();
        self.vars.push(Value::Null);
        let v = self.parse_inner(Some(slot))?;
        self.vars[slot] = v.clone();
        Some(v)
    }

    /// An array key (never recorded).
    fn parse_key(&mut self) -> Option<Value> {
        match self.b.get(self.pos)? {
            b'i' | b's' => self.parse_inner(None),
            _ => None,
        }
    }

    fn parse_inner(&mut self, slot: Option<usize>) -> Option<Value> {
        let tag = *self.b.get(self.pos)?;
        self.pos += 1;
        match tag {
            b'N' => {
                self.expect(b';')?;
                Some(Value::Null)
            }
            b'b' => {
                self.expect(b':')?;
                match self.read_until(b';')? {
                    b"0" => Some(Value::Bool(false)),
                    b"1" => Some(Value::Bool(true)),
                    _ => None,
                }
            }
            b'i' => {
                self.expect(b':')?;
                let tok = self.read_until(b';')?;
                Some(Value::Int(parse_i64(tok)?))
            }
            b'd' => {
                self.expect(b':')?;
                let tok = self.read_until(b';')?;
                Some(Value::Float(parse_f64(tok)?))
            }
            b's' => {
                self.expect(b':')?;
                let content = self.quoted()?.to_vec();
                self.expect(b';')?;
                Some(Value::str(content))
            }
            b'r' | b'R' => {
                self.expect(b':')?;
                let idx = parse_usize(self.read_until(b';')?)?;
                if idx == 0 || idx > self.vars.len() {
                    return None;
                }
                // (A PHP reference `R:` is approximated by a copy; objects
                // keep their identity either way.)
                Some(self.vars[idx - 1].clone())
            }
            b'a' => {
                self.expect(b':')?;
                let count = parse_usize(self.read_until(b':')?)?;
                self.expect(b'{')?;
                let mut arr = PhpArray::new();
                for _ in 0..count {
                    let key_val = self.parse_key()?;
                    let key = ArrayKey::from_value(&key_val)?;
                    let val = self.parse_value()?;
                    arr.insert(key, val);
                }
                self.expect(b'}')?;
                Some(Value::Array(arr))
            }
            b'E' => {
                self.expect(b':')?;
                let content = self.quoted()?.to_vec();
                self.expect(b';')?;
                let colon = content.iter().position(|&c| c == b':')?;
                let (cls, case) = (&content[..colon], &content[colon + 1..]);
                let cid = self.lookup_class(cls)?;
                if !self.host.is_enum_class(cid) {
                    return None; // php: "Class 'X' is not an enum"
                }
                let cname = self.host.class_name(cid).unwrap_or_else(|| cls.to_vec());
                match self.host.class_constant_eval(&cname, case) {
                    Ok(Some(v @ Value::Object(_))) => {
                        // Only a case, not a plain constant, qualifies.
                        match &v {
                            Value::Object(h) if h.borrow().class == cid => Some(v),
                            _ => None,
                        }
                    }
                    Ok(_) => None, // php: "Undefined constant X::Y"
                    Err(e) => {
                        self.err = Some(e);
                        None
                    }
                }
            }
            b'O' | b'C' => self.parse_object(tag, slot),
            _ => None,
        }
    }

    /// Resolve a class for unserialization (autoloading it). A Thrown from an
    /// autoloader aborts the parse.
    fn lookup_class(&mut self, name: &[u8]) -> Option<u32> {
        if !name
            .iter()
            .all(|&c| c.is_ascii_alphanumeric() || c == b'_' || c == b'\\' || c >= 0x80)
            || name.is_empty()
        {
            return None;
        }
        self.host.resolve_class_autoload(name)
    }

    fn class_allowed(&self, name: &[u8]) -> bool {
        match &self.allowed {
            Allowed::All => true,
            Allowed::None => false,
            Allowed::Some(list) => list.iter().any(|n| n.eq_ignore_ascii_case(name)),
        }
    }

    fn parse_object(&mut self, tag: u8, slot: Option<usize>) -> Option<Value> {
        self.expect(b':')?;
        let name = self.quoted()?.to_vec();
        self.expect(b':')?;
        let allowed = self.class_allowed(&name);
        let cid = if allowed {
            self.lookup_class(&name)
        } else {
            None
        };
        if self.err.is_some() {
            return None;
        }
        let incomplete = cid.is_none();
        let class_name: Vec<u8> = match cid {
            Some(id) => self.host.class_name(id).unwrap_or_else(|| name.clone()),
            None => INCOMPLETE_CLASS.to_vec(),
        };
        if let Some(id) = cid {
            use rhp_values::ClassKind;
            let what = match self.host.class_kind(id) {
                Some(ClassKind::Enum) => Some("enum"),
                Some(ClassKind::Interface) => Some("interface"),
                Some(ClassKind::Trait) => Some("trait"),
                _ => None,
            };
            if let Some(what) = what {
                self.err = Some(Thrown::error(
                    "Error",
                    format!(
                        "Cannot instantiate {what} {}",
                        String::from_utf8_lossy(&class_name)
                    ),
                ));
                return None;
            }
        }
        let mut init = Vec::new();
        if incomplete {
            init.push((
                INCOMPLETE_NAME.as_bytes().to_vec(),
                Value::str(name.clone()),
            ));
        }
        let obj = match self.host.instantiate(&class_name, init)? {
            Value::Object(h) => h,
            _ => return None,
        };
        if let Some(s) = slot {
            self.vars[s] = Value::Object(obj.clone());
        }
        if tag == b'C' {
            let len = parse_usize(self.read_until(b':')?)?;
            self.expect(b'{')?;
            if self.pos + len > self.b.len() {
                return None;
            }
            let data = self.b[self.pos..self.pos + len].to_vec();
            self.pos += len;
            self.expect(b'}')?;
            if !incomplete && self.host.class_has_method(cid?, b"unserialize") {
                if let Err(e) = self
                    .host
                    .call_callable(&method_callable(&obj, "unserialize"), &[Value::str(data)])
                {
                    self.err = Some(e);
                    return None;
                }
            }
            return Some(Value::Object(obj));
        }
        let count = parse_usize(self.read_until(b':')?)?;
        self.expect(b'{')?;
        let has_unser = !incomplete && self.host.class_has_method(cid?, b"__unserialize");
        let mut data = PhpArray::new();
        for _ in 0..count {
            let key_val = self.parse_key()?;
            let val = self.parse_value()?;
            if has_unser {
                data.insert(ArrayKey::from_value(&key_val)?, val);
            } else {
                let key = conv::to_string_bytes(&key_val);
                let bare = crate::dump::unmangle(&key).to_vec();
                obj.borrow_mut()
                    .properties
                    .insert(PhpStr::from_bytes(bare), val);
            }
        }
        self.expect(b'}')?;
        if has_unser {
            self.delayed.push((obj.clone(), Delayed::Unserialize(data)));
        } else if !incomplete && self.host.class_has_method(cid?, b"__wakeup") {
            self.delayed.push((obj.clone(), Delayed::Wakeup));
        }
        Some(Value::Object(obj))
    }
}

/// Parse an optionally-signed integer token, saturating on overflow (php
/// clamps out-of-range ints to `PHP_INT_MIN`/`PHP_INT_MAX`).
fn parse_i64(tok: &[u8]) -> Option<i64> {
    if tok.is_empty() {
        return None;
    }
    let (neg, digits) = match tok[0] {
        b'+' => (false, &tok[1..]),
        b'-' => (true, &tok[1..]),
        _ => (false, tok),
    };
    if digits.is_empty() || !digits.iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let s = std::str::from_utf8(tok).ok()?;
    match s.parse::<i64>() {
        Ok(n) => Some(n),
        Err(_) => Some(if neg { i64::MIN } else { i64::MAX }),
    }
}

fn parse_usize(tok: &[u8]) -> Option<usize> {
    if tok.is_empty() || !tok.iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    std::str::from_utf8(tok).ok()?.parse::<usize>().ok()
}

/// Parse a `d:` float token. Handles php's `INF` / `-INF` / `NAN` spellings and
/// otherwise delegates to Rust's `f64` parser (which accepts `1.0E+20` etc.).
fn parse_f64(tok: &[u8]) -> Option<f64> {
    match tok {
        b"INF" => return Some(f64::INFINITY),
        b"-INF" => return Some(f64::NEG_INFINITY),
        b"NAN" => return Some(f64::NAN),
        _ => {}
    }
    let s = std::str::from_utf8(tok).ok()?;
    s.parse::<f64>().ok()
}

fn fn_unserialize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let bytes = conv::to_string_bytes(&deref(&args[0]));
    let mut allowed = Allowed::All;
    if let Some(Value::Array(opts)) = args.get(1).map(deref) {
        if let Some(a) = opts.get(&ArrayKey::Str(PhpStr::from("allowed_classes"))) {
            allowed = match deref(a) {
                Value::Bool(true) => Allowed::All,
                Value::Bool(false) => Allowed::None,
                Value::Array(list) => Allowed::Some(
                    list.iter()
                        .map(|(_, v)| conv::to_string_bytes(&deref(v)))
                        .collect(),
                ),
                other => {
                    let tn = match &other {
                        Value::Int(_) => "int".to_string(),
                        Value::Float(_) => "float".to_string(),
                        Value::Str(_) => "string".to_string(),
                        Value::Null => "null".to_string(),
                        Value::Object(h) => {
                            let id = h.borrow().class;
                            String::from_utf8_lossy(&host.class_name(id).unwrap_or_default())
                                .into_owned()
                        }
                        Value::Closure(_) => "Closure".to_string(),
                        _ => other.type_name().to_string(),
                    };
                    return Err(Thrown::type_error(format!(
                        "unserialize(): Option \"allowed_classes\" must be of type array|bool, {tn} given"
                    )));
                }
            };
        }
    }
    let mut p = Parser {
        b: &bytes,
        pos: 0,
        host,
        vars: Vec::new(),
        delayed: Vec::new(),
        allowed,
        err: None,
    };
    // php tolerates trailing "extra data" at the top level: it still returns
    // the parsed value (only emits a warning). Malformed/incomplete input
    // yields `false` (php also warns "Error at offset X of Y bytes").
    let parsed = p.parse_value();
    if let Some(e) = p.err.take() {
        return Err(e);
    }
    let Some(v) = parsed else {
        return Ok(Value::Bool(false));
    };
    for (obj, call) in std::mem::take(&mut p.delayed) {
        match call {
            Delayed::Wakeup => {
                p.host
                    .call_callable(&method_callable(&obj, "__wakeup"), &[])?;
            }
            Delayed::Unserialize(data) => {
                p.host.call_callable(
                    &method_callable(&obj, "__unserialize"),
                    &[Value::Array(data)],
                )?;
            }
        }
    }
    Ok(v)
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    }
    put(m, "serialize", fn_serialize, 1, Some(1));
    put(m, "unserialize", fn_unserialize, 1, Some(2));
}

// ---------------------------------------------------------------------------
// tests (expected values hardcoded from php 8.5.8)
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::approx_constant)] // deliberate literal test values, not PI
mod tests {
    use super::*;

    struct NullHost;
    impl Host for NullHost {
        fn write_out(&mut self, _bytes: &[u8]) {}
    }

    fn ser(v: Value) -> Vec<u8> {
        let mut h = NullHost;
        match fn_serialize(&mut h, &[v]).unwrap() {
            Value::Str(s) => s.as_bytes().to_vec(),
            _ => panic!("serialize did not return a string"),
        }
    }

    fn unser(s: &[u8]) -> Value {
        let mut h = NullHost;
        fn_unserialize(&mut h, &[Value::str(s.to_vec())]).unwrap()
    }

    fn arr(items: Vec<(ArrayKey, Value)>) -> Value {
        let mut a = PhpArray::new();
        for (k, v) in items {
            a.insert(k, v);
        }
        Value::Array(a)
    }

    // --- serialize: scalars -------------------------------------------------

    #[test]
    fn serialize_scalars() {
        // php -r 'echo serialize(null);'            => N;
        assert_eq!(ser(Value::Null), b"N;");
        // php -r 'echo serialize(false);'           => b:0;
        assert_eq!(ser(Value::Bool(false)), b"b:0;");
        // php -r 'echo serialize(true);'            => b:1;
        assert_eq!(ser(Value::Bool(true)), b"b:1;");
        // php -r 'echo serialize(42);'              => i:42;
        assert_eq!(ser(Value::Int(42)), b"i:42;");
        // php -r 'echo serialize(-7);'              => i:-7;
        assert_eq!(ser(Value::Int(-7)), b"i:-7;");
        // php -r 'echo serialize(3.14);'            => d:3.14;
        assert_eq!(ser(Value::Float(3.14)), b"d:3.14;");
        // php -r 'echo serialize("hello");'         => s:5:"hello";
        assert_eq!(ser(Value::str("hello")), b"s:5:\"hello\";");
    }

    #[test]
    fn serialize_float_precision() {
        // php -r 'echo serialize(0.1);'             => d:0.1;
        assert_eq!(ser(Value::Float(0.1)), b"d:0.1;");
        // php -r 'echo serialize(0.1+0.2);'         => d:0.30000000000000004;
        assert_eq!(ser(Value::Float(0.1 + 0.2)), b"d:0.30000000000000004;");
        // php -r 'echo serialize(100.0);'           => d:100;
        assert_eq!(ser(Value::Float(100.0)), b"d:100;");
        // php -r 'echo serialize(1.0e20);'          => d:1.0E+20;
        assert_eq!(ser(Value::Float(1.0e20)), b"d:1.0E+20;");
        // php -r 'echo serialize(INF);'             => d:INF;
        assert_eq!(ser(Value::Float(f64::INFINITY)), b"d:INF;");
        // php -r 'echo serialize(-INF);'            => d:-INF;
        assert_eq!(ser(Value::Float(f64::NEG_INFINITY)), b"d:-INF;");
        // php -r 'echo serialize(NAN);'             => d:NAN;
        assert_eq!(ser(Value::Float(f64::NAN)), b"d:NAN;");
    }

    #[test]
    fn serialize_byte_length_is_bytes() {
        // php -r 'echo bin2hex(serialize("\x00\x01ab"));' => 733a343a2200016162223b
        // i.e. s:4:"\x00\x01ab"; — 4 BYTES, quotes unescaped
        assert_eq!(
            ser(Value::str(vec![0u8, 1, b'a', b'b'])),
            b"s:4:\"\x00\x01ab\";"
        );
        // php -r 'echo serialize("he\"ll\\o");'     => s:7:"he"ll\o";
        assert_eq!(ser(Value::str("he\"ll\\o")), b"s:7:\"he\"ll\\o\";");
    }

    #[test]
    fn serialize_arrays() {
        // php -r 'echo serialize([1,"two",3.5,true,null]);'
        //   => a:5:{i:0;i:1;i:1;s:3:"two";i:2;d:3.5;i:3;b:1;i:4;N;}
        let v = arr(vec![
            (ArrayKey::Int(0), Value::Int(1)),
            (ArrayKey::Int(1), Value::str("two")),
            (ArrayKey::Int(2), Value::Float(3.5)),
            (ArrayKey::Int(3), Value::Bool(true)),
            (ArrayKey::Int(4), Value::Null),
        ]);
        assert_eq!(
            ser(v),
            &b"a:5:{i:0;i:1;i:1;s:3:\"two\";i:2;d:3.5;i:3;b:1;i:4;N;}"[..]
        );
    }

    #[test]
    fn serialize_nested_and_string_keys() {
        // php -r 'echo serialize(["a"=>1,"b"=>[2,3]]);'
        //   => a:2:{s:1:"a";i:1;s:1:"b";a:2:{i:0;i:2;i:1;i:3;}}
        let inner = arr(vec![
            (ArrayKey::Int(0), Value::Int(2)),
            (ArrayKey::Int(1), Value::Int(3)),
        ]);
        let v = arr(vec![
            (
                ArrayKey::Str(rhp_values::PhpStr::from_bytes(b"a".to_vec())),
                Value::Int(1),
            ),
            (
                ArrayKey::Str(rhp_values::PhpStr::from_bytes(b"b".to_vec())),
                inner,
            ),
        ]);
        assert_eq!(
            ser(v),
            &b"a:2:{s:1:\"a\";i:1;s:1:\"b\";a:2:{i:0;i:2;i:1;i:3;}}"[..]
        );
    }

    #[test]
    fn serialize_empty_array() {
        // php -r 'echo serialize([]);'              => a:0:{}
        assert_eq!(ser(arr(vec![])), b"a:0:{}");
    }

    // --- unserialize: happy path -------------------------------------------

    #[test]
    fn unserialize_scalars() {
        assert!(matches!(unser(b"N;"), Value::Null));
        assert!(matches!(unser(b"b:1;"), Value::Bool(true)));
        assert!(matches!(unser(b"b:0;"), Value::Bool(false)));
        assert!(matches!(unser(b"i:42;"), Value::Int(42)));
        assert!(matches!(unser(b"i:-7;"), Value::Int(-7)));
        match unser(b"d:3.14;") {
            Value::Float(f) => assert_eq!(f, 3.14),
            _ => panic!(),
        }
        match unser(b"s:5:\"hello\";") {
            Value::Str(s) => assert_eq!(s.as_bytes(), b"hello"),
            _ => panic!(),
        }
    }

    #[test]
    fn unserialize_float_specials() {
        match unser(b"d:0.30000000000000004;") {
            Value::Float(f) => assert_eq!(f, 0.1 + 0.2),
            _ => panic!(),
        }
        match unser(b"d:1.0E+20;") {
            Value::Float(f) => assert_eq!(f, 1.0e20),
            _ => panic!(),
        }
        assert!(matches!(unser(b"d:INF;"), Value::Float(f) if f == f64::INFINITY));
        assert!(matches!(unser(b"d:-INF;"), Value::Float(f) if f == f64::NEG_INFINITY));
        assert!(matches!(unser(b"d:NAN;"), Value::Float(f) if f.is_nan()));
    }

    #[test]
    fn unserialize_array() {
        // php -r 'var_dump(unserialize("a:2:{i:0;i:1;i:1;i:2;}"));'
        //   => array(2){ [0]=>int(1) [1]=>int(2) }
        match unser(b"a:2:{i:0;i:1;i:1;i:2;}") {
            Value::Array(a) => {
                assert_eq!(a.len(), 2);
                assert!(matches!(a.get(&ArrayKey::Int(0)), Some(Value::Int(1))));
                assert!(matches!(a.get(&ArrayKey::Int(1)), Some(Value::Int(2))));
            }
            _ => panic!(),
        }
        assert!(matches!(unser(b"a:0:{}"), Value::Array(a) if a.is_empty()));
    }

    #[test]
    fn unserialize_int_overflow_saturates() {
        // php -r 'var_dump(unserialize("i:9999999999999999999999;"));'
        //   => int(9223372036854775807)
        assert!(matches!(
            unser(b"i:9999999999999999999999;"),
            Value::Int(i64::MAX)
        ));
    }

    // --- round trip ---------------------------------------------------------

    #[test]
    fn round_trip() {
        let v = arr(vec![
            (ArrayKey::Int(0), Value::Int(1)),
            (ArrayKey::Int(1), Value::str("two")),
            (ArrayKey::Int(2), Value::Float(3.5)),
            (ArrayKey::Int(3), Value::Bool(true)),
            (ArrayKey::Int(4), Value::Null),
        ]);
        let s = ser(v);
        assert_eq!(
            s,
            &b"a:5:{i:0;i:1;i:1;s:3:\"two\";i:2;d:3.5;i:3;b:1;i:4;N;}"[..]
        );
        match unser(&s) {
            Value::Array(a) => {
                assert_eq!(a.len(), 5);
                assert!(matches!(a.get(&ArrayKey::Int(0)), Some(Value::Int(1))));
                assert!(
                    matches!(a.get(&ArrayKey::Int(1)), Some(Value::Str(x)) if x.as_bytes() == b"two")
                );
                assert!(matches!(a.get(&ArrayKey::Int(2)), Some(Value::Float(f)) if *f == 3.5));
                assert!(matches!(a.get(&ArrayKey::Int(3)), Some(Value::Bool(true))));
                assert!(matches!(a.get(&ArrayKey::Int(4)), Some(Value::Null)));
            }
            _ => panic!(),
        }
    }

    // --- unserialize: malformed => false -----------------------------------

    #[test]
    fn unserialize_malformed_returns_false() {
        // All of these are `bool(false)` under php 8.5.8.
        assert!(matches!(unser(b"x"), Value::Bool(false)));
        assert!(matches!(unser(b""), Value::Bool(false)));
        assert!(matches!(unser(b"i:42"), Value::Bool(false))); // missing ';'
        assert!(matches!(unser(b"s:5:\"hel\";"), Value::Bool(false))); // short string
        assert!(matches!(unser(b"s:3:\"ab\";"), Value::Bool(false))); // len mismatch
        assert!(matches!(unser(b"b:2;"), Value::Bool(false))); // bad bool
    }

    #[test]
    fn unserialize_trailing_data_tolerated() {
        // php -r 'var_dump(unserialize("s:3:\"abc\";extra"));' => string(3) "abc"
        match unser(b"s:3:\"abc\";extra") {
            Value::Str(s) => assert_eq!(s.as_bytes(), b"abc"),
            _ => panic!(),
        }
    }
}
