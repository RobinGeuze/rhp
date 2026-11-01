//! Native PHP serialization builtins: `serialize` / `unserialize`.
//!
//! Scope: null, bool, int, float, string, (nested) array, and objects (`O:`
//! with php-mangled property names, `__serialize`/`__unserialize`,
//! `__sleep`/`__wakeup`, enums as `E:`, `__PHP_Incomplete_Class`), with php's
//! `var_hash` back-references: a repeated object is `r:n;`, a repeated PHP
//! reference `R:n;` (so cycles terminate and identity round-trips).
//! `Serializable` (`C:`) is not modelled. php's warnings (bad `__sleep`
//! names, "Error at offset", undefined enum case, …) are not emitted: the
//! engine has no warning channel yet; the values / return codes match.
//!
//! Format notes (php 8.5, `serialize_precision = -1`):
//! - `N;`                       null
//! - `b:0;` / `b:1;`            bool
//! - `i:42;`                    int
//! - `d:3.14;`                  float (shortest round-trip via `fmt::float_serialize`)
//! - `s:5:"hello";`             string (BYTE length, raw bytes, quotes NOT escaped)
//! - `a:2:{i:0;i:1;i:1;i:2;}`   array (count pairs of key/value)

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use rhp_values::{conv, fmt, ArrayKey, PhpArray, PhpStr, Value};

use crate::{Builtin, Host, Thrown, TypeNameLower};

/// Deref a (possibly `Ref`) value to an owned value.
fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

const INCOMPLETE: &[u8] = b"__PHP_Incomplete_Class";
const INCOMPLETE_NAME: &[u8] = b"__PHP_Incomplete_Class_Name";

// ---------------------------------------------------------------------------
// serialize
// ---------------------------------------------------------------------------

/// What a `var_hash` slot was registered for.
#[derive(Hash, PartialEq, Eq, Clone, Copy)]
enum VarKey {
    Obj(u32),
    Ref(usize),
}

/// php's `php_serialize_data`: every serialized value takes the next slot
/// number; objects and references remember theirs for `r:` / `R:`.
struct Serializer {
    n: i64,
    seen: HashMap<VarKey, i64>,
}

impl Serializer {
    /// `php_add_var_hash`: 0 for a first sighting, else the earlier slot.
    fn add_var_hash(&mut self, v: &Value) -> i64 {
        self.n += 1;
        let (key, is_ref) = match v {
            Value::Ref(rc) => match &*rc.borrow() {
                // References to objects are treated as if the reference
                // didn't exist.
                Value::Object(h) => (VarKey::Obj(h.borrow().id), true),
                _ => (VarKey::Ref(Rc::as_ptr(rc) as usize), true),
            },
            Value::Object(h) => (VarKey::Obj(h.borrow().id), false),
            _ => return 0,
        };
        match self.seen.get(&key) {
            Some(&n) => {
                // References are only counted once.
                if is_ref && n != -1 {
                    self.n -= 1;
                }
                n
            }
            None => {
                self.seen.insert(key, self.n);
                0
            }
        }
    }

    /// A nested element / property value: a reference nobody else holds is
    /// serialized as its plain value (`Z_REFCOUNT == 1`).
    fn nested(&mut self, host: &mut dyn Host, out: &mut Vec<u8>, v: &Value) -> Result<(), Thrown> {
        match v {
            Value::Ref(rc) if Rc::strong_count(rc) == 1 => {
                let inner = rc.borrow().clone();
                self.value(host, out, &inner)
            }
            _ => self.value(host, out, v),
        }
    }

