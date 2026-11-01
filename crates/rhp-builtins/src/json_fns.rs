//! JSON builtins: `json_encode`, `json_decode`, `json_last_error`,
//! `json_last_error_msg`, and the `JSON_*` constants.
//!
//! Every behaviour is verified byte-for-byte against the local php 8.5.8 binary
//! (see the `#[cfg(test)]` block — expected values are hardcoded from `php -r`).
//!
//! ## Scope / known gaps
//! - `json_decode` decodes JSON objects to a `stdClass` (default / `$associative
//!   == false`) via `Host::make_object`, or to an associative array when
//!   `$associative` is true (or `JSON_OBJECT_AS_ARRAY`). The intentional
//!   divergence from php; everything else (numbers, unicode escapes incl.
//!   surrogate pairs, depth limit, duplicate keys = last wins, trailing data =
//!   error, `json_last_error`) matches php.
//! - `JSON_THROW_ON_ERROR` throws a `JsonException`-named error, but cannot set
//!   `getCode()` (engine errors carry no code), and catching it requires the
//!   `JsonException` class to be registered by the host.

use std::cell::Cell;

use rhp_values::{conv, fmt, ArrayKey, PhpArray, PhpStr, Value};

use crate::{Builtin, Host, Thrown};

// --- json_last_error state --------------------------------------------------

// Error codes (php `JSON_ERROR_*`).
const JSON_ERROR_NONE: i64 = 0;
const JSON_ERROR_DEPTH: i64 = 1;
const JSON_ERROR_STATE_MISMATCH: i64 = 2;
const JSON_ERROR_CTRL_CHAR: i64 = 3;
const JSON_ERROR_SYNTAX: i64 = 4;
const JSON_ERROR_UTF8: i64 = 5;
const JSON_ERROR_RECURSION: i64 = 6;
const JSON_ERROR_INF_OR_NAN: i64 = 7;
const JSON_ERROR_UNSUPPORTED_TYPE: i64 = 8;
const JSON_ERROR_INVALID_PROPERTY_NAME: i64 = 9;
const JSON_ERROR_UTF16: i64 = 10;
const JSON_ERROR_NON_BACKED_ENUM: i64 = 11;

// Encode flags.
const JSON_HEX_TAG: i64 = 1;
const JSON_HEX_AMP: i64 = 2;
const JSON_HEX_APOS: i64 = 4;
const JSON_HEX_QUOT: i64 = 8;
const JSON_FORCE_OBJECT: i64 = 16;
const JSON_NUMERIC_CHECK: i64 = 32;
const JSON_UNESCAPED_SLASHES: i64 = 64;
const JSON_PRETTY_PRINT: i64 = 128;
const JSON_UNESCAPED_UNICODE: i64 = 256;
const JSON_PARTIAL_OUTPUT_ON_ERROR: i64 = 512;
const JSON_PRESERVE_ZERO_FRACTION: i64 = 1024;
const JSON_INVALID_UTF8_IGNORE: i64 = 1_048_576;
const JSON_INVALID_UTF8_SUBSTITUTE: i64 = 2_097_152;
const JSON_THROW_ON_ERROR: i64 = 4_194_304;

// Decode flags.
const JSON_OBJECT_AS_ARRAY: i64 = 1;
const JSON_BIGINT_AS_STRING: i64 = 2;

thread_local! {
    static LAST_ERROR: Cell<i64> = const { Cell::new(0) };
}

pub(crate) fn reset_state() {
    set_last_error(0);
}

fn set_last_error(code: i64) {
    LAST_ERROR.with(|c| c.set(code));
}

fn get_last_error() -> i64 {
    LAST_ERROR.with(|c| c.get())
}

/// php's `json_last_error_msg` text for an error code.
fn error_msg(code: i64) -> &'static str {
    match code {
        JSON_ERROR_NONE => "No error",
        JSON_ERROR_DEPTH => "Maximum stack depth exceeded",
        JSON_ERROR_STATE_MISMATCH => "State mismatch (invalid or malformed JSON)",
        JSON_ERROR_CTRL_CHAR => "Control character error, possibly incorrectly encoded",
        JSON_ERROR_SYNTAX => "Syntax error",
        JSON_ERROR_UTF8 => "Malformed UTF-8 characters, possibly incorrectly encoded",
        JSON_ERROR_RECURSION => "Recursion detected",
        JSON_ERROR_INF_OR_NAN => "Inf and NaN cannot be JSON encoded",
        JSON_ERROR_UNSUPPORTED_TYPE => "Type is not supported",
        JSON_ERROR_INVALID_PROPERTY_NAME => "The decoded property name is invalid",
        JSON_ERROR_UTF16 => "Single unpaired UTF-16 surrogate in unicode escape",
        JSON_ERROR_NON_BACKED_ENUM => "Non-backed enums have no default serialization",
        _ => "Unknown error",
    }
}

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

// ---------------------------------------------------------------------------
// json_encode
// ---------------------------------------------------------------------------

