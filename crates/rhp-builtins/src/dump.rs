//! Byte-exact `var_dump` / `print_r` / `var_export` formatters.
//!
//! Indentation and punctuation verified against php 8.5.8
//! (tests/golden/{var_dump,print_r,var_export}_nested.out).

use rhp_values::conv;
use rhp_values::{fmt, ArrayKey, PhpStr, Value};

fn sp(n: usize, out: &mut Vec<u8>) {
    out.extend(std::iter::repeat_n(b' ', n));
}

/// Split a php-mangled property name: `"\0Class\0name"` -> (name, Some(Class)),
/// `"\0*\0name"` -> (name, Some("*")), plain `name` -> (name, None).
pub fn unmangle_prop(key: &[u8]) -> (&[u8], Option<&[u8]>) {
    if key.first() == Some(&0) {
        if let Some(end) = key[1..].iter().position(|&b| b == 0) {
            return (&key[end + 2..], Some(&key[1..end + 1]));
        }
    }
    (key, None)
}

/// An object's properties keyed by their php-mangled names (debug view).
fn mangled_props(host: &dyn crate::Host, h: &rhp_values::ObjectHandle) -> Vec<(PhpStr, Value)> {
    let b = h.borrow();
    b.properties
        .iter()
        .map(|(k, v)| (host.mangled_prop_name(b.class, k), v.clone()))
        .collect()
}