    fn value(&mut self, host: &mut dyn Host, out: &mut Vec<u8>, v: &Value) -> Result<(), Thrown> {
        match self.add_var_hash(v) {
            0 => {}
            -1 => {
                out.extend_from_slice(b"N;");
                return Ok(());
            }
            n => {
                let tag = if matches!(v, Value::Ref(_)) { "R" } else { "r" };
                out.extend_from_slice(format!("{tag}:{n};").as_bytes());
                return Ok(());
            }
        }
        match deref(v) {
            Value::Null => out.extend_from_slice(b"N;"),
            Value::Bool(b) => out.extend_from_slice(if b { b"b:1;" } else { b"b:0;" }),
            Value::Int(n) => out.extend_from_slice(format!("i:{n};").as_bytes()),
            Value::Float(f) => {
                out.extend_from_slice(b"d:");
                out.extend_from_slice(&fmt::float_serialize(f));
                out.push(b';');
            }
            Value::Str(s) => serialize_str(out, s.as_bytes()),
            Value::Array(a) => {
                out.extend_from_slice(format!("a:{}:{{", a.len()).as_bytes());
                for (k, val) in a.iter() {
                    serialize_key(out, k);
                    self.nested(host, out, val)?;
                }
                out.push(b'}');
            }
            Value::Object(h) => self.object(host, out, &h)?,
            Value::Closure(_) => {
                return Err(Thrown::error(
                    "Exception",
                    "Serialization of 'Closure' is not allowed",
                ))
            }
            Value::Resource(_) => out.extend_from_slice(b"i:0;"), // php serializes a resource as 0
            Value::Ref(_) => unreachable!("deref'd above"),
        }
        Ok(())
    }

    fn object(
        &mut self,
        host: &mut dyn Host,
        out: &mut Vec<u8>,
        h: &rhp_values::ObjectHandle,
    ) -> Result<(), Thrown> {
        let class_id = h.borrow().class;
        let mut cname = host.class_name(class_id).unwrap_or_default();
        if cname.eq_ignore_ascii_case(b"Generator") {
            return Err(Thrown::error(
                "Exception",
                "Serialization of 'Generator' is not allowed",
            ));
        }
        if host.is_enum_class(class_id) {
            let case = h
                .borrow()
                .properties
                .get(&PhpStr::from("name"))
                .map(conv::to_string_bytes)
                .unwrap_or_default();
            let body = [&cname[..], b":", &case[..]].concat();
            out.extend_from_slice(format!("E:{}:\"", body.len()).as_bytes());
            out.extend_from_slice(&body);
            out.extend_from_slice(b"\";");
            return Ok(());
        }
        let obj = Value::Object(h.clone());
        let call = |host: &mut dyn Host, m: &str| {
            let mut cb = PhpArray::new();
            cb.push(obj.clone());
            cb.push(Value::str(m));
            host.call_callable(&Value::Array(cb), &[])
        };
        let head = |out: &mut Vec<u8>, cname: &[u8], n: usize| {
            out.extend_from_slice(format!("O:{}:\"", cname.len()).as_bytes());
            out.extend_from_slice(cname);
            out.extend_from_slice(format!("\":{n}:{{").as_bytes());
        };
        if host.class_has_method(class_id, b"__serialize") {
            let data = match deref(&call(host, "__serialize")?) {
                Value::Array(a) => a,
                _ => {
                    return Err(Thrown::type_error(format!(
                        "{}::__serialize() must return an array",
                        String::from_utf8_lossy(&cname)
                    )))
                }
            };
            head(out, &cname, data.len());
            for (k, val) in data.iter() {
                serialize_key(out, k);
                self.nested(host, out, val)?;
            }
            out.push(b'}');
            return Ok(());
        }
        // The object's slots under their php-mangled names.
        let mut props: Vec<(PhpStr, Value)> = {
            let b = h.borrow();
            b.properties
                .iter()
                .map(|(k, v)| (host.mangled_prop_name(class_id, k), v.clone()))
                .collect()
        };
        let incomplete = cname == INCOMPLETE;
        if !incomplete && host.class_has_method(class_id, b"__sleep") {
            let names = match deref(&call(host, "__sleep")?) {
                Value::Array(a) => a,
                // php warns "C::__sleep() should return an array only
                // containing the names of instance-variables to serialize".
                _ => {
                    out.extend_from_slice(b"N;");
                    return Ok(());
                }
            };
            // (__sleep may have changed the properties.)
            let props: Vec<(PhpStr, Value)> = {
                let b = h.borrow();
                b.properties
                    .iter()
                    .map(|(k, v)| (host.mangled_prop_name(class_id, k), v.clone()))
                    .collect()
            };
            // php_var_serialize_get_sleep_props: the name as is, then this
            // class's private mangling, then the protected one; a name that
            // matches none is skipped ("returned as member variable from
            // __sleep() but does not exist"), a repeat is dropped ("is
            // returned from __sleep() multiple times").
            let mut picked: Vec<(PhpStr, Value)> = Vec::new();
            for (_, n) in names.iter() {
                let n = conv::to_string_bytes(&deref(n));
                let priv_ = [&b"\0"[..], &cname[..], b"\0", &n[..]].concat();
                let prot = [&b"\0*\0"[..], &n[..]].concat();
                let hit = [&n[..], &priv_[..], &prot[..]]
                    .iter()
                    .find_map(|want| props.iter().find(|(k, _)| k.as_bytes() == *want));
                if let Some((k, v)) = hit {
                    if !picked.iter().any(|(pk, _)| pk == k) {
                        picked.push((k.clone(), v.clone()));
                    }
                }
            }
            head(out, &cname, picked.len());
            for (k, v) in &picked {
                serialize_str(out, k.as_bytes());
                self.nested(host, out, v)?;
            }
            out.push(b'}');
            return Ok(());
        }
        if incomplete {
            // An incomplete object serializes under its original class name,
            // without the magic name member.
            if let Some(i) = props
                .iter()
                .position(|(k, _)| k.as_bytes() == INCOMPLETE_NAME)
            {
                if let Value::Str(orig) = deref(&props[i].1) {
                    cname = orig.as_bytes().to_vec();
                }
                props.remove(i);
            }
        }
        head(out, &cname, props.len());
        for (k, v) in &props {
            serialize_key(out, &ArrayKey::from_bytes(k.as_bytes()));
            self.nested(host, out, v)?;
        }
        out.push(b'}');
        Ok(())
    }
}