fn fn_json_encode(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let flags = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    let mut out = Vec::new();
    set_last_error(JSON_ERROR_NONE);
    let mut enc = Encoder {
        flags,
        arrays: Vec::new(),
        objects: Vec::new(),
    };
    // An exception thrown by a jsonSerialize() propagates; the encoder's own
    // error code then stays NONE.
    let ok = match enc.value(host, &deref(&args[0]), &mut out, 0) {
        Ok(ok) => ok,
        Err(e) => {
            if flags & JSON_THROW_ON_ERROR == 0 || flags & JSON_PARTIAL_OUTPUT_ON_ERROR != 0 {
                set_last_error(JSON_ERROR_NONE);
            }
            return Err(e);
        }
    };
    if ok {
        // With JSON_PARTIAL_OUTPUT_ON_ERROR a substituted failure keeps its
        // json_last_error code.
        Ok(Value::str(out))
    } else {
        // Failure sets json_last_error at the failure site.
        if flags & JSON_THROW_ON_ERROR != 0 {
            return Err(json_exception(host, get_last_error()));
        }
        Ok(Value::Bool(false))
    }
}

fn indent(out: &mut Vec<u8>, level: usize) {
    for _ in 0..level {
        out.extend_from_slice(b"    ");
    }
}

/// php's `php_json_encoder`: the flags plus the containers being encoded
/// (recursion protection).
struct Encoder {
    flags: i64,
    arrays: Vec<usize>,
    objects: Vec<u32>,
}

impl Encoder {
    /// A failure `code`: with JSON_PARTIAL_OUTPUT_ON_ERROR, `subst` is written
    /// in place of the value and encoding continues.
    fn fail(&self, code: i64, out: &mut Vec<u8>, subst: &[u8]) -> bool {
        set_last_error(code);
        if self.flags & JSON_PARTIAL_OUTPUT_ON_ERROR != 0 {
            out.extend_from_slice(subst);
            true
        } else {
            false
        }
    }

    /// Encode one value. `Ok(false)` on an unencodable value (NAN/INF, invalid
    /// UTF-8, recursion, …) — php then fails the whole call.
    fn value(
        &mut self,
        host: &mut dyn Host,
        v: &Value,
        out: &mut Vec<u8>,
        level: usize,
    ) -> Result<bool, Thrown> {
        let flags = self.flags;
        let pretty = flags & JSON_PRETTY_PRINT != 0;
        match v {
            Value::Null => out.extend_from_slice(b"null"),
            Value::Bool(true) => out.extend_from_slice(b"true"),
            Value::Bool(false) => out.extend_from_slice(b"false"),
            Value::Int(n) => out.extend_from_slice(n.to_string().as_bytes()),
            Value::Float(f) => {
                if !f.is_finite() {
                    return Ok(self.fail(JSON_ERROR_INF_OR_NAN, out, b"0"));
                }
                encode_float(*f, out, flags);
            }
            Value::Str(s) => {
                if flags & JSON_NUMERIC_CHECK != 0 {
                    if let Some(num) = conv::numeric_string(s.as_bytes()) {
                        match num {
                            conv::Num::Int(n) => out.extend_from_slice(n.to_string().as_bytes()),
                            conv::Num::Float(f) if f.is_finite() => encode_float(f, out, flags),
                            conv::Num::Float(_) => {
                                return Ok(encode_string(s.as_bytes(), out, flags))
                            }
                        }
                        return Ok(true);
                    }
                }
                return Ok(encode_string(s.as_bytes(), out, flags));
            }
            Value::Array(a) => {
                let id = a.data_ptr();
                if self.arrays.contains(&id) {
                    return Ok(self.fail(JSON_ERROR_RECURSION, out, b"null"));
                }
                self.arrays.push(id);
                let r = self.array(host, a, out, level, pretty);
                self.arrays.pop();
                return r;
            }
            Value::Object(h) => return self.object(host, h, out, level, pretty),
            Value::Closure(_) => {
                // A Closure encodes as an empty object in php.
                return self.props(host, &[], out, level, pretty);
            }
            Value::Resource(_) => {
                return Ok(self.fail(JSON_ERROR_UNSUPPORTED_TYPE, out, b"null"));
            }
            Value::Ref(r) => {
                let inner = r.borrow().clone();
                return self.value(host, &inner, out, level);
            }
        }
        Ok(true)
    }

    /// An object: `JsonSerializable::jsonSerialize()`'s result, a backed
    /// enum's value, else a JSON object of its public properties (php skips
    /// every mangled — protected / private — name regardless of scope).
    fn object(
        &mut self,
        host: &mut dyn Host,
        h: &rhp_values::ObjectHandle,
        out: &mut Vec<u8>,
        level: usize,
        pretty: bool,
    ) -> Result<bool, Thrown> {
        let (class_id, id) = {
            let b = h.borrow();
            (b.class, b.id)
        };
        let cname = host.class_name(class_id).unwrap_or_default();
        if !host.is_enum_class(class_id) && host.reflect_is_a(&cname, b"JsonSerializable") {
            if self.objects.contains(&id) {
                return Ok(self.fail(JSON_ERROR_RECURSION, out, b"null"));
            }
            self.objects.push(id);
            let mut cb = PhpArray::new();
            cb.push(Value::Object(h.clone()));
            cb.push(Value::str("jsonSerialize"));
            let r = host.call_callable(&Value::Array(cb), &[]);
            let r = match r {
                Ok(v) => deref(&v),
                Err(t) => {
                    self.objects.pop();
                    return Err(t);
                }
            };
            // `return $this;` encodes the object's properties directly.
            let same = matches!(&r, Value::Object(o) if o.borrow().id == id);
            let res = if same {
                self.objects.pop();
                self.plain_object(host, h, out, level, pretty)
            } else {
                let res = self.value(host, &r, out, level);
                self.objects.pop();
                res
            };
            return res;
        }
        if host.is_enum_class(class_id) {
            let value = h.borrow().properties.get(&PhpStr::from("value")).cloned();
            return match value {
                Some(v) => self.value(host, &deref(&v), out, level),
                None => Ok(self.fail(JSON_ERROR_NON_BACKED_ENUM, out, b"0")),
            };
        }
        self.plain_object(host, h, out, level, pretty)
    }

