//! The `openssl` extension — minimal, enough to let Composer build its
//! `HttpDownloader` (which validates the CA bundle with `openssl_x509_parse`).
//!
//! Backed by the pure-Rust `x509-parser` crate. Only certificate *parsing* is
//! implemented so far; the crypto/TLS surface (sign/verify/encrypt, pkey, etc.)
//! is not. `openssl_x509_parse` returns the subset of php's array that real
//! callers in Composer read (name / subject / issuer / validity / serial), plus
//! an `extensions` map carrying `subjectAltName` when present.

use std::collections::HashMap;

use rhp_values::{conv, ArrayKey, PhpArray, Value};
use x509_parser::prelude::*;

use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

/// Map an attribute-type OID to php's short name (e.g. `commonName` -> `CN`).
/// When `short` is false php uses the long name instead.
fn attr_name(oid_short: &str, short: bool) -> String {
    // oid_registry gives us the long name ("commonName"); php's array keys are
    // the SN ("CN") by default, or the LN when $short_names is false.
    let sn = match oid_short {
        "commonName" => "CN",
        "countryName" => "C",
        "stateOrProvinceName" => "ST",
        "localityName" => "L",
        "organizationName" => "O",
        "organizationalUnitName" => "OU",
        "emailAddress" => "emailAddress",
        "serialNumber" => "serialNumber",
        "domainComponent" => "DC",
        other => other,
    };
    if short {
        sn.to_string()
    } else {
        oid_short.to_string()
    }
}

/// Build php's `/C=NL/O=.../CN=...` one-line name (always short names) and the
/// per-attribute assoc array (keys honour `$short_names`) from an X.509 name.
fn name_to_php(name: &X509Name, short: bool) -> (String, Value) {
    let mut oneline = String::new();
    let mut assoc = PhpArray::new();
    for rdn in name.iter() {
        for attr in rdn.iter() {
            let long = oid2sn(attr.attr_type(), oid_registry())
                .unwrap_or("UNKNOWN")
                .to_string();
            let val = attr.as_str().map(|s| s.to_string()).unwrap_or_default();
            // The one-line DN always uses short names in php.
            oneline.push('/');
            oneline.push_str(&attr_name(&long, true));
            oneline.push('=');
            oneline.push_str(&val);
            let key = attr_name(&long, short);
            assoc.insert(
                ArrayKey::from_bytes(key.as_bytes()),
                Value::str(val.into_bytes()),
            );
        }
    }
    (oneline, Value::Array(assoc))
}

fn fn_openssl_x509_parse(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = conv::to_string_bytes(&deref(&args[0]));
    // $short_names defaults to true.
    let short = args
        .get(1)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(true);

    // Accept PEM (possibly a bundle with leading comments / many certs — php
    // parses the first certificate) or raw DER. `iter_from_buffer` scans for the
    // first BEGIN marker, so leading comment lines are skipped.
    let der_owned;
    let der: &[u8] = if data.windows(10).any(|w| w == b"-----BEGIN") {
        match Pem::iter_from_buffer(&data).next() {
            Some(Ok(pem)) => {
                der_owned = pem.contents;
                &der_owned
            }
            _ => return Ok(Value::Bool(false)),
        }
    } else {
        &data
    };

    let cert = match parse_x509_certificate(der) {
        Ok((_, c)) => c,
        Err(_) => return Ok(Value::Bool(false)),
    };

    let mut out = PhpArray::new();
    let (subj_line, subj) = name_to_php(cert.subject(), short);
    let (_issuer_line, issuer) = name_to_php(cert.issuer(), short);
    out.insert(
        ArrayKey::from_bytes(b"name"),
        Value::str(subj_line.into_bytes()),
    );
    out.insert(ArrayKey::from_bytes(b"subject"), subj);
    out.insert(ArrayKey::from_bytes(b"issuer"), issuer);
    out.insert(
        ArrayKey::from_bytes(b"version"),
        Value::Int(cert.version().0 as i64),
    );
    out.insert(
        ArrayKey::from_bytes(b"serialNumber"),
        Value::str(cert.raw_serial_as_string().replace(':', "").into_bytes()),
    );
    out.insert(
        ArrayKey::from_bytes(b"validFrom_time_t"),
        Value::Int(cert.validity().not_before.timestamp()),
    );
    out.insert(
        ArrayKey::from_bytes(b"validTo_time_t"),
        Value::Int(cert.validity().not_after.timestamp()),
    );

    // extensions: expose subjectAltName as php's "DNS:a, DNS:b" string, since
    // TlsHelper reads it.
    let mut exts = PhpArray::new();
    for ext in cert.extensions() {
        if let ParsedExtension::SubjectAlternativeName(san) = ext.parsed_extension() {
            let names: Vec<String> = san
                .general_names
                .iter()
                .filter_map(|gn| match gn {
                    GeneralName::DNSName(n) => Some(format!("DNS:{n}")),
                    GeneralName::IPAddress(ip) => Some(format!(
                        "IP Address:{}",
                        ip.iter()
                            .map(|b| b.to_string())
                            .collect::<Vec<_>>()
                            .join(".")
                    )),
                    _ => None,
                })
                .collect();
            exts.insert(
                ArrayKey::from_bytes(b"subjectAltName"),
                Value::str(names.join(", ").into_bytes()),
            );
        }
    }
    out.insert(ArrayKey::from_bytes(b"extensions"), Value::Array(exts));

    Ok(Value::Array(out))
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    m.insert(
        "openssl_x509_parse",
        Builtin {
            name: "openssl_x509_parse",
            func: fn_openssl_x509_parse,
            min_args: 1,
            max_args: Some(2),
        },
    );
}

/// `OPENSSL_*` constants read by Composer's platform detection.
pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        (
            "OPENSSL_VERSION_TEXT",
            Value::str("OpenSSL 3.6.3 9 Jun 2026"),
        ),
        ("OPENSSL_VERSION_NUMBER", Value::Int(811597872)),
        ("OPENSSL_RAW_DATA", Value::Int(1)),
        ("OPENSSL_ZERO_PADDING", Value::Int(2)),
    ]
}