fn serialize_str(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(format!("s:{}:\"", bytes.len()).as_bytes());
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\";");
}

fn serialize_key(out: &mut Vec<u8>, k: &ArrayKey) {
    match k {
        ArrayKey::Int(n) => out.extend_from_slice(format!("i:{n};").as_bytes()),
        ArrayKey::Str(s) => serialize_str(out, s.as_bytes()),
    }
}

fn fn_serialize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = Vec::new();
    let mut s = Serializer {
        n: 0,
        seen: HashMap::new(),
    };
    s.value(host, &mut out, &deref(&args[0]))?;
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// unserialize
// ---------------------------------------------------------------------------

/// Why parsing stopped: malformed input (`false`) or a thrown error.
enum Fail {
    Malformed,
    Thrown(Thrown),
}

impl From<Thrown> for Fail {
    fn from(t: Thrown) -> Self {
        Fail::Thrown(t)
    }
}

type PResult<T> = Result<T, Fail>;

/// The `allowed_classes` option.
enum Allowed {
    All,
    None,
    /// Lowercased class names.
    Only(Vec<Vec<u8>>),
}

type Cell = Rc<RefCell<Value>>;

struct Parser<'a, 'h> {
    host: &'h mut dyn Host,
    b: &'a [u8],
    pos: usize,
    /// php's `var_hash` slots: every parsed value (but `R:`) in order. A
    /// container holds `Value::Ref(slot)` while parsing; slots no `R:` names
    /// are unwrapped back to plain values afterwards.
    slots: Vec<Cell>,
    /// Slots an `R:` targets: they stay PHP references.
    referenced: HashSet<usize>,
    /// Objects awaiting `__unserialize` (with its data) / `__wakeup`, in
    /// php's call order (they run once the whole payload has been parsed).
    pending: Vec<(Value, Option<Value>)>,
    allowed: Allowed,
}

fn malformed<T>() -> PResult<T> {
    Err(Fail::Malformed)
}