    fn plain_object(
        &mut self,
        host: &mut dyn Host,
        h: &rhp_values::ObjectHandle,
        out: &mut Vec<u8>,
        level: usize,
        pretty: bool,
    ) -> Result<bool, Thrown> {
        let id = h.borrow().id;
        if self.objects.contains(&id) {
            return Ok(self.fail(JSON_ERROR_RECURSION, out, b"null"));
        }
        // get_properties_for(JSON): an ArrayObject / ArrayIterator encodes
        // its storage (always as an object).
        let sxe = crate::ext::simplexml::sxe_props(host, &Value::Object(h.clone()), false);
        let props: Vec<(PhpStr, Value)> = match crate::spl_array_storage(h) {
            _ if sxe.is_some() => sxe.unwrap_or_default(),
            Some((Value::Array(a), _)) => a
                .iter()
                .map(|(k, v)| {
                    let k = match k {
                        rhp_values::ArrayKey::Int(n) => PhpStr::from(n.to_string().as_str()),
                        rhp_values::ArrayKey::Str(s) => s.clone(),
                    };
                    (k, deref(v))
                })
                .collect(),
            _ => {
                let b = h.borrow();
                b.properties
                    .iter()
                    .filter(|(k, _)| {
                        host.mangled_prop_name(b.class, k).as_bytes().first() != Some(&0)
                            && !host.is_internal_slot(b.class, k)
                    })
                    .map(|(k, v)| (k.clone(), deref(v)))
                    .collect()
            }
        };
        self.objects.push(id);
        let r = self.props(host, &props, out, level, pretty);
        self.objects.pop();
        r
    }

    /// Encode an object's properties as a JSON object (`{"k":v,...}`, always braces).
    fn props(
        &mut self,
        host: &mut dyn Host,
        props: &[(PhpStr, Value)],
        out: &mut Vec<u8>,
        level: usize,
        pretty: bool,
    ) -> Result<bool, Thrown> {
        out.push(b'{');
        if props.is_empty() {
            out.push(b'}');
            return Ok(true);
        }
        for (i, (k, val)) in props.iter().enumerate() {
            if i > 0 {
                out.push(b',');
            }
            if pretty {
                out.push(b'\n');
                indent(out, level + 1);
            }
            if !encode_string(k.as_bytes(), out, self.flags) {
                return Ok(false);
            }
            out.extend_from_slice(if pretty { b": " } else { b":" });
            if !self.value(host, val, out, level + 1)? {
                return Ok(false);
            }
        }
        if pretty {
            out.push(b'\n');
            indent(out, level);
        }
        out.push(b'}');
        Ok(true)
    }

    fn array(
        &mut self,
        host: &mut dyn Host,
        a: &PhpArray,
        out: &mut Vec<u8>,
        level: usize,
        pretty: bool,
    ) -> Result<bool, Thrown> {
        let flags = self.flags;
        let force_object = flags & JSON_FORCE_OBJECT != 0;
        let is_list = !force_object
            && a.iter()
                .enumerate()
                .all(|(i, (k, _))| matches!(k, ArrayKey::Int(n) if *n == i as i64));
        let (open, close) = if is_list { (b'[', b']') } else { (b'{', b'}') };
        out.push(open);
        if a.is_empty() {
            out.push(close);
            return Ok(true);
        }
        for (i, (k, val)) in a.iter().enumerate() {
            if i > 0 {
                out.push(b',');
            }
            if pretty {
                out.push(b'\n');
                indent(out, level + 1);
            }
            if !is_list {
                let ok = match k {
                    ArrayKey::Int(n) => encode_string(n.to_string().as_bytes(), out, flags),
                    ArrayKey::Str(s) => encode_string(s.as_bytes(), out, flags),
                };
                if !ok {
                    return Ok(false);
                }
                out.extend_from_slice(if pretty { b": " } else { b":" });
            }
            if !self.value(host, val, out, level + 1)? {
                return Ok(false);
            }
        }
        if pretty {
            out.push(b'\n');
            indent(out, level);
        }
        out.push(close);
        Ok(true)
    }
}

fn encode_float(f: f64, out: &mut Vec<u8>, flags: i64) {
    let mut s = fmt::float_serialize(f);
    // php's json uses a lowercase exponent ('1.0e+20'); float_serialize emits the
    // var_dump-style uppercase 'E'.
    for b in s.iter_mut() {
        if *b == b'E' {
            *b = b'e';
        }
    }
    // JSON_PRESERVE_ZERO_FRACTION: an integral value keeps a `.0`.
    if flags & JSON_PRESERVE_ZERO_FRACTION != 0
        && s.iter().all(|&b| b.is_ascii_digit() || b == b'-')
    {
        s.extend_from_slice(b".0");
    }
    out.extend_from_slice(&s);
}

