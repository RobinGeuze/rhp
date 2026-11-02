//! The `xml` extension: php's expat-compatible SAX parser (`xml_parser_*`),
//! which php builds on libxml2 (`XML_SAX_IMPL` is "libxml"). The native
//! side runs the shared parser in SAX mode and hands back the callback
//! sequence (cut off at the first fatal error, like libxml2's disableSAX);
//! the `XMLParser` class and the functions are PHP-source ([`PRELUDE_PHP`]).

use std::collections::HashMap;

use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::ext::libxml::parser::{self, SaxEv};
use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(deref).unwrap_or(Value::Null)
}

fn s(b: impl Into<Vec<u8>>) -> Value {
    Value::str(b)
}

fn list(items: Vec<Value>) -> Value {
    let mut a = PhpArray::new();
    for i in items {
        a.push(i);
    }
    Value::Array(a)
}

/// Line / column / byte offset at `pos` of `data`.
fn position(data: &[u8], pos: usize) -> (i64, i64) {
    let mut line = 1;
    let mut col = 1;
    for &c in &data[..pos.min(data.len())] {
        if c == b'\n' {
            line += 1;
            col = 1;
        } else if !(0x80..0xC0).contains(&c) {
            col += 1;
        }
    }
    (line, col)
}

/// `__xmlp_parse($data, $nsMode, $separator)`: ['ev' => events, 'err' =>
/// null | [code, line, column, byte], 'end' => [line, column, byte]].
fn fn_parse(_host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = conv::to_string_bytes(&arg(args, 0));
    let ns_mode = conv::to_bool(&arg(args, 1));
    let sep = conv::to_string_bytes(&arg(args, 2));
    let (events, errs, fatal_at) = parser::parse_sax(&data);
    let name = |ns: &Option<Vec<u8>>, local: &[u8], qname: &[u8]| -> Vec<u8> {
        if !ns_mode {
            return qname.to_vec();
        }
        match ns {
            Some(u) if !u.is_empty() => [&u[..], &sep[..], local].concat(),
            _ => local.to_vec(),
        }
    };
    let mut out = Vec::new();
    for ev in events {
        match ev {
            SaxEv::Start {
                qname,
                ns,
                local,
                raw_attrs,
                attrs,
                nsdecls,
            } => {
                if ns_mode {
                    for (p, u) in &nsdecls {
                        let pv = if p.is_empty() {
                            Value::Null
                        } else {
                            s(p.clone())
                        };
                        out.push(list(vec![s(b"ns".to_vec()), pv, s(u.clone())]));
                    }
                }
                let mut a = Vec::new();
                if ns_mode {
                    for (ans, al, v) in &attrs {
                        a.push(list(vec![s(name(ans, al, al)), s(v.clone())]));
                    }
                } else {
                    for (q, v) in &raw_attrs {
                        a.push(list(vec![s(q.clone()), s(v.clone())]));
                    }
                }
                out.push(list(vec![
                    s(b"s".to_vec()),
                    s(name(&ns, &local, &qname)),
                    list(a),
                ]));
            }
            SaxEv::End {
                qname,
                ns,
                local,
                nsdecls,
            } => {
                out.push(list(vec![s(b"e".to_vec()), s(name(&ns, &local, &qname))]));
                if ns_mode {
                    for p in nsdecls {
                        let pv = if p.is_empty() { Value::Null } else { s(p) };
                        out.push(list(vec![s(b"nse".to_vec()), pv]));
                    }
                }
            }
            SaxEv::Chars(d) => out.push(list(vec![s(b"c".to_vec()), s(d)])),
            SaxEv::Pi(t, d) => out.push(list(vec![s(b"pi".to_vec()), s(t), s(d)])),
            SaxEv::Comment(c) => out.push(list(vec![
                s(b"d".to_vec()),
                s([b"<!--".as_slice(), &c, b"-->"].concat()),
            ])),
        }
    }
    let mut r = PhpArray::new();
    r.insert(ArrayKey::from_bytes(b"ev"), list(out));
    let err = errs.iter().find(|e| e.level == 3);
    let errv = match err {
        Some(e) => {
            // libxml2 resets the input when it halts on these
            let halts = matches!(e.code, 1 | 4 | 5 | 9 | 32);
            let byte = if halts {
                0
            } else {
                fatal_at.unwrap_or(0) as i64
            };
            list(vec![
                Value::Int(e.code as i64),
                Value::Int(e.line as i64),
                Value::Int(e.column as i64),
                Value::Int(byte),
                Value::Int(fatal_at.unwrap_or(0) as i64),
            ])
        }
        None => Value::Null,
    };
    r.insert(ArrayKey::from_bytes(b"err"), errv);
    let (line, col) = position(&data, data.len());
    r.insert(
        ArrayKey::from_bytes(b"end"),
        list(vec![
            Value::Int(line),
            Value::Int(col),
            Value::Int(data.len() as i64),
        ]),
    );
    Ok(Value::Array(r))
}