impl<'a> Parser<'a, '_> {
    fn expect(&mut self, c: u8) -> PResult<()> {
        if self.pos < self.b.len() && self.b[self.pos] == c {
            self.pos += 1;
            Ok(())
        } else {
            malformed()
        }
    }

    /// Read bytes up to (not including) `term`, consuming `term`.
    fn read_until(&mut self, term: u8) -> PResult<&'a [u8]> {
        let start = self.pos;
        while self.pos < self.b.len() && self.b[self.pos] != term {
            self.pos += 1;
        }
        if self.pos >= self.b.len() {
            return malformed();
        }
        let slice = &self.b[start..self.pos];
        self.pos += 1; // consume terminator
        Ok(slice)
    }

    fn read_len(&mut self, term: u8) -> PResult<usize> {
        parse_usize(self.read_until(term)?).ok_or(Fail::Malformed)
    }

    /// `"<len bytes>"` after a length prefix.
    fn read_quoted(&mut self, len: usize) -> PResult<Vec<u8>> {
        self.expect(b'"')?;
        let bytes = self
            .b
            .get(self.pos..self.pos + len)
            .ok_or(Fail::Malformed)?
            .to_vec();
        self.pos += len;
        self.expect(b'"')?;
        Ok(bytes)
    }

    /// A key (array key / property name): parsed without a var_hash slot.
    fn parse_key(&mut self) -> PResult<Value> {
        match self.b.get(self.pos) {
            Some(b'i') | Some(b's') => {
                let cell = Rc::new(RefCell::new(Value::Null));
                self.parse_into(&cell)?;
                let v = cell.borrow().clone();
                Ok(v)
            }
            _ => malformed(),
        }
    }

    /// One value as stored in its container: `Value::Ref(slot)`.
    fn parse_slot(&mut self) -> PResult<Value> {
        if self.b.get(self.pos) == Some(&b'R') {
            self.pos += 1;
            self.expect(b':')?;
            let id = self.read_len(b';')?;
            let cell = match id.checked_sub(1).and_then(|i| self.slots.get(i)) {
                Some(c) => c.clone(),
                None => return malformed(),
            };
            self.referenced.insert(Rc::as_ptr(&cell) as usize);
            return Ok(Value::Ref(cell));
        }
        let cell = Rc::new(RefCell::new(Value::Null));
        self.slots.push(cell.clone());
        self.parse_into(&cell)?;
        Ok(Value::Ref(cell))
    }

    fn parse_into(&mut self, cell: &Cell) -> PResult<()> {
        let v = match self.b.get(self.pos).copied() {
            Some(b'N') => {
                self.pos += 1;
                self.expect(b';')?;
                Value::Null
            }
            Some(b'b') => {
                self.pos += 1;
                self.expect(b':')?;
                match self.read_until(b';')? {
                    b"0" => Value::Bool(false),
                    b"1" => Value::Bool(true),
                    _ => return malformed(),
                }
            }
            Some(b'i') => {
                self.pos += 1;
                self.expect(b':')?;
                let tok = self.read_until(b';')?;
                Value::Int(parse_i64(tok).ok_or(Fail::Malformed)?)
            }
            Some(b'd') => {
                self.pos += 1;
                self.expect(b':')?;
                let tok = self.read_until(b';')?;
                Value::Float(parse_f64(tok).ok_or(Fail::Malformed)?)
            }
            Some(b's') => {
                self.pos += 1;
                self.expect(b':')?;
                let len = self.read_len(b':')?;
                let content = self.read_quoted(len)?;
                self.expect(b';')?;
                Value::str(content)
            }
            Some(b'r') => {
                self.pos += 1;
                self.expect(b':')?;
                let id = self.read_len(b';')?;
                let target = match id.checked_sub(1).and_then(|i| self.slots.get(i)) {
                    Some(c) if !Rc::ptr_eq(c, cell) => deref(&c.borrow()),
                    _ => return malformed(),
                };
                match target {
                    Value::Object(_) => target,
                    _ => return malformed(),
                }
            }
            Some(b'a') => {
                self.pos += 1;
                self.expect(b':')?;
                let count = self.read_len(b':')?;
                self.expect(b'{')?;
                let mut arr = PhpArray::new();
                for _ in 0..count {
                    let key = ArrayKey::from_value(&self.parse_key()?).ok_or(Fail::Malformed)?;
                    let val = self.parse_slot()?;
                    arr.insert(key, val);
                }
                self.expect(b'}')?;
                Value::Array(arr)
            }
            Some(b'O') => return self.parse_object(cell),
            Some(b'E') => {
                self.pos += 1;
                self.expect(b':')?;
                let len = self.read_len(b':')?;
                let body = self.read_quoted(len)?;
                self.expect(b';')?;
                self.enum_case(&body)?
            }
            _ => return malformed(),
        };
        *cell.borrow_mut() = v;
        Ok(())
    }

    /// `E:len:"Enum:Case";`. php warns (and fails) on a missing colon, an
    /// unknown class, a non-enum class, an undefined constant or a constant
    /// that is not a case.
    fn enum_case(&mut self, body: &[u8]) -> PResult<Value> {
        let colon = body
            .iter()
            .position(|&c| c == b':')
            .ok_or(Fail::Malformed)?;
        let (class, case) = (&body[..colon], &body[colon + 1..]);
        let cid = self
            .host
            .resolve_class_autoload(class)
            .ok_or(Fail::Malformed)?;
        if !self.host.is_enum_class(cid) {
            return malformed();
        }
        match self.host.class_constant(class, case) {
            Some(Value::Object(h)) if h.borrow().class == cid => Ok(Value::Object(h)),
            _ => malformed(),
        }
    }

    fn class_allowed(&self, name: &[u8]) -> bool {
        match &self.allowed {
            Allowed::All => true,
            Allowed::None => false,
            Allowed::Only(list) => list.iter().any(|n| n[..] == name.to_ascii_lowercase()[..]),
        }
    }

    /// `O:len:"Class":count:{props}`: the object is created (and takes its
    /// slot) before its properties are read, so `r:` inside can name it.
    fn parse_object(&mut self, cell: &Cell) -> PResult<()> {
        self.pos += 1;
        self.expect(b':')?;
        let len = self.read_len(b':')?;
        let cname = self.read_quoted(len)?;
        if cname.is_empty() || cname[0] == 0 || cname[0] == b'\\' || !valid_class_name(&cname) {
            return malformed();
        }
        self.expect(b':')?;
        let count = self.read_len(b':')?;
        self.expect(b'{')?;

        let cid = if self.class_allowed(&cname) {
            self.host.resolve_class_autoload(&cname)
        } else {
            None
        };
        let (obj, has_unserialize, has_wakeup) = match cid {
            Some(cid) => {
                let name = self.host.class_name(cid).unwrap_or_else(|| cname.clone());
                // Enums / interfaces / abstract classes throw "Cannot
                // instantiate ...".
                let obj = self.host.reflect_new_instance(&name, Vec::new(), false)?;
                let has_unserialize = self.host.class_has_method(cid, b"__unserialize");
                let has_wakeup = self.host.class_has_method(cid, b"__wakeup");
                (obj, has_unserialize, has_wakeup)
            }
            None => {
                // Unknown or disallowed class: an incomplete object that
                // remembers the name.
                let obj = self
                    .host
                    .instantiate(
                        INCOMPLETE,
                        vec![(INCOMPLETE_NAME.to_vec(), Value::str(cname))],
                    )
                    .ok_or(Fail::Malformed)?;
                (obj, false, false)
            }
        };
        *cell.borrow_mut() = obj.clone();

        if has_unserialize {
            let mut data = PhpArray::new();
            for _ in 0..count {
                let key = ArrayKey::from_value(&self.parse_key()?).ok_or(Fail::Malformed)?;
                let val = self.parse_slot()?;
                data.insert(key, val);
            }
            self.expect(b'}')?;
            self.pending.push((obj, Some(Value::Array(data))));
            return Ok(());
        }
        for _ in 0..count {
            let key = match self.parse_key()? {
                Value::Int(n) => n.to_string().into_bytes(),
                Value::Str(s) => s.as_bytes().to_vec(),
                _ => return malformed(),
            };
            let val = self.parse_slot()?;
            if !self.host.unserialize_set_prop(&obj, &key, val) {
                return malformed();
            }
        }
        self.expect(b'}')?;
        if has_wakeup {
            self.pending.push((obj, None));
        }
        Ok(())
    }

    /// Replace every slot wrapper no `R:` named by its plain value (php only
    /// has references where the payload asked for them).
    fn unwrap(&self, v: Value, objs: &mut HashSet<u32>, cells: &mut HashSet<usize>) -> Value {
        match v {
            Value::Ref(cell) => {
                let ptr = Rc::as_ptr(&cell) as usize;
                if self.referenced.contains(&ptr) {
                    if cells.insert(ptr) {
                        let inner = std::mem::replace(&mut *cell.borrow_mut(), Value::Null);
                        let inner = self.unwrap(inner, objs, cells);
                        *cell.borrow_mut() = inner;
                    }
                    Value::Ref(cell)
                } else {
                    let inner = std::mem::replace(&mut *cell.borrow_mut(), Value::Null);
                    let inner = self.unwrap(inner, objs, cells);
                    // Keep the slot readable for any later visitor.
                    *cell.borrow_mut() = inner.clone();
                    inner
                }
            }
            Value::Array(a) => {
                let mut out = PhpArray::new();
                for (k, val) in a.iter() {
                    out.insert(k.clone(), self.unwrap(val.clone(), objs, cells));
                }
                Value::Array(out)
            }
            Value::Object(h) => {
                let id = h.borrow().id;
                if objs.insert(id) {
                    let props: Vec<(PhpStr, Value)> = h
                        .borrow()
                        .properties
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    for (k, val) in props {
                        let u = self.unwrap(val, objs, cells);
                        if let Some(slot) = h.borrow_mut().properties.get_mut(&k) {
                            *slot = u;
                        }
                    }
                }
                Value::Object(h)
            }
            other => other,
        }
    }
}

