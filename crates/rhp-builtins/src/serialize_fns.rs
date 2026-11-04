//! Native PHP serialization builtins: `serialize` / `unserialize`.
//!
//! Scope: null, bool, int, float, string, (nested) array, and objects (`O:`
//! with php-mangled property names, `__serialize`/`__unserialize`,
//! `__sleep`/`__wakeup`, enums as `E:`, `__PHP_Incomplete_Class`), with php's
//! `var_hash` back-references: a repeated object is `r:n;`, a repeated PHP
//! reference `R:n;` (so cycles terminate and identity round-trips).
//! `unserialize` reads `C:` (php's `object_custom`) for the classes with a
//! `ce->unserialize`: GMP and user classes implementing `Serializable`;
//! `serialize` never writes it. php's warnings (bad `__sleep`
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

use rhp_values::ops::deref;
use rhp_values::{conv, fmt, ArrayKey, PhpArray, PhpStr, Value};

use crate::{Host, Thrown};

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
        if host.not_serializable(class_id) {
            // php's `%s`: an anonymous class's name ends at its NUL.
            let shown = cname.split(|&b| b == 0).next().unwrap_or_default();
            return Err(Thrown::error(
                "Exception",
                format!(
                    "Serialization of '{}' is not allowed",
                    String::from_utf8_lossy(shown)
                ),
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
        // The object's slots under their php-mangled names, or an internal
        // class's get_properties_for(SERIALIZE).
        let view = crate::zend::object_handlers::properties_for(
            host,
            h,
            crate::zend::object_handlers::PropPurpose::Serialize,
        )?;
        let mut props: Vec<(PhpStr, Value)> = match view {
            Some(a) => a
                .iter()
                .map(|(k, v)| {
                    let k = match k {
                        ArrayKey::Int(n) => PhpStr::from(n.to_string().as_str()),
                        ArrayKey::Str(s) => s.clone(),
                    };
                    (k, v.clone())
                })
                .collect(),
            None => {
                let b = h.borrow();
                b.properties
                    .iter()
                    .filter(|(k, _)| !host.is_internal_slot(class_id, k))
                    .map(|(k, v)| (host.mangled_prop_name(class_id, k), v.clone()))
                    .collect()
            }
        };
        let incomplete = cname == INCOMPLETE;
        if !incomplete && host.class_has_method(class_id, b"__sleep") {
            let names = match deref(&call(host, "__sleep")?) {
                Value::Array(a) => a,
                _ => {
                    host.emit_diagnostic(
                        crate::E_WARNING,
                        &format!(
                            "serialize(): {}::__sleep() should return an array only containing the names of instance-variables to serialize",
                            String::from_utf8_lossy(&cname)
                        ),
                    )?;
                    out.extend_from_slice(b"N;");
                    return Ok(());
                }
            };
            // (__sleep may have changed the properties.)
            let props: Vec<(PhpStr, Value)> = {
                let b = h.borrow();
                b.properties
                    .iter()
                    .filter(|(k, _)| !host.is_internal_slot(class_id, k))
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
                match hit {
                    Some((k, v)) => {
                        if !picked.iter().any(|(pk, _)| pk == k) {
                            picked.push((k.clone(), v.clone()));
                        } else {
                            host.emit_diagnostic(
                                crate::E_WARNING,
                                &format!(
                                    "serialize(): \"{}\" is returned from __sleep() multiple times",
                                    String::from_utf8_lossy(&n)
                                ),
                            )?;
                        }
                    }
                    None => host.emit_diagnostic(
                        crate::E_WARNING,
                        &format!(
                            "serialize(): \"{}\" returned as member variable from __sleep() but does not exist",
                            String::from_utf8_lossy(&n)
                        ),
                    )?,
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
            // Property names are always string keys (`s:1:"0";`).
            serialize_str(out, k.as_bytes());
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
    /// Where php's "Error at offset" points: the innermost failure.
    err_pos: Option<usize>,
    /// A value was expected but the container ended (`}`).
    unexpected_end: bool,
}

/// php's var_unserializer gives up once memory_limit was crossed (the VM
/// then reports it).
fn oom_stop() -> PResult<()> {
    if rhp_values::mem::state().attention() {
        return malformed();
    }
    Ok(())
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
        let Some(bytes) = self.b.get(self.pos..self.pos + len) else {
            // php points just past the type letter and colon (`start + 2`).
            let at = self.b[..self.pos]
                .iter()
                .rposition(|&c| c == b':')
                .unwrap_or(0);
            let at = self.b[..at]
                .iter()
                .rposition(|&c| c == b':')
                .map_or(0, |i| i + 1);
            return self.fail_at(at);
        };
        let bytes = bytes.to_vec();
        self.pos += len;
        if self.expect(b'"').is_err() {
            return self.fail_at(self.pos);
        }
        Ok(bytes)
    }

    /// Fail with php's error offset at `pos` (unless an inner failure set it).
    fn fail_at<T>(&mut self, pos: usize) -> PResult<T> {
        self.err_pos.get_or_insert(pos);
        malformed()
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
            Some(b'}') => {
                self.unexpected_end = true;
                self.fail_at(self.pos)
            }
            _ => self.fail_at(self.pos),
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
        let start = self.pos;
        let r = self.parse_into_inner(cell);
        if matches!(r, Err(Fail::Malformed)) {
            self.err_pos.get_or_insert(start);
        }
        r
    }

    fn parse_into_inner(&mut self, cell: &Cell) -> PResult<()> {
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
                if self.expect(b';').is_err() {
                    return self.fail_at(self.pos);
                }
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
                    oom_stop()?;
                }
                if self.expect(b'}').is_err() {
                    return self.fail_at(self.pos);
                }
                Value::Array(arr)
            }
            Some(b'O') => return self.parse_object(cell),
            Some(b'C') => return self.parse_custom(cell),
            Some(b'}') => {
                // php: "this is the case where we have less data than planned"
                self.unexpected_end = true;
                return self.fail_at(self.pos);
            }
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
        let Some(cid) = self.host.resolve_class_autoload(class)? else {
            let msg = format!(
                "unserialize(): Class '{}' not found",
                String::from_utf8_lossy(class)
            );
            self.host.emit_diagnostic(crate::E_WARNING, &msg)?;
            return malformed();
        };
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
            self.host.resolve_class_autoload(&cname)?
        } else {
            None
        };
        let (obj, has_unserialize, has_wakeup) = match cid {
            Some(cid) => {
                let name = self.host.class_name(cid).unwrap_or_else(|| cname.clone());
                if self.host.not_serializable(cid) {
                    return Err(Thrown::error(
                        "Exception",
                        format!(
                            "Unserialization of '{}' is not allowed",
                            String::from_utf8_lossy(&name)
                        ),
                    )
                    .into());
                }
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
                oom_stop()?;
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
            oom_stop()?;
        }
        self.expect(b'}')?;
        if has_wakeup {
            self.pending.push((obj, None));
        }
        Ok(())
    }

    /// `C:len:"Class":datalen:{data}` (php's `object_custom`): the class's
    /// `ce->unserialize` reads `data`, parsing its values through the same
    /// var_hash.
    fn parse_custom(&mut self, cell: &Cell) -> PResult<()> {
        self.pos += 1;
        self.expect(b':')?;
        let len = self.read_len(b':')?;
        let cname = self.read_quoted(len)?;
        if cname.is_empty() || cname[0] == 0 || cname[0] == b'\\' || !valid_class_name(&cname) {
            return malformed();
        }
        if self.b.get(self.pos) != Some(&b':') {
            return self.fail_at(self.pos);
        }
        self.pos += 1;
        let cid = if self.class_allowed(&cname) {
            self.host.resolve_class_autoload(&cname)?
        } else {
            None
        };
        let name = match cid {
            Some(cid) => self.host.class_name(cid).unwrap_or_else(|| cname.clone()),
            None => INCOMPLETE.to_vec(),
        };
        if cid.is_some_and(|cid| self.host.not_serializable(cid)) {
            return Err(Thrown::error(
                "Exception",
                format!(
                    "Unserialization of '{}' is not allowed",
                    String::from_utf8_lossy(&name)
                ),
            )
            .into());
        }
        let datalen = self.parse_iv2()?;
        if self.b.len() - self.pos < 2 || self.b[self.pos] != b':' {
            return self.fail_at(self.pos);
        }
        if self.b[self.pos + 1] != b'{' {
            return self.fail_at(self.pos + 1);
        }
        self.pos += 2;
        let present = (self.b.len() - self.pos) as i64;
        if datalen < 0 || present <= datalen {
            let msg = format!(
                "Insufficient data for unserializing - {datalen} required, {present} present"
            );
            self.host.emit_diagnostic(crate::E_WARNING, &msg)?;
            return self.fail_at(self.pos);
        }
        let end = self.pos + datalen as usize;
        if self.b[end] != b'}' {
            return self.fail_at(end);
        }
        let data = &self.b[self.pos..end];
        match cid {
            Some(cid) if crate::ext::gmp::is_gmp_class(self.host, cid) => {
                let obj = crate::ext::gmp::unserialize_create(self.host)?;
                *cell.borrow_mut() = obj.clone();
                // The callback reads only its data.
                let whole = self.b;
                self.b = &whole[..end];
                let r = self.gmp_unserialize(&obj);
                self.b = whole;
                r?;
            }
            Some(_) if self.host.reflect_is_a(&name, b"Serializable") => {
                // zend_user_unserialize
                let obj = self.host.reflect_new_instance(&name, Vec::new(), false)?;
                *cell.borrow_mut() = obj.clone();
                let mut cb = PhpArray::new();
                cb.push(obj);
                cb.push(Value::str("unserialize"));
                self.host
                    .call_callable(&Value::Array(cb), &[Value::str(data.to_vec())])?;
            }
            // php's own Serializable classes (ArrayObject, SplObjectStorage…)
            // that rhp's do not implement yet.
            Some(cid) if self.stub_serializable(cid) => {
                return Err(Thrown::unsupported(
                    format!(
                        "unserialize() of a C: {} payload",
                        String::from_utf8_lossy(&name)
                    ),
                    "the class does not implement Serializable yet",
                )
                .into());
            }
            _ => {
                let msg = format!(
                    "Class {} has no unserializer",
                    String::from_utf8_lossy(&name)
                );
                self.host.emit_diagnostic(crate::E_WARNING, &msg)?;
                let obj = match cid {
                    Some(_) => self.host.reflect_new_instance(&name, Vec::new(), false)?,
                    None => self
                        .host
                        .instantiate(
                            INCOMPLETE,
                            vec![(INCOMPLETE_NAME.to_vec(), Value::str(cname))],
                        )
                        .ok_or(Fail::Malformed)?,
                };
                *cell.borrow_mut() = obj;
            }
        }
        oom_stop()?;
        self.pos = end + 1;
        Ok(())
    }

    /// Whether php's stubs declare that class `cid` or an ancestor
    /// implements `Serializable`.
    fn stub_serializable(&self, cid: u32) -> bool {
        let mut cur = Some(cid);
        while let Some(id) = cur {
            let name = self.host.class_name(id).unwrap_or_default();
            if rhp_bytecode::signatures::class(&name)
                .is_some_and(|c| c.interfaces.contains(&"Serializable"))
            {
                return true;
            }
            cur = self
                .host
                .class_parent_name(id)
                .and_then(|p| self.host.resolve_class(&p));
        }
        false
    }

    /// gmp_unserialize: the number as a decimal string, then the properties.
    fn gmp_unserialize(&mut self, obj: &Value) -> PResult<()> {
        let fail = |p: &mut Self, what: &str, prev: Option<Thrown>| -> Fail {
            let e = Thrown::error("Exception", format!("Could not unserialize {what}"));
            Fail::Thrown(match prev {
                Some(prev) => p.host.chain_thrown(e, prev),
                None => e,
            })
        };
        let num = match self.parse_slot() {
            Ok(v) => deref(&v),
            Err(Fail::Malformed) => return Err(fail(self, "number", None)),
            Err(Fail::Thrown(t)) => return Err(fail(self, "number", Some(t))),
        };
        let Value::Str(num) = num else {
            return Err(fail(self, "number", None));
        };
        if let Err(e) = crate::ext::gmp::unserialize_number(obj, num.as_bytes()) {
            return Err(fail(self, "number", Some(e)));
        }
        let props = match self.parse_slot() {
            Ok(v) => deref(&v),
            Err(Fail::Malformed) => return Err(fail(self, "properties", None)),
            Err(Fail::Thrown(t)) => return Err(fail(self, "properties", Some(t))),
        };
        let Value::Array(props) = props else {
            return Err(fail(self, "properties", None));
        };
        // zend_hash_copy into the property table.
        if let Value::Object(h) = obj {
            let mut o = h.borrow_mut();
            for (k, v) in props.iter() {
                let key = match k {
                    ArrayKey::Int(n) => PhpStr::from_bytes(n.to_string().into_bytes()),
                    ArrayKey::Str(s) => s.clone(),
                };
                o.properties.insert(key, v.clone());
            }
        }
        Ok(())
    }

    /// php's `parse_iv2`: an optionally signed integer (a too long one warns
    /// and saturates).
    fn parse_iv2(&mut self) -> PResult<i64> {
        let mut neg = false;
        match self.b.get(self.pos) {
            Some(b'-') => {
                neg = true;
                self.pos += 1;
            }
            Some(b'+') => self.pos += 1,
            _ => {}
        }
        while self.b.get(self.pos) == Some(&b'0') {
            self.pos += 1;
        }
        let start = self.pos;
        let mut v: u64 = 0;
        while let Some(&c) = self.b.get(self.pos).filter(|c| c.is_ascii_digit()) {
            v = v.wrapping_mul(10).wrapping_add((c - b'0') as u64);
            self.pos += 1;
        }
        // MAX_LENGTH_OF_LONG - 1 digits
        if self.pos - start > 19 || v > i64::MAX as u64 + neg as u64 {
            crate::warn(self.host, "unserialize", "Numerical result out of range")?;
            return Ok(if neg { i64::MIN } else { i64::MAX });
        }
        Ok(if neg {
            (v as i64).wrapping_neg()
        } else {
            v as i64
        })
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
fn allowed_option(host: &mut dyn Host, opts: Option<&Value>) -> Result<Allowed, Thrown> {
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
                let v = deref(v);
                // php: a string, or an object through its string cast.
                if !matches!(v, Value::Str(_) | Value::Object(_)) {
                    return Err(Thrown::type_error(format!(
                        "unserialize(): Option \"allowed_classes\" must be an array of class names, {} given",
                        crate::zend::zpp::zval_value_name(host, &v)
                    )));
                }
                let s = crate::zend::zpp::zval_get_string(host, &v)?;
                if !valid_class_name(&s) {
                    return Err(Thrown::error(
                        "ValueError",
                        format!(
                            "unserialize(): Option \"allowed_classes\" must be an array of class names, \"{}\" given",
                            String::from_utf8_lossy(&s)
                        ),
                    ));
                }
                names.push(s.to_ascii_lowercase())
            }
            Ok(Allowed::Only(names))
        }
        Some(other) => Err(Thrown::type_error(format!(
            "unserialize(): Option \"allowed_classes\" must be of type array|bool, {} given",
            crate::zend::zpp::zval_value_name(host, &other)
        ))),
    }
}

fn fn_unserialize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let bytes = conv::to_string_bytes(&deref(&args[0]));
    let allowed = allowed_option(host, args.get(1))?;
    let mut p = Parser {
        host,
        b: &bytes,
        pos: 0,
        slots: Vec::new(),
        referenced: HashSet::new(),
        pending: Vec::new(),
        allowed,
        err_pos: None,
        unexpected_end: false,
    };
    // php tolerates trailing "extra data" at the top level: it still returns
    // the parsed value (only emits a warning). Malformed/incomplete input
    // yields `false`.
    let total = bytes.len();
    if total == 0 {
        return Ok(Value::Bool(false));
    }
    let v = match p.parse_slot() {
        Ok(v) => v,
        Err(Fail::Malformed) => {
            let at = p.err_pos.unwrap_or(0);
            if p.unexpected_end {
                crate::warn(p.host, "unserialize", "Unexpected end of serialized data")?;
            }
            crate::warn(
                p.host,
                "unserialize",
                &format!("Error at offset {at} of {total} bytes"),
            )?;
            return Ok(Value::Bool(false));
        }
        Err(Fail::Thrown(t)) => return Err(t),
    };
    if p.pos < total {
        let at = p.pos;
        crate::warn(
            p.host,
            "unserialize",
            &format!("Extra data starting at offset {at} of {total} bytes"),
        )?;
    }
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

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[("serialize", fn_serialize), ("unserialize", fn_unserialize)],
    );
}

#[cfg(test)]
mod tests;