/// php's error_mapping (ext/xml/compat.c), indexed by libxml2 error code.
const ERRORS: &[&str] = &[
    "No error",
    "No memory",
    "Invalid document start",
    "Empty document",
    "Not well-formed (invalid token)",
    "Invalid document end",
    "Invalid hexadecimal character reference",
    "Invalid decimal character reference",
    "Invalid character reference",
    "Invalid character",
    "XML_ERR_CHARREF_AT_EOF",
    "XML_ERR_CHARREF_IN_PROLOG",
    "XML_ERR_CHARREF_IN_EPILOG",
    "XML_ERR_CHARREF_IN_DTD",
    "XML_ERR_ENTITYREF_AT_EOF",
    "XML_ERR_ENTITYREF_IN_PROLOG",
    "XML_ERR_ENTITYREF_IN_EPILOG",
    "XML_ERR_ENTITYREF_IN_DTD",
    "PEReference at end of document",
    "PEReference in prolog",
    "PEReference in epilog",
    "PEReference: forbidden within markup decl in internal subset",
    "XML_ERR_ENTITYREF_NO_NAME",
    "EntityRef: expecting ';'",
    "PEReference: no name",
    "PEReference: expecting ';'",
    "Undeclared entity error",
    "Undeclared entity warning",
    "Unparsed Entity",
    "XML_ERR_ENTITY_IS_EXTERNAL",
    "XML_ERR_ENTITY_IS_PARAMETER",
    "Unknown encoding",
    "Unsupported encoding",
    "String not started expecting ' or \"",
    "String not closed expecting \" or '",
    "Namespace declaration error",
    "EntityValue: \" or ' expected",
    "EntityValue: \" or ' expected",
    "< in attribute",
    "Attribute not started",
    "Attribute not finished",
    "Attribute without value",
    "Attribute redefined",
    "SystemLiteral \" or ' expected",
    "SystemLiteral \" or ' expected",
    "Comment not finished",
    "Processing Instruction not started",
    "Processing Instruction not finished",
    "NOTATION: Name expected here",
    "'>' required to close NOTATION declaration",
    "'(' required to start ATTLIST enumeration",
    "'(' required to start ATTLIST enumeration",
    "MixedContentDecl : '|' or ')*' expected",
    "XML_ERR_MIXED_NOT_FINISHED",
    "ELEMENT in DTD not started",
    "ELEMENT in DTD not finished",
    "XML declaration not started",
    "XML declaration not finished",
    "XML_ERR_CONDSEC_NOT_STARTED",
    "XML conditional section not closed",
    "Content error in the external subset",
    "DOCTYPE not finished",
    "Sequence ']]>' not allowed in content",
    "CDATA not finished",
    "Reserved XML Name",
    "Space required",
    "XML_ERR_SEPARATOR_REQUIRED",
    "NmToken expected in ATTLIST enumeration",
    "XML_ERR_NAME_REQUIRED",
    "MixedContentDecl : '#PCDATA' expected",
    "SYSTEM or PUBLIC, the URI is missing",
    "PUBLIC, the Public Identifier is missing",
    "< required",
    "> required",
    "</ required",
    "= required",
    "Mismatched tag",
    "Tag not finished",
    "standalone accepts only 'yes' or 'no'",
    "Invalid XML encoding name",
    "Comment must not contain '--' (double-hyphen)",
    "Invalid encoding",
    "external parsed entities cannot be standalone",
    "XML conditional section '[' expected",
    "Entity value required",
    "chunk is not well balanced",
    "extra content at the end of well balanced chunk",
    "XML_ERR_ENTITY_CHAR_ERROR",
    "PEReferences forbidden in internal subset",
    "Detected an entity reference loop",
    "XML_ERR_ENTITY_BOUNDARY",
    "Invalid URI",
    "Fragment not allowed",
    "XML_WAR_CATALOG_PI",
    "XML_ERR_NO_DTD",
    "conditional section INCLUDE or IGNORE keyword expected",
    "Version in XML Declaration missing",
    "XML_WAR_UNKNOWN_VERSION",
    "XML_WAR_LANG_VALUE",
    "XML_WAR_NS_URI",
    "XML_WAR_NS_URI_RELATIVE",
    "Missing encoding in text declaration",
];