/// Append a JSON string literal for `bytes`. Returns false on invalid UTF-8.
fn encode_string(bytes: &[u8], out: &mut Vec<u8>, flags: i64) -> bool {
    let owned;
    let s = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        // JSON_INVALID_UTF8_IGNORE drops, JSON_INVALID_UTF8_SUBSTITUTE replaces
        // (U+FFFD) each invalid sequence.
        Err(_) if flags & (JSON_INVALID_UTF8_IGNORE | JSON_INVALID_UTF8_SUBSTITUTE) != 0 => {
            let mut t = String::with_capacity(bytes.len());
            for chunk in bytes.utf8_chunks() {
                t.push_str(chunk.valid());
                if !chunk.invalid().is_empty() && flags & JSON_INVALID_UTF8_IGNORE == 0 {
                    t.push('\u{fffd}');
                }
            }
            owned = t;
            &owned
        }
        Err(_) => {
            set_last_error(JSON_ERROR_UTF8);
            return false;
        }
    };
    let unescaped_slashes = flags & JSON_UNESCAPED_SLASHES != 0;
    let unescaped_unicode = flags & JSON_UNESCAPED_UNICODE != 0;
    out.push(b'"');
    for ch in s.chars() {
        let cp = ch as u32;
        match ch {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '/' if !unescaped_slashes => out.extend_from_slice(b"\\/"),
            '\u{08}' => out.extend_from_slice(b"\\b"),
            '\u{09}' => out.extend_from_slice(b"\\t"),
            '\u{0a}' => out.extend_from_slice(b"\\n"),
            '\u{0c}' => out.extend_from_slice(b"\\f"),
            '\u{0d}' => out.extend_from_slice(b"\\r"),
            _ if cp < 0x20 => out.extend_from_slice(format!("\\u{cp:04x}").as_bytes()),
            _ if cp < 0x80 => out.push(cp as u8),
            _ if unescaped_unicode => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
            _ if cp <= 0xFFFF => out.extend_from_slice(format!("\\u{cp:04x}").as_bytes()),
            _ => {
                let v = cp - 0x10000;
                let hi = 0xD800 + (v >> 10);
                let lo = 0xDC00 + (v & 0x3FF);
                out.extend_from_slice(format!("\\u{hi:04x}\\u{lo:04x}").as_bytes());
            }
        }
    }
    out.push(b'"');
    true
}

// ---------------------------------------------------------------------------
// json_decode
// ---------------------------------------------------------------------------

fn fn_json_decode(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let json = conv::to_string_bytes(&deref(&args[0]));
    // $associative: null (default) -> objects as stdClass (we fall back to array);
    // true -> array; false -> stdClass (-> array fallback). JSON_OBJECT_AS_ARRAY
    // also forces array.
    let assoc_arg = args.get(1).map(deref);
    let flags = args.get(3).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    let assoc = match assoc_arg {
        None | Some(Value::Null) => flags & JSON_OBJECT_AS_ARRAY != 0,
        Some(v) => conv::to_bool(&v),
    };
    let max_depth = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(512);
    let bigint_as_string = flags & JSON_BIGINT_AS_STRING != 0;

    // Validate UTF-8 before parsing (php returns JSON_ERROR_UTF8 for invalid).
    if std::str::from_utf8(&json).is_err() {
        set_last_error(JSON_ERROR_UTF8);
        return throw_or_null(host, flags, JSON_ERROR_UTF8);
    }

    let mut p = Parser {
        s: &json,
        i: 0,
        max_depth,
        assoc,
        bigint_as_string,
        host,
    };
    p.skip_ws();
    let result = p.parse_value(1);
    match result {
        Ok(v) => {
            p.skip_ws();
            if p.i != p.s.len() {
                // trailing non-whitespace data.
                set_last_error(JSON_ERROR_SYNTAX);
                return throw_or_null(host, flags, JSON_ERROR_SYNTAX);
            }
            set_last_error(JSON_ERROR_NONE);
            Ok(v)
        }
        Err(code) => {
            set_last_error(code);
            throw_or_null(host, flags, code)
        }
    }
}

fn throw_or_null(host: &mut dyn Host, flags: i64, code: i64) -> Result<Value, Thrown> {
    if flags & JSON_THROW_ON_ERROR != 0 {
        Err(json_exception(host, code))
    } else {
        Ok(Value::Null)
    }
}

/// `new JsonException(json_last_error_msg(), code)` (php_json_throw...).
fn json_exception(host: &mut dyn Host, code: i64) -> Thrown {
    let args = vec![Value::str(error_msg(code)), Value::Int(code)];
    match host.reflect_new_instance(b"JsonException", args, true) {
        Ok(Value::Object(h)) => Thrown::Object(h),
        Ok(_) => Thrown::error("JsonException", error_msg(code)),
        Err(t) => t,
    }
}

struct Parser<'a, 'h> {
    s: &'a [u8],
    i: usize,
    max_depth: i64,
    assoc: bool,
    bigint_as_string: bool,
    host: &'h mut dyn Host,
}