/// php's `zend_is_valid_class_name`: name chars plus `\`.
fn valid_class_name(name: &[u8]) -> bool {
    name.iter()
        .all(|&c| c.is_ascii_alphanumeric() || c == b'_' || c == b'\\' || c >= 0x80)
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

/// The `allowed_classes` entry of unserialize()'s `$options`.
fn allowed_option(opts: Option<&Value>) -> Result<Allowed, Thrown> {
    let Some(Value::Array(opts)) = opts.map(deref) else {
        return Ok(Allowed::All);
    };
    match opts
        .get(&ArrayKey::from_bytes(b"allowed_classes"))
        .map(deref)
    {
        None => Ok(Allowed::All),
        Some(Value::Bool(true)) => Ok(Allowed::All),
        Some(Value::Bool(false)) => Ok(Allowed::None),
        Some(Value::Array(list)) => {
            let mut names = Vec::new();
            for (_, v) in list.iter() {
                match deref(v) {
                    Value::Str(s) => {
                        if !valid_class_name(s.as_bytes()) {
                            return Err(Thrown::error(
                                "ValueError",
                                format!(
                                    "unserialize(): Option \"allowed_classes\" must be an array of class names, \"{}\" given",
                                    String::from_utf8_lossy(s.as_bytes())
                                ),
                            ));
                        }
                        names.push(s.as_bytes().to_ascii_lowercase())
                    }
                    other => {
                        return Err(Thrown::type_error(format!(
                            "unserialize(): Option \"allowed_classes\" must be an array of class names, {} given",
                            other.type_name_lower()
                        )))
                    }
                }
            }
            Ok(Allowed::Only(names))
        }
        Some(other) => Err(Thrown::type_error(format!(
            "unserialize(): Option \"allowed_classes\" must be of type array|bool, {} given",
            other.type_name_lower()
        ))),
    }
}

fn fn_unserialize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let bytes = conv::to_string_bytes(&deref(&args[0]));
    let allowed = allowed_option(args.get(1))?;
    let mut p = Parser {
        host,
        b: &bytes,
        pos: 0,
        slots: Vec::new(),
        referenced: HashSet::new(),
        pending: Vec::new(),
        allowed,
    };
    // php tolerates trailing "extra data" at the top level: it still returns
    // the parsed value (only emits a warning). Malformed/incomplete input
    // yields `false`.
    let v = match p.parse_slot() {
        Ok(v) => v,
        Err(Fail::Malformed) => return Ok(Value::Bool(false)),
        Err(Fail::Thrown(t)) => return Err(t),
    };
    let (mut objs, mut cells) = (HashSet::new(), HashSet::new());
    let v = deref(&p.unwrap(v, &mut objs, &mut cells));
    let pending: Vec<(Value, Option<Value>)> = std::mem::take(&mut p.pending)
        .into_iter()
        .map(|(o, d)| (o, d.map(|d| p.unwrap(d, &mut objs, &mut cells))))
        .collect();
    for (obj, data) in pending {
        let mut cb = PhpArray::new();
        cb.push(obj);
        match data {
            Some(d) => {
                cb.push(Value::str("__unserialize"));
                p.host.call_callable(&Value::Array(cb), &[d])?;
            }
            None => {
                cb.push(Value::str("__wakeup"));
                p.host.call_callable(&Value::Array(cb), &[])?;
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