fn fn_error_string(_host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let c = conv::to_int(&arg(args, 0));
    Ok(match usize::try_from(c).ok().and_then(|i| ERRORS.get(i)) {
        Some(m) => s(m.as_bytes().to_vec()),
        None => s(b"Unknown".to_vec()),
    })
}

/// UTF-8 → target encoding (ISO-8859-1 / US-ASCII replace with '?').
fn fn_encode(_host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = conv::to_string_bytes(&arg(args, 0));
    let enc = conv::to_string_bytes(&arg(args, 1)).to_ascii_uppercase();
    let limit = match &enc[..] {
        b"ISO-8859-1" => 0xFF,
        b"US-ASCII" => 0x7F,
        _ => return Ok(s(data)),
    };
    let st = String::from_utf8_lossy(&data);
    let out: Vec<u8> = st
        .chars()
        .map(|c| {
            if (c as u32) <= limit {
                c as u32 as u8
            } else {
                b'?'
            }
        })
        .collect();
    Ok(s(out))
}

/// `__xmlp_deprecated($msg, $_, $level = E_DEPRECATED)`: a diagnostic from
/// the calling user line.
fn fn_deprecated(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let msg = conv::to_string_bytes(&arg(args, 0));
    let level = match arg(args, 2) {
        Value::Null => crate::E_DEPRECATED,
        v => conv::to_int(&v),
    };
    host.emit_diagnostic(level, &String::from_utf8_lossy(&msg))?;
    Ok(Value::Null)
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    m.insert(
        "__xmlp_deprecated",
        Builtin {
            name: "__xmlp_deprecated",
            func: fn_deprecated,
            min_args: 1,
            max_args: Some(3),
        },
    );
    m.insert(
        "__xmlp_parse",
        Builtin {
            name: "__xmlp_parse",
            func: fn_parse,
            min_args: 3,
            max_args: Some(3),
        },
    );
    m.insert(
        "__xmlp_encode",
        Builtin {
            name: "__xmlp_encode",
            func: fn_encode,
            min_args: 2,
            max_args: Some(2),
        },
    );
    m.insert(
        "xml_error_string",
        Builtin {
            name: "xml_error_string",
            func: fn_error_string,
            min_args: 1,
            max_args: Some(1),
        },
    );
}

pub fn constants() -> Vec<(&'static str, Value)> {
    let i = Value::Int;
    vec![
        ("XML_ERROR_NONE", i(0)),
        ("XML_ERROR_NO_MEMORY", i(1)),
        ("XML_ERROR_SYNTAX", i(2)),
        ("XML_ERROR_NO_ELEMENTS", i(3)),
        ("XML_ERROR_INVALID_TOKEN", i(4)),
        ("XML_ERROR_UNCLOSED_TOKEN", i(5)),
        ("XML_ERROR_PARTIAL_CHAR", i(6)),
        ("XML_ERROR_TAG_MISMATCH", i(7)),
        ("XML_ERROR_DUPLICATE_ATTRIBUTE", i(8)),
        ("XML_ERROR_JUNK_AFTER_DOC_ELEMENT", i(9)),
        ("XML_ERROR_PARAM_ENTITY_REF", i(10)),
        ("XML_ERROR_UNDEFINED_ENTITY", i(11)),
        ("XML_ERROR_RECURSIVE_ENTITY_REF", i(12)),
        ("XML_ERROR_ASYNC_ENTITY", i(13)),
        ("XML_ERROR_BAD_CHAR_REF", i(14)),
        ("XML_ERROR_BINARY_ENTITY_REF", i(15)),
        ("XML_ERROR_ATTRIBUTE_EXTERNAL_ENTITY_REF", i(16)),
        ("XML_ERROR_MISPLACED_XML_PI", i(17)),
        ("XML_ERROR_UNKNOWN_ENCODING", i(18)),
        ("XML_ERROR_INCORRECT_ENCODING", i(19)),
        ("XML_ERROR_UNCLOSED_CDATA_SECTION", i(20)),
        ("XML_ERROR_EXTERNAL_ENTITY_HANDLING", i(21)),
        ("XML_OPTION_CASE_FOLDING", i(1)),
        ("XML_OPTION_TARGET_ENCODING", i(2)),
        ("XML_OPTION_SKIP_TAGSTART", i(3)),
        ("XML_OPTION_SKIP_WHITE", i(4)),
        ("XML_OPTION_PARSE_HUGE", i(5)),
        ("XML_SAX_IMPL", s(b"libxml".to_vec())),
    ]
}

pub const PRELUDE_PHP: &str = include_str!("prelude.php");