impl Parser<'_, '_> {
    fn skip_ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    /// Parse one JSON value at the current position. `depth` is the nesting level
    /// of this value (the top-level value is depth 1); exceeding `max_depth`
    /// yields `JSON_ERROR_DEPTH`.
    fn parse_value(&mut self, depth: i64) -> Result<Value, i64> {
        if depth > self.max_depth {
            return Err(JSON_ERROR_DEPTH);
        }
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(depth),
            Some(b'[') => self.parse_array(depth),
            Some(b'"') => Ok(Value::str(self.parse_string()?)),
            Some(b't') => self.parse_literal(b"true", Value::Bool(true)),
            Some(b'f') => self.parse_literal(b"false", Value::Bool(false)),
            Some(b'n') => self.parse_literal(b"null", Value::Null),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.parse_number(),
            _ => Err(JSON_ERROR_SYNTAX),
        }
    }

    fn parse_literal(&mut self, lit: &[u8], val: Value) -> Result<Value, i64> {
        if self.s[self.i..].starts_with(lit) {
            self.i += lit.len();
            Ok(val)
        } else {
            Err(JSON_ERROR_SYNTAX)
        }
    }

    fn parse_object(&mut self, depth: i64) -> Result<Value, i64> {
        self.i += 1; // consume '{'
        let mut pairs: Vec<(Vec<u8>, Value)> = Vec::new();
        self.skip_ws();
        if self.peek() != Some(b'}') {
            loop {
                self.skip_ws();
                if self.peek() != Some(b'"') {
                    return Err(JSON_ERROR_SYNTAX);
                }
                let key = self.parse_string()?;
                self.skip_ws();
                if self.peek() != Some(b':') {
                    return Err(JSON_ERROR_SYNTAX);
                }
                self.i += 1;
                // php_json_parser_object_update: an object property name may
                // not start with NUL (mangled names).
                if !self.assoc && key.first() == Some(&0) {
                    return Err(JSON_ERROR_INVALID_PROPERTY_NAME);
                }
                let val = self.parse_value(depth + 1)?;
                // Duplicate keys: last wins (overwrite in place, keep position).
                if let Some(slot) = pairs.iter_mut().find(|(k, _)| *k == key) {
                    slot.1 = val;
                } else {
                    pairs.push((key, val));
                }
                self.skip_ws();
                match self.peek() {
                    Some(b',') => self.i += 1,
                    Some(b'}') => {
                        self.i += 1;
                        break;
                    }
                    _ => return Err(JSON_ERROR_SYNTAX),
                }
            }
        } else {
            self.i += 1;
        }
        if self.assoc {
            let mut arr = PhpArray::new();
            for (k, v) in pairs {
                arr.insert(ArrayKey::from_bytes(&k), v);
            }
            Ok(Value::Array(arr))
        } else {
            // Decode to an object (stdClass via the host; array fallback otherwise).
            Ok(self.host.make_object(pairs))
        }
    }

    fn parse_array(&mut self, depth: i64) -> Result<Value, i64> {
        self.i += 1; // consume '['
        let mut arr = PhpArray::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Value::Array(arr));
        }
        loop {
            let val = self.parse_value(depth + 1)?;
            arr.push(val);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(JSON_ERROR_SYNTAX),
            }
        }
        Ok(Value::Array(arr))
    }

    /// Parse a `"`-delimited JSON string into decoded UTF-8 bytes.
    fn parse_string(&mut self) -> Result<Vec<u8>, i64> {
        self.i += 1; // consume opening '"'
        let mut out = Vec::new();
        loop {
            let c = self.peek().ok_or(JSON_ERROR_SYNTAX)?;
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.i += 1;
                    let e = self.peek().ok_or(JSON_ERROR_SYNTAX)?;
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => self.parse_unicode_escape(&mut out)?,
                        _ => return Err(JSON_ERROR_SYNTAX),
                    }
                }
                // Literal control characters are not allowed in JSON strings.
                0x00..=0x1f => return Err(JSON_ERROR_CTRL_CHAR),
                _ => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }

    /// Parse a `\uXXXX` escape (the `\u` already consumed), handling UTF-16
    /// surrogate pairs. Appends UTF-8 bytes to `out`.
    fn parse_unicode_escape(&mut self, out: &mut Vec<u8>) -> Result<(), i64> {
        let hi = self.read_hex4()?;
        let cp = if (0xD800..=0xDBFF).contains(&hi) {
            // High surrogate: require a following \uDC00..\uDFFF.
            if self.peek() == Some(b'\\') && self.s.get(self.i + 1) == Some(&b'u') {
                self.i += 2;
                let lo = self.read_hex4()?;
                if !(0xDC00..=0xDFFF).contains(&lo) {
                    return Err(JSON_ERROR_UTF16);
                }
                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
            } else {
                return Err(JSON_ERROR_UTF16);
            }
        } else if (0xDC00..=0xDFFF).contains(&hi) {
            // Lone low surrogate.
            return Err(JSON_ERROR_UTF16);
        } else {
            hi
        };
        match char::from_u32(cp) {
            Some(ch) => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                Ok(())
            }
            None => Err(JSON_ERROR_UTF16),
        }
    }

    fn read_hex4(&mut self) -> Result<u32, i64> {
        if self.i + 4 > self.s.len() {
            return Err(JSON_ERROR_SYNTAX);
        }
        let mut v: u32 = 0;
        for _ in 0..4 {
            let d = match self.s[self.i] {
                b'0'..=b'9' => (self.s[self.i] - b'0') as u32,
                b'a'..=b'f' => (self.s[self.i] - b'a' + 10) as u32,
                b'A'..=b'F' => (self.s[self.i] - b'A' + 10) as u32,
                _ => return Err(JSON_ERROR_SYNTAX),
            };
            v = v * 16 + d;
            self.i += 1;
        }
        Ok(v)
    }

    /// Parse a strict-JSON number: `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
    fn parse_number(&mut self) -> Result<Value, i64> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        // Integer part.
        match self.peek() {
            Some(b'0') => self.i += 1, // single leading zero only
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.i += 1;
                }
            }
            _ => return Err(JSON_ERROR_SYNTAX),
        }
        let mut is_float = false;
        // Fraction.
        if self.peek() == Some(b'.') {
            is_float = true;
            self.i += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(JSON_ERROR_SYNTAX);
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        // Exponent.
        if matches!(self.peek(), Some(b'e' | b'E')) {
            is_float = true;
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(JSON_ERROR_SYNTAX);
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        let text = std::str::from_utf8(&self.s[start..self.i]).unwrap();
        if is_float {
            return Ok(Value::Float(text.parse::<f64>().unwrap_or(f64::INFINITY)));
        }
        match text.parse::<i64>() {
            Ok(n) => Ok(Value::Int(n)),
            Err(_) => {
                // Integer out of i64 range.
                if self.bigint_as_string {
                    Ok(Value::str(text.as_bytes().to_vec()))
                } else {
                    Ok(Value::Float(text.parse::<f64>().unwrap_or(f64::INFINITY)))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// json_last_error / json_last_error_msg
// ---------------------------------------------------------------------------

fn fn_json_last_error(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(get_last_error()))
}

fn fn_json_last_error_msg(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(error_msg(get_last_error())))
}

// ---------------------------------------------------------------------------
// registration + constants
// ---------------------------------------------------------------------------

pub fn register(m: &mut std::collections::HashMap<&'static str, Builtin>) {
    macro_rules! reg {
        ($name:literal, $func:path, $min:literal, $max:expr) => {
            m.insert(
                $name,
                Builtin {
                    name: $name,
                    func: $func,
                    min_args: $min,
                    max_args: $max,
                },
            );
        };
    }
    reg!("json_encode", fn_json_encode, 1, Some(3));
    reg!("json_decode", fn_json_decode, 1, Some(4));
    reg!("json_last_error", fn_json_last_error, 0, Some(0));
    reg!("json_last_error_msg", fn_json_last_error_msg, 0, Some(0));
}

/// The `JSON_*` constants (chained into the engine's `constants()` table).
pub fn json_constants() -> Vec<(&'static str, Value)> {
    vec![
        ("JSON_ERROR_NONE", Value::Int(JSON_ERROR_NONE)),
        ("JSON_ERROR_DEPTH", Value::Int(JSON_ERROR_DEPTH)),
        (
            "JSON_ERROR_STATE_MISMATCH",
            Value::Int(JSON_ERROR_STATE_MISMATCH),
        ),
        ("JSON_ERROR_CTRL_CHAR", Value::Int(JSON_ERROR_CTRL_CHAR)),
        ("JSON_ERROR_SYNTAX", Value::Int(JSON_ERROR_SYNTAX)),
        ("JSON_ERROR_UTF8", Value::Int(JSON_ERROR_UTF8)),
        ("JSON_ERROR_RECURSION", Value::Int(JSON_ERROR_RECURSION)),
        ("JSON_ERROR_INF_OR_NAN", Value::Int(JSON_ERROR_INF_OR_NAN)),
        (
            "JSON_ERROR_UNSUPPORTED_TYPE",
            Value::Int(JSON_ERROR_UNSUPPORTED_TYPE),
        ),
        (
            "JSON_ERROR_INVALID_PROPERTY_NAME",
            Value::Int(JSON_ERROR_INVALID_PROPERTY_NAME),
        ),
        ("JSON_ERROR_UTF16", Value::Int(JSON_ERROR_UTF16)),
        (
            "JSON_ERROR_NON_BACKED_ENUM",
            Value::Int(JSON_ERROR_NON_BACKED_ENUM),
        ),
        ("JSON_HEX_TAG", Value::Int(JSON_HEX_TAG)),
        ("JSON_HEX_AMP", Value::Int(JSON_HEX_AMP)),
        ("JSON_HEX_APOS", Value::Int(JSON_HEX_APOS)),
        ("JSON_HEX_QUOT", Value::Int(JSON_HEX_QUOT)),
        ("JSON_FORCE_OBJECT", Value::Int(JSON_FORCE_OBJECT)),
        ("JSON_NUMERIC_CHECK", Value::Int(JSON_NUMERIC_CHECK)),
        ("JSON_UNESCAPED_SLASHES", Value::Int(JSON_UNESCAPED_SLASHES)),
        ("JSON_PRETTY_PRINT", Value::Int(JSON_PRETTY_PRINT)),
        ("JSON_UNESCAPED_UNICODE", Value::Int(JSON_UNESCAPED_UNICODE)),
        (
            "JSON_PARTIAL_OUTPUT_ON_ERROR",
            Value::Int(JSON_PARTIAL_OUTPUT_ON_ERROR),
        ),
        (
            "JSON_PRESERVE_ZERO_FRACTION",
            Value::Int(JSON_PRESERVE_ZERO_FRACTION),
        ),
        (
            "JSON_INVALID_UTF8_IGNORE",
            Value::Int(JSON_INVALID_UTF8_IGNORE),
        ),
        (
            "JSON_INVALID_UTF8_SUBSTITUTE",
            Value::Int(JSON_INVALID_UTF8_SUBSTITUTE),
        ),
        ("JSON_THROW_ON_ERROR", Value::Int(JSON_THROW_ON_ERROR)),
        ("JSON_OBJECT_AS_ARRAY", Value::Int(JSON_OBJECT_AS_ARRAY)),
        ("JSON_BIGINT_AS_STRING", Value::Int(JSON_BIGINT_AS_STRING)),
    ]
}

// ---------------------------------------------------------------------------
// tests — expected values captured from php 8.5.8
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct NoopHost;
    impl Host for NoopHost {
        fn write_out(&mut self, _: &[u8]) {}
    }

    fn s(b: &str) -> Value {
        Value::str(b)
    }

    fn enc(v: &Value, flags: i64) -> String {
        let r = fn_json_encode(&mut NoopHost, &[v.clone(), Value::Int(flags)]).unwrap();
        match r {
            Value::Str(s) => String::from_utf8(s.as_bytes().to_vec()).unwrap(),
            Value::Bool(false) => "<false>".to_string(),
            _ => unreachable!(),
        }
    }

    fn dec(json: &str) -> Value {
        fn_json_decode(&mut NoopHost, &[s(json)]).unwrap()
    }

    fn dec_f(json: &str, assoc: Value, depth: i64, flags: i64) -> Value {
        fn_json_decode(
            &mut NoopHost,
            &[s(json), assoc, Value::Int(depth), Value::Int(flags)],
        )
        .unwrap()
    }

    fn arr(items: &[Value]) -> Value {
        let mut a = PhpArray::new();
        for v in items {
            a.push(v.clone());
        }
        Value::Array(a)
    }

    fn map(pairs: &[(&str, Value)]) -> Value {
        let mut a = PhpArray::new();
        for (k, v) in pairs {
            a.insert(ArrayKey::from_bytes(k.as_bytes()), v.clone());
        }
        Value::Array(a)
    }

    // --- encode ----------------------------------------------------------

    #[test]
    fn encode_scalars_and_strings() {
        assert_eq!(enc(&Value::Null, 0), "null");
        assert_eq!(enc(&Value::Bool(true), 0), "true");
        assert_eq!(enc(&Value::Int(42), 0), "42");
        assert_eq!(enc(&Value::Float(1.0), 0), "1");
        assert_eq!(enc(&Value::Float(1.5), 0), "1.5");
        assert_eq!(enc(&Value::Float(1e20), 0), "1.0e+20");
        assert_eq!(enc(&Value::Float(f64::NAN), 0), "<false>");
        assert_eq!(enc(&s("a/b"), 0), "\"a\\/b\"");
        assert_eq!(enc(&s("café"), 0), "\"caf\\u00e9\"");
        assert_eq!(enc(&s("日本"), 0), "\"\\u65e5\\u672c\"");
        assert_eq!(enc(&s("emoji😀"), 0), "\"emoji\\ud83d\\ude00\"");
        assert_eq!(enc(&Value::str(vec![0xC3, 0x28]), 0), "<false>");
    }

    #[test]
    fn encode_flags() {
        assert_eq!(enc(&s("a/b"), JSON_UNESCAPED_SLASHES), "\"a/b\"");
        assert_eq!(enc(&s("café"), JSON_UNESCAPED_UNICODE), "\"café\"");
        assert_eq!(
            enc(&arr(&[s("1"), s("2.5"), s("x")]), JSON_NUMERIC_CHECK),
            "[1,2.5,\"x\"]"
        );
        assert_eq!(
            enc(&arr(&[Value::Int(1), Value::Int(2)]), JSON_FORCE_OBJECT),
            "{\"0\":1,\"1\":2}"
        );
    }

    #[test]
    fn encode_pretty_print() {
        assert_eq!(enc(&arr(&[]), JSON_PRETTY_PRINT), "[]");
        assert_eq!(
            enc(&map(&[("a", arr(&[]))]), JSON_PRETTY_PRINT),
            "{\n    \"a\": []\n}"
        );
        assert_eq!(
            enc(
                &arr(&[Value::Int(1), arr(&[Value::Int(2), Value::Int(3)])]),
                JSON_PRETTY_PRINT
            ),
            "[\n    1,\n    [\n        2,\n        3\n    ]\n]"
        );
        assert_eq!(
            enc(
                &map(&[
                    ("a", Value::Int(1)),
                    ("b", arr(&[Value::Int(1), Value::Int(2), Value::Int(3)])),
                    ("c", map(&[("x", Value::Bool(true)), ("y", Value::Null)])),
                    ("d", arr(&[])),
                ]),
                JSON_PRETTY_PRINT
            ),
            "{\n    \"a\": 1,\n    \"b\": [\n        1,\n        2,\n        3\n    ],\n    \"c\": {\n        \"x\": true,\n        \"y\": null\n    },\n    \"d\": []\n}"
        );
    }

    // --- decode ----------------------------------------------------------

    fn is_int(v: &Value, n: i64) -> bool {
        matches!(v, Value::Int(x) if *x == n)
    }

    #[test]
    fn decode_scalars_and_numbers() {
        assert!(is_int(&dec("123"), 123));
        assert!(matches!(dec("123.45"), Value::Float(f) if (f - 123.45).abs() < 1e-9));
        assert!(matches!(dec("1e3"), Value::Float(f) if f == 1000.0));
        assert!(is_int(&dec("-0"), 0));
        assert!(matches!(dec("0.0"), Value::Float(f) if f == 0.0));
        assert!(matches!(dec("true"), Value::Bool(true)));
        assert!(matches!(dec("false"), Value::Bool(false)));
        assert!(matches!(dec("null"), Value::Null));
        // overflow -> float by default
        assert!(matches!(dec("12345678901234567890"), Value::Float(_)));
        // BIGINT_AS_STRING -> string
        let big = dec_f(
            "12345678901234567890",
            Value::Null,
            512,
            JSON_BIGINT_AS_STRING,
        );
        assert!(matches!(&big, Value::Str(s) if s.as_bytes() == b"12345678901234567890"));
    }

    #[test]
    fn decode_strings_and_unicode() {
        assert!(matches!(dec("\"hi\\u0041\""), Value::Str(s) if s.as_bytes() == b"hiA"));
        // surrogate pair -> 😀 (U+1F600)
        assert!(
            matches!(dec("\"\\ud83d\\ude00\""), Value::Str(s) if s.as_bytes() == "😀".as_bytes())
        );
        // lone surrogate -> error -> null
        assert!(matches!(dec("\"\\ud83d\""), Value::Null));
        assert_eq!(get_last_error(), JSON_ERROR_UTF16);
    }

    #[test]
    fn decode_containers() {
        assert!(matches!(dec("[1,2,3]"), Value::Array(a) if a.len() == 3));
        assert!(matches!(dec("  [1,2]  "), Value::Array(a) if a.len() == 2));
        let obj = dec("{\"a\":1,\"b\":2}");
        assert!(matches!(&obj, Value::Array(a) if a.len() == 2));
        // duplicate key -> last wins
        let dup = dec("{\"a\":1,\"a\":2}");
        if let Value::Array(a) = dup {
            assert_eq!(a.len(), 1);
            assert!(is_int(a.get(&ArrayKey::from_bytes(b"a")).unwrap(), 2));
        } else {
            panic!("expected array");
        }
    }

    #[test]
    fn decode_errors() {
        assert!(matches!(dec("[1,2] x"), Value::Null));
        assert_eq!(get_last_error(), JSON_ERROR_SYNTAX);
        assert!(matches!(dec("[1,2"), Value::Null));
        assert!(matches!(dec("{bad}"), Value::Null));
        assert!(matches!(dec(""), Value::Null));
        assert_eq!(get_last_error(), JSON_ERROR_SYNTAX);
        assert!(matches!(dec("01"), Value::Null));
        assert!(matches!(dec("[1,]"), Value::Null));
        // control char in string
        assert!(matches!(dec("\"a\nb\""), Value::Null));
        assert_eq!(get_last_error(), JSON_ERROR_CTRL_CHAR);
    }

    #[test]
    fn decode_depth_limit() {
        // [1] is 1 level deep: needs depth >= 2.
        assert!(matches!(dec_f("[1]", Value::Null, 1, 0), Value::Null));
        assert_eq!(get_last_error(), JSON_ERROR_DEPTH);
        assert!(matches!(dec_f("[1]", Value::Null, 2, 0), Value::Array(_)));
        assert!(matches!(dec_f("[[1]]", Value::Null, 2, 0), Value::Null));
        assert!(matches!(dec_f("[[1]]", Value::Null, 3, 0), Value::Array(_)));
        assert!(matches!(dec_f("5", Value::Null, 1, 0), Value::Int(5)));
    }

    #[test]
    fn decode_round_trip_with_encode() {
        let original = map(&[
            ("name", s("café")),
            ("nums", arr(&[Value::Int(1), Value::Int(2), Value::Int(3)])),
            ("nested", map(&[("ok", Value::Bool(true))])),
        ]);
        let encoded = enc(&original, 0);
        let decoded = dec(&encoded);
        // re-encode the decoded value; should match the first encoding.
        assert_eq!(enc(&decoded, 0), encoded);
    }

    #[test]
    fn last_error_msg_roundtrip() {
        dec("{bad}");
        assert!(matches!(
            fn_json_last_error(&mut NoopHost, &[]).unwrap(),
            Value::Int(x) if x == JSON_ERROR_SYNTAX
        ));
        let msg = fn_json_last_error_msg(&mut NoopHost, &[]).unwrap();
        assert!(matches!(msg, Value::Str(s) if s.as_bytes() == b"Syntax error"));
        dec("[1]");
        assert!(matches!(
            fn_json_last_error(&mut NoopHost, &[]).unwrap(),
            Value::Int(0)
        ));
    }
}
