//! Byte-exact `var_dump` / `print_r` / `var_export` formatters.
//!
//! Indentation and punctuation verified against php 8.5.8
//! (tests/golden/{var_dump,print_r,var_export}_nested.out).

use rhp_values::conv;
use rhp_values::{fmt, ArrayKey, PhpStr, Value};

fn sp(n: usize, out: &mut Vec<u8>) {
    out.extend(std::iter::repeat(b' ').take(n));
}

/// `var_dump` of one value at the given indentation (spaces). `host` supplies
/// class names / enum-ness for object rendering.
pub fn var_dump(host: &dyn crate::Host, out: &mut Vec<u8>, v: &Value, indent: usize) {
    let v = deref(v);
    sp(indent, out);
    match &v {
        Value::Null => out.extend_from_slice(b"NULL\n"),
        Value::Bool(b) => {
            out.extend_from_slice(if *b { b"bool(true)\n" } else { b"bool(false)\n" })
        }
        Value::Int(n) => out.extend_from_slice(format!("int({n})\n").as_bytes()),
        Value::Float(f) => {
            out.extend_from_slice(b"float(");
            out.extend_from_slice(&fmt::float_serialize(*f));
            out.extend_from_slice(b")\n");
        }
        Value::Str(s) => {
            out.extend_from_slice(format!("string({}) \"", s.len()).as_bytes());
            out.extend_from_slice(s.as_bytes());
            out.extend_from_slice(b"\"\n");
        }
        Value::Array(a) => {
            out.extend_from_slice(format!("array({}) {{\n", a.len()).as_bytes());
            for (k, val) in a.iter() {
                sp(indent + 2, out);
                dump_key(out, k);
                out.extend_from_slice(b"=>\n");
                var_dump(host, out, val, indent + 2);
            }
            sp(indent, out);
            out.extend_from_slice(b"}\n");
        }
        Value::Object(h) => {
            // Snapshot under a short borrow so recursion can't double-borrow.
            let (class_id, obj_id, props) = {
                let b = h.borrow();
                let props: Vec<(PhpStr, Value)> =
                    b.properties.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                (b.class, b.id, props)
            };
            let cname = host.class_name(class_id).unwrap_or_else(|| b"class".to_vec());
            if host.is_enum_class(class_id) {
                // `enum(Name::Case)` — the case is the object's `name` property.
                let case = props
                    .iter()
                    .find(|(k, _)| k.as_bytes() == b"name")
                    .map(|(_, v)| conv::to_string_bytes(v))
                    .unwrap_or_default();
                out.extend_from_slice(b"enum(");
                out.extend_from_slice(&cname);
                out.extend_from_slice(b"::");
                out.extend_from_slice(&case);
                out.extend_from_slice(b")\n");
            } else {
                out.extend_from_slice(
                    format!(
                        "object({})#{} ({}) {{\n",
                        String::from_utf8_lossy(&cname),
                        obj_id,
                        props.len()
                    )
                    .as_bytes(),
                );
                for (k, val) in &props {
                    sp(indent + 2, out);
                    out.extend_from_slice(b"[\"");
                    out.extend_from_slice(k.as_bytes());
                    out.extend_from_slice(b"\"]=>\n");
                    var_dump(host, out, val, indent + 2);
                }
                sp(indent, out);
                out.extend_from_slice(b"}\n");
            }
        }
        Value::Closure(_) => {
            out.extend_from_slice(b"object(Closure)#0 (0) {\n");
            sp(indent, out);
            out.extend_from_slice(b"}\n");
        }
        Value::Ref(_) => unreachable!("deref'd above"),
    }
}

fn dump_key(out: &mut Vec<u8>, k: &ArrayKey) {
    match k {
        ArrayKey::Int(n) => out.extend_from_slice(format!("[{n}]").as_bytes()),
        ArrayKey::Str(s) => {
            out.extend_from_slice(b"[\"");
            out.extend_from_slice(s.as_bytes());
            out.extend_from_slice(b"\"]");
        }
    }
}

/// `print_r` of one value. `indent` = column of the opening `(` for arrays.
pub fn print_r(out: &mut Vec<u8>, v: &Value, indent: usize) {
    let v = deref(v);
    match &v {
        Value::Array(a) => {
            out.extend_from_slice(b"Array\n");
            sp(indent, out);
            out.extend_from_slice(b"(\n");
            for (k, val) in a.iter() {
                sp(indent + 4, out);
                out.push(b'[');
                print_r_key(out, k);
                out.extend_from_slice(b"] => ");
                if matches!(deref(val), Value::Array(_)) {
                    print_r(out, val, indent + 8);
                    out.push(b'\n');
                } else {
                    out.extend_from_slice(&conv::to_string_bytes(val));
                    out.push(b'\n');
                }
            }
            sp(indent, out);
            out.extend_from_slice(b")\n");
        }
        scalar => out.extend_from_slice(&conv::to_string_bytes(scalar)),
    }
}

fn print_r_key(out: &mut Vec<u8>, k: &ArrayKey) {
    match k {
        ArrayKey::Int(n) => out.extend_from_slice(n.to_string().as_bytes()),
        ArrayKey::Str(s) => out.extend_from_slice(s.as_bytes()),
    }
}

/// `var_export` of one value. `indent` = leading spaces of this array's lines.
pub fn var_export(out: &mut Vec<u8>, v: &Value, indent: usize) {
    let v = deref(v);
    match &v {
        Value::Null => out.extend_from_slice(b"NULL"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Int(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::Float(f) => out.extend_from_slice(&export_float(*f)),
        Value::Str(s) => export_string(out, s.as_bytes()),
        Value::Array(a) => {
            out.extend_from_slice(b"array (\n");
            for (k, val) in a.iter() {
                sp(indent + 2, out);
                export_key(out, k);
                out.extend_from_slice(b" => ");
                if matches!(deref(val), Value::Array(_)) {
                    out.push(b'\n');
                    sp(indent + 2, out);
                    var_export(out, val, indent + 2);
                    out.extend_from_slice(b",\n");
                } else {
                    var_export(out, val, indent + 2);
                    out.extend_from_slice(b",\n");
                }
            }
            sp(indent, out);
            out.push(b')');
        }
        Value::Object(_) | Value::Closure(_) => out.extend_from_slice(b"NULL"),
        Value::Ref(_) => unreachable!("deref'd above"),
    }
}

fn export_key(out: &mut Vec<u8>, k: &ArrayKey) {
    match k {
        ArrayKey::Int(n) => out.extend_from_slice(n.to_string().as_bytes()),
        ArrayKey::Str(s) => export_string(out, s.as_bytes()),
    }
}

/// Single-quoted string literal with `\` and `'` escaped (var_export style).
fn export_string(out: &mut Vec<u8>, s: &[u8]) {
    out.push(b'\'');
    for &b in s {
        if b == b'\\' || b == b'\'' {
            out.push(b'\\');
        }
        out.push(b);
    }
    out.push(b'\'');
}

/// var_export floats always look like floats (append `.0` if integer-valued).
fn export_float(f: f64) -> Vec<u8> {
    let s = fmt::float_serialize(f);
    if s.iter().any(|&b| matches!(b, b'.' | b'e' | b'E' | b'N' | b'I')) {
        s
    } else {
        [s, b".0".to_vec()].concat()
    }
}

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        other => other.clone(),
    }
}