/// `var_dump` of one value at the given indentation (spaces). `host` supplies
/// class names / enum-ness for object rendering.
pub fn var_dump(host: &dyn crate::Host, out: &mut Vec<u8>, v: &Value, indent: usize) {
    let v = deref(v);
    sp(indent, out);
    match &v {
        Value::Null => out.extend_from_slice(b"NULL\n"),
        Value::Bool(b) => out.extend_from_slice(if *b {
            b"bool(true)\n"
        } else {
            b"bool(false)\n"
        }),
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
            let props = mangled_props(host, h);
            let (class_id, obj_id) = {
                let b = h.borrow();
                (b.class, b.id)
            };
            let cname = host
                .class_name(class_id)
                .unwrap_or_else(|| b"class".to_vec());
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
                    let (name, owner) = unmangle_prop(k.as_bytes());
                    out.extend_from_slice(name);
                    out.push(b'"');
                    match owner {
                        None => {}
                        Some(b"*") => out.extend_from_slice(b":protected"),
                        Some(class) => {
                            out.extend_from_slice(b":\"");
                            out.extend_from_slice(class);
                            out.extend_from_slice(b"\":private");
                        }
                    }
                    out.extend_from_slice(b"]=>\n");
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
        Value::Resource(r) => {
            out.extend_from_slice(
                format!("resource({}) of type ({})\n", r.id, r.type_name).as_bytes(),
            );
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
pub fn print_r(host: &dyn crate::Host, out: &mut Vec<u8>, v: &Value, indent: usize) {
    print_r_value(host, out, v, indent, &mut Vec::new());
}

/// `print_r` of one value; `printing` holds the ids of the objects currently
/// being printed, so a cycle prints ` *RECURSION*` like php instead of
/// recursing forever.
fn print_r_value(
    host: &dyn crate::Host,
    out: &mut Vec<u8>,
    v: &Value,
    indent: usize,
    printing: &mut Vec<u32>,
) {
    let v = deref(v);
    match &v {
        Value::Array(a) => {
            out.extend_from_slice(b"Array\n");
            let entries: Vec<(Vec<u8>, Value)> = a
                .iter()
                .map(|(k, val)| (print_r_key(k), val.clone()))
                .collect();
            print_r_block(host, out, &entries, indent, printing);
        }
        Value::Object(h) => {
            // Snapshot under a short borrow so recursion can't double-borrow.
            let entries: Vec<(Vec<u8>, Value)> = mangled_props(host, h)
                .into_iter()
                .map(|(k, v)| {
                    let label = match unmangle_prop(k.as_bytes()) {
                        (name, None) => name.to_vec(),
                        (name, Some(b"*")) => [name, b":protected"].concat(),
                        (name, Some(class)) => [name, b":", class, b":private"].concat(),
                    };
                    (label, v)
                })
                .collect();
            let (class_id, obj_id) = {
                let b = h.borrow();
                (b.class, b.id)
            };
            out.extend_from_slice(
                &host
                    .class_name(class_id)
                    .unwrap_or_else(|| b"class".to_vec()),
            );
            if host.is_enum_class(class_id) {
                out.extend_from_slice(b" Enum");
                match entries
                    .iter()
                    .find(|(k, _)| k == b"value")
                    .map(|(_, v)| deref(v))
                {
                    Some(Value::Int(_)) => out.extend_from_slice(b":int"),
                    Some(Value::Str(_)) => out.extend_from_slice(b":string"),
                    _ => {}
                }
                out.push(b'\n');
            } else {
                out.extend_from_slice(b" Object\n");
            }
            if printing.contains(&obj_id) {
                out.extend_from_slice(b" *RECURSION*");
                return;
            }
            printing.push(obj_id);
            print_r_block(host, out, &entries, indent, printing);
            printing.pop();
        }
        scalar => out.extend_from_slice(&conv::to_string_bytes(scalar)),
    }
}

/// The `(\n    [key] => value\n ... )\n` body shared by arrays and objects.
fn print_r_block(
    host: &dyn crate::Host,
    out: &mut Vec<u8>,
    entries: &[(Vec<u8>, Value)],
    indent: usize,
    printing: &mut Vec<u32>,
) {
    sp(indent, out);
    out.extend_from_slice(b"(\n");
    for (k, val) in entries {
        sp(indent + 4, out);
        out.push(b'[');
        out.extend_from_slice(k);
        out.extend_from_slice(b"] => ");
        if matches!(deref(val), Value::Array(_) | Value::Object(_)) {
            print_r_value(host, out, val, indent + 8, printing);
        } else {
            out.extend_from_slice(&conv::to_string_bytes(val));
        }
        out.push(b'\n');
    }
    sp(indent, out);
    out.extend_from_slice(b")\n");
}

fn print_r_key(k: &ArrayKey) -> Vec<u8> {
    match k {
        ArrayKey::Int(n) => n.to_string().into_bytes(),
        ArrayKey::Str(s) => s.as_bytes().to_vec(),
    }
}

/// `var_export` of one value — php's `php_var_export_ex`; `level` starts at 1.
pub fn var_export(host: &dyn crate::Host, out: &mut Vec<u8>, v: &Value, level: usize) {
    let v = deref(v);
    match &v {
        Value::Null => out.extend_from_slice(b"NULL"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        // i64::MIN as a literal would parse as a float.
        Value::Int(i64::MIN) => out.extend_from_slice(b"-9223372036854775807-1"),
        Value::Int(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::Float(f) => out.extend_from_slice(&export_float(*f)),
        Value::Str(s) => export_string(out, s.as_bytes()),
        Value::Array(a) => {
            if level > 1 {
                out.push(b'\n');
                sp(level - 1, out);
            }
            out.extend_from_slice(b"array (\n");
            for (k, val) in a.iter() {
                sp(level + 1, out);
                export_key(out, k);
                out.extend_from_slice(b" => ");
                var_export(host, out, val, level + 2);
                out.extend_from_slice(b",\n");
            }
            if level > 1 {
                sp(level - 1, out);
            }
            out.push(b')');
        }
        Value::Object(h) => {
            let class_id = h.borrow().class;
            let cname = host
                .class_name(class_id)
                .unwrap_or_else(|| b"class".to_vec());
            let is_enum = host.is_enum_class(class_id);
            let props = mangled_props(host, h);
            if level > 1 {
                out.push(b'\n');
                sp(level - 1, out);
            }
            let is_std = cname.eq_ignore_ascii_case(b"stdClass");
            if is_std {
                out.extend_from_slice(b"(object) array(\n");
            } else {
                out.push(b'\\');
                out.extend_from_slice(&cname);
                if is_enum {
                    let case = props
                        .iter()
                        .find(|(k, _)| k.as_bytes() == b"name")
                        .map(|(_, v)| conv::to_string_bytes(v))
                        .unwrap_or_default();
                    out.extend_from_slice(b"::");
                    out.extend_from_slice(&case);
                } else {
                    out.extend_from_slice(b"::__set_state(array(\n");
                }
            }
            if !is_enum {
                for (k, val) in &props {
                    sp(level + 2, out);
                    let (name, _) = unmangle_prop(k.as_bytes());
                    // Property names are always string keys (even "0").
                    out.push(b'\'');
                    for &b in name {
                        if b == b'\\' || b == b'\'' {
                            out.push(b'\\');
                        }
                        out.push(b);
                    }
                    out.push(b'\'');
                    out.extend_from_slice(b" => ");
                    var_export(host, out, val, level + 2);
                    out.extend_from_slice(b",\n");
                }
                if level > 1 {
                    sp(level - 1, out);
                }
            }
            if is_std {
                out.push(b')');
            } else if !is_enum {
                out.extend_from_slice(b"))");
            }
        }
        Value::Closure(_) => {
            // A Closure exports as `\Closure::__set_state(array(\n))`.
            if level > 1 {
                out.push(b'\n');
                sp(level - 1, out);
            }
            out.extend_from_slice(b"\\Closure::__set_state(array(\n");
            if level > 1 {
                sp(level - 1, out);
            }
            out.extend_from_slice(b"))");
        }
        Value::Resource(_) => out.extend_from_slice(b"NULL"),
        Value::Ref(_) => unreachable!("deref'd above"),
    }
}

fn export_key(out: &mut Vec<u8>, k: &ArrayKey) {
    match k {
        ArrayKey::Int(n) => out.extend_from_slice(n.to_string().as_bytes()),
        ArrayKey::Str(s) => export_string(out, s.as_bytes()),
    }
}

/// Single-quoted string literal with `\` and `'` escaped and NUL bytes spliced
/// as `' . "\0" . '` (var_export style).
fn export_string(out: &mut Vec<u8>, s: &[u8]) {
    out.push(b'\'');
    for &b in s {
        if b == 0 {
            out.extend_from_slice(b"' . \"\\0\" . '");
            continue;
        }
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
    if s.iter()
        .any(|&b| matches!(b, b'.' | b'e' | b'E' | b'N' | b'I'))
    {
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
