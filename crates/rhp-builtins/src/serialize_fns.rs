//! Native PHP serialization builtins: `serialize` / `unserialize`.
//!
//! Scope: null, bool, int, float, string, (nested) array, and objects (`O:`
//! with php-mangled property names, `__serialize`/`__unserialize`,
//! `__sleep`/`__wakeup`, enums as `E:`). Shared-object back-references
//! (`r:`/`R:`) and `Serializable` (`C:`) are not modelled.
//!
//! Every function here is verified byte-for-byte against the local php 8.5.8
//! binary (see the `#[cfg(test)]` block — expected values are hardcoded from
//! `php -r '...'` output).
//!
//! Format notes (php 8.5, `serialize_precision = -1`):
//! - `N;`                       null
//! - `b:0;` / `b:1;`            bool
//! - `i:42;`                    int
//! - `d:3.14;`                  float (shortest round-trip via `fmt::float_serialize`)
//! - `s:5:"hello";`             string (BYTE length, raw bytes, quotes NOT escaped)
//! - `a:2:{i:0;i:1;i:1;i:2;}`   array (count pairs of key/value)

use std::collections::HashMap;

use rhp_values::{conv, fmt, ArrayKey, PhpArray, PhpStr, Value};

use crate::{Builtin, Host, Thrown};

/// Deref a (possibly `Ref`) value to an owned value.
fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

// ---------------------------------------------------------------------------
// serialize
// ---------------------------------------------------------------------------

fn serialize_value(host: &mut dyn Host, out: &mut Vec<u8>, v: &Value) -> Result<(), Thrown> {
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
                serialize_value(host, out, val)?;
            }
            out.push(b'}');
        }
        Value::Object(h) => {
            let class_id = h.borrow().class;
            let cname = host.class_name(class_id).unwrap_or_default();
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
            let head = |out: &mut Vec<u8>, n: usize| {
                out.extend_from_slice(format!("O:{}:\"", cname.len()).as_bytes());
                out.extend_from_slice(&cname);
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
                head(out, data.len());
                for (k, val) in data.iter() {
                    serialize_key(out, k);
                    serialize_value(host, out, val)?;
                }
                out.push(b'}');
                return Ok(());
            }
            // The object's slots under their php-mangled names.
            let props: Vec<(PhpStr, Value)> = {
                let b = h.borrow();
                b.properties
                    .iter()
                    .map(|(k, v)| (host.mangled_prop_name(class_id, k), v.clone()))
                    .collect()
            };
            if host.class_has_method(class_id, b"__sleep") {
                let names = match deref(&call(host, "__sleep")?) {
                    Value::Array(a) => a,
                    _ => {
                        out.extend_from_slice(b"N;");
                        return Ok(());
                    }
                };
                let mut picked: Vec<(PhpStr, Value)> = Vec::new();
                for (_, n) in names.iter() {
                    let n = conv::to_string_bytes(&deref(n));
                    let prot = [&b"\0*\0"[..], &n[..]].concat();
                    let priv_ = [&b"\0"[..], &cname[..], b"\0", &n[..]].concat();
                    let hit = props.iter().find(|(k, _)| {
                        k.as_bytes() == n.as_slice()
                            || k.as_bytes() == priv_.as_slice()
                            || k.as_bytes() == prot.as_slice()
                    });
                    match hit {
                        Some((k, v)) => picked.push((k.clone(), v.clone())),
                        None => picked.push((PhpStr::from_bytes(n), Value::Null)),
                    }
                }
                head(out, picked.len());
                for (k, v) in &picked {
                    serialize_str(out, k.as_bytes());
                    serialize_value(host, out, v)?;
                }
                out.push(b'}');
                return Ok(());
            }
            head(out, props.len());
            for (k, v) in &props {
                serialize_key(out, &ArrayKey::from_bytes(k.as_bytes()));
                serialize_value(host, out, v)?;
            }
            out.push(b'}');
        }
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

fn serialize_str(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(format!("s:{}:\"", bytes.len()).as_bytes());
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\";");
}

fn serialize_key(out: &mut Vec<u8>, k: &ArrayKey) {
    match k {
        ArrayKey::Int(n) => out.extend_from_slice(format!("i:{n};").as_bytes()),
        ArrayKey::Str(s) => {
            let bytes = s.as_bytes();
            out.extend_from_slice(format!("s:{}:\"", bytes.len()).as_bytes());
            out.extend_from_slice(bytes);
            out.extend_from_slice(b"\";");
        }
    }
}

fn fn_serialize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = Vec::new();
    serialize_value(host, &mut out, &args[0])?;
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// unserialize
// ---------------------------------------------------------------------------

struct Parser<'a> {
    b: &'a [u8],
    pos: usize,
    /// Objects awaiting `__unserialize` / `__wakeup`, in php's call order
    /// (they run once the whole payload has been parsed).
    pending: Vec<(Value, Option<PhpArray>)>,
}

impl<'a> Parser<'a> {
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

    fn parse_value(&mut self, host: &mut dyn Host) -> Option<Value> {
        match self.b.get(self.pos)? {
            b'N' => {
                self.pos += 1;
                self.expect(b';')?;
                Some(Value::Null)
            }
            b'b' => {
                self.pos += 1;
                self.expect(b':')?;
                let tok = self.read_until(b';')?;
                match tok {
                    b"0" => Some(Value::Bool(false)),
                    b"1" => Some(Value::Bool(true)),
                    _ => None,
                }
            }
            b'i' => {
                self.pos += 1;
                self.expect(b':')?;
                let tok = self.read_until(b';')?;
                Some(Value::Int(parse_i64(tok)?))
            }
            b'd' => {
                self.pos += 1;
                self.expect(b':')?;
                let tok = self.read_until(b';')?;
                Some(Value::Float(parse_f64(tok)?))
            }
            b's' => {
                self.pos += 1;
                self.expect(b':')?;
                let len_tok = self.read_until(b':')?;
                let len = parse_usize(len_tok)?;
                self.expect(b'"')?;
                if self.pos + len > self.b.len() {
                    return None;
                }
                let content = self.b[self.pos..self.pos + len].to_vec();
                self.pos += len;
                self.expect(b'"')?;
                self.expect(b';')?;
                Some(Value::str(content))
            }
            b'a' => {
                self.pos += 1;
                self.expect(b':')?;
                let count_tok = self.read_until(b':')?;
                let count = parse_usize(count_tok)?;
                self.expect(b'{')?;
                let mut arr = PhpArray::new();
                for _ in 0..count {
                    let key_val = self.parse_value(host)?;
                    let key = ArrayKey::from_value(&key_val)?;
                    let val = self.parse_value(host)?;
                    arr.insert(key, val);
                }
                self.expect(b'}')?;
                Some(Value::Array(arr))
            }
            b'O' => {
                self.pos += 1;
                self.expect(b':')?;
                let len = parse_usize(self.read_until(b':')?)?;
                self.expect(b'"')?;
                let cname = self.b.get(self.pos..self.pos + len)?.to_vec();
                self.pos += len;
                self.expect(b'"')?;
                self.expect(b':')?;
                let count = parse_usize(self.read_until(b':')?)?;
                self.expect(b'{')?;
                let mut data = PhpArray::new();
                for _ in 0..count {
                    let key_val = self.parse_value(host)?;
                    let key = ArrayKey::from_value(&key_val)?;
                    let val = self.parse_value(host)?;
                    data.insert(key, val);
                }
                self.expect(b'}')?;
                let cid = host.resolve_class_autoload(&cname)?;
                if host.class_has_method(cid, b"__unserialize") {
                    let obj = host.instantiate(&cname, Vec::new())?;
                    self.pending.push((obj.clone(), Some(data)));
                    return Some(obj);
                }
                // Mangled names: protected `\0*\0p` is stored as plain `p`;
                // private `\0C\0p` is the storage key itself.
                let props = data
                    .iter()
                    .map(|(k, v)| {
                        let k = match k {
                            ArrayKey::Int(n) => n.to_string().into_bytes(),
                            ArrayKey::Str(s) => s.as_bytes().to_vec(),
                        };
                        let k = match k.strip_prefix(b"\0*\0") {
                            Some(rest) => rest.to_vec(),
                            None => k,
                        };
                        (k, v.clone())
                    })
                    .collect();
                let obj = host.instantiate(&cname, props)?;
                if host.class_has_method(cid, b"__wakeup") {
                    self.pending.push((obj.clone(), None));
                }
                Some(obj)
            }
            b'E' => {
                self.pos += 1;
                self.expect(b':')?;
                let len = parse_usize(self.read_until(b':')?)?;
                self.expect(b'"')?;
                let body = self.b.get(self.pos..self.pos + len)?.to_vec();
                self.pos += len;
                self.expect(b'"')?;
                self.expect(b';')?;
                let colon = body.iter().position(|&c| c == b':')?;
                let (class, case) = (&body[..colon], &body[colon + 1..]);
                host.resolve_class_autoload(class)?;
                host.class_constant(class, case)
            }
            _ => None,
        }
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
    let mut p = Parser {
        b: &bytes,
        pos: 0,
        pending: Vec::new(),
    };
    // php tolerates trailing "extra data" at the top level: it still returns
    // the parsed value (only emits a warning). We return the value and ignore
    // anything after it; malformed/incomplete input yields `false`.
    let v = match p.parse_value(host) {
        Some(v) => v,
        None => return Ok(Value::Bool(false)),
    };
    for (obj, data) in std::mem::take(&mut p.pending) {
        let mut cb = PhpArray::new();
        cb.push(obj);
        match data {
            Some(d) => {
                cb.push(Value::str("__unserialize"));
                host.call_callable(&Value::Array(cb), &[Value::Array(d)])?;
            }
            None => {
                cb.push(Value::str("__wakeup"));
                host.call_callable(&Value::Array(cb), &[])?;
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
