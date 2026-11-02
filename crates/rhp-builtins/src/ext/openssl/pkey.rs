//! Public keys: `openssl_pkey_get_public` / `openssl_get_publickey`,
//! `openssl_pkey_get_details`, `openssl_pkey_free` / `openssl_free_key`,
//! `openssl_verify` and `openssl_get_md_methods` (php-src
//! `ext/openssl/openssl.c`, OpenSSL 3 semantics).
//!
//! Keys are held as their SubjectPublicKeyInfo DER, parsed with a tiny DER
//! reader. Verification is implemented directly over `num-bigint`: RSA
//! PKCS#1 v1.5 (DigestInfo comparison) and PSS (MGF1, salt length recovered
//! from the signature, as OpenSSL's verify-side "auto"), and ECDSA over the
//! NIST P-256/P-384/P-521 and secp256k1 curves. RSA and EC keys are
//! supported; other key types are rejected as unreadable.
//!
//! An `OpenSSLAsymmetricKey` is a prelude-defined final class; the key
//! material lives in a per-thread side table keyed by the object (weakly, so
//! a freed object's slot is dropped).

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use num_bigint::BigUint;
use num_traits::{One, Zero};
use rhp_values::{ArrayKey, ObjectData, PhpArray, Value};

use crate::ext::zpp::{deref, given_type, str_arg};
use crate::{deprecated_fn, warn, Host, Thrown};

// ---------------------------------------------------------------------------
// DER
// ---------------------------------------------------------------------------

/// Read one TLV at the start of `b`: `(tag, value, rest)`.
fn tlv(b: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    if b.len() < 2 {
        return None;
    }
    let tag = b[0];
    let (len, hdr) = if b[1] < 0x80 {
        (b[1] as usize, 2)
    } else {
        let n = (b[1] & 0x7f) as usize;
        if n == 0 || n > 4 || b.len() < 2 + n {
            return None;
        }
        let mut l = 0usize;
        for &x in &b[2..2 + n] {
            l = (l << 8) | x as usize;
        }
        (l, 2 + n)
    };
    if b.len() < hdr + len {
        return None;
    }
    Some((tag, &b[hdr..hdr + len], &b[hdr + len..]))
}

fn der_len(n: usize) -> Vec<u8> {
    if n < 0x80 {
        vec![n as u8]
    } else {
        let bytes: Vec<u8> = n
            .to_be_bytes()
            .iter()
            .copied()
            .skip_while(|&x| x == 0)
            .collect();
        let mut v = vec![0x80 | bytes.len() as u8];
        v.extend(bytes);
        v
    }
}

fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut v = vec![tag];
    v.extend(der_len(content.len()));
    v.extend_from_slice(content);
    v
}

/// Strip leading zero bytes of an unsigned big-endian integer.
fn strip0(b: &[u8]) -> &[u8] {
    let i = b.iter().position(|&x| x != 0).unwrap_or(b.len());
    &b[i..]
}

const OID_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
const OID_EC: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];

// ---------------------------------------------------------------------------
// keys
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum CurveId {
    P256,
    P384,
    P521,
    K256,
}

struct Curve {
    name: &'static str,
    oid_text: &'static str,
    oid: &'static [u8],
    bits: u64,
    p: &'static str,
    a: &'static str,
    b: &'static str,
    gx: &'static str,
    gy: &'static str,
    n: &'static str,
}

fn curve(id: CurveId) -> Curve {
    match id {
        CurveId::P256 => Curve {
            name: "prime256v1",
            oid_text: "1.2.840.10045.3.1.7",
            oid: &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07],
            bits: 256,
            p: "ffffffff00000001000000000000000000000000ffffffffffffffffffffffff",
            a: "ffffffff00000001000000000000000000000000fffffffffffffffffffffffc",
            b: "5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b",
            gx: "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296",
            gy: "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5",
            n: "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551",
        },
        CurveId::P384 => Curve {
            name: "secp384r1",
            oid_text: "1.3.132.0.34",
            oid: &[0x2b, 0x81, 0x04, 0x00, 0x22],
            bits: 384,
            p: "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000ffffffff",
            a: "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000fffffffc",
            b: "b3312fa7e23ee7e4988e056be3f82d19181d9c6efe8141120314088f5013875ac656398d8a2ed19d2a85c8edd3ec2aef",
            gx: "aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab7",
            gy: "3617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c00a60b1ce1d7e819d7a431d7c90ea0e5f",
            n: "ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973",
        },
        CurveId::P521 => Curve {
            name: "secp521r1",
            oid_text: "1.3.132.0.35",
            oid: &[0x2b, 0x81, 0x04, 0x00, 0x23],
            bits: 521,
            p: "01ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            a: "01fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc",
            b: "0051953eb9618e1c9a1f929a21a0b68540eea2da725b99b315f3b8b489918ef109e156193951ec7e937b1652c0bd3bb1bf073573df883d2c34f1ef451fd46b503f00",
            gx: "00c6858e06b70404e9cd9e3ecb662395b4429c648139053fb521f828af606b4d3dbaa14b5e77efe75928fe1dc127a2ffa8de3348b3c1856a429bf97e7e31c2e5bd66",
            gy: "011839296a789a3bc0045c8a5fb42c7d1bd998f54449579b446817afbd17273e662c97ee72995ef42640c550b9013fad0761353c7086a272c24088be94769fd16650",
            n: "01fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffa51868783bf2f966b7fcc0148f709a5d03bb5c9b8899c47aebb6fb71e91386409",
        },
        CurveId::K256 => Curve {
            name: "secp256k1",
            oid_text: "1.3.132.0.10",
            oid: &[0x2b, 0x81, 0x04, 0x00, 0x0a],
            bits: 256,
            p: "fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f",
            a: "0000000000000000000000000000000000000000000000000000000000000000",
            b: "0000000000000000000000000000000000000000000000000000000000000007",
            gx: "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            gy: "483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8",
            n: "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141",
        },
    }
}

fn hexn(s: &str) -> BigUint {
    BigUint::parse_bytes(s.as_bytes(), 16).unwrap_or_default()
}

enum KeyKind {
    Rsa {
        n: Vec<u8>,
        e: Vec<u8>,
    },
    Ec {
        curve: CurveId,
        x: Vec<u8>,
        y: Vec<u8>,
    },
}

pub(crate) struct PubKey {
    spki: Vec<u8>,
    kind: KeyKind,
}

/// Parse a SubjectPublicKeyInfo DER into a supported key.
fn parse_spki(spki: &[u8]) -> Option<PubKey> {
    let (t, body, _) = tlv(spki)?;
    if t != 0x30 {
        return None;
    }
    let (t, alg, rest) = tlv(body)?;
    if t != 0x30 {
        return None;
    }
    let (t, bits, _) = tlv(rest)?;
    if t != 0x03 || bits.is_empty() || bits[0] != 0 {
        return None;
    }
    let key = &bits[1..];
    let (t, oid, params) = tlv(alg)?;
    if t != 0x06 {
        return None;
    }
    let kind = if oid == OID_RSA {
        let (t, seq, _) = tlv(key)?;
        if t != 0x30 {
            return None;
        }
        let (t, n, rest) = tlv(seq)?;
        if t != 0x02 {
            return None;
        }
        let (t, e, _) = tlv(rest)?;
        if t != 0x02 {
            return None;
        }
        KeyKind::Rsa {
            n: strip0(n).to_vec(),
            e: strip0(e).to_vec(),
        }
    } else if oid == OID_EC {
        let (t, coid, _) = tlv(params)?;
        if t != 0x06 {
            return None;
        }
        let id = [CurveId::P256, CurveId::P384, CurveId::P521, CurveId::K256]
            .into_iter()
            .find(|&c| curve(c).oid == coid)?;
        let c = curve(id);
        let flen = c.bits.div_ceil(8) as usize;
        let (x, y) = match key.first()? {
            4 if key.len() == 1 + 2 * flen => (key[1..1 + flen].to_vec(), key[1 + flen..].to_vec()),
            2 | 3 if key.len() == 1 + flen => {
                // decompress: y = sqrt(x^3 + ax + b), p ≡ 3 (mod 4) for all four curves
                let p = hexn(c.p);
                let x = BigUint::from_bytes_be(&key[1..]);
                let rhs = (x.modpow(&BigUint::from(3u8), &p) + hexn(c.a) * &x + hexn(c.b)) % &p;
                let mut y = rhs.modpow(&((&p + BigUint::one()) >> 2), &p);
                if (&y * &y) % &p != rhs {
                    return None;
                }
                let odd = y.bit(0);
                if odd != (key[0] == 3) {
                    y = &p - y;
                }
                (pad(&x.to_bytes_be(), flen), pad(&y.to_bytes_be(), flen))
            }
            _ => return None,
        };
        KeyKind::Ec { curve: id, x, y }
    } else {
        return None;
    };
    Some(PubKey {
        spki: spki.to_vec(),
        kind,
    })
}

fn pad(b: &[u8], len: usize) -> Vec<u8> {
    if b.len() >= len {
        return b.to_vec();
    }
    let mut v = vec![0u8; len - b.len()];
    v.extend_from_slice(b);
    v
}

/// All PEM blocks `(label, der)` in `data`.
fn pem_blocks(data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let text = String::from_utf8_lossy(data);
    let mut out = Vec::new();
    let mut rest: &str = &text;
    while let Some(i) = rest.find("-----BEGIN ") {
        let after = &rest[i + 11..];
        let Some(j) = after.find("-----") else { break };
        let label = after[..j].to_string();
        let body_start = &after[j + 5..];
        let end_marker = format!("-----END {label}-----");
        let Some(k) = body_start.find(&end_marker) else {
            break;
        };
        let body = &body_start[..k];
        // skip RFC 1421 headers (lines with ':') before the base64 payload
        let b64: String = body
            .lines()
            .filter(|l| !l.contains(':'))
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if let Some(d) = b64_decode(&b64) {
            out.push((label, d));
        }
        rest = &body_start[k + end_marker.len()..];
    }
    out
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut nbits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
            acc &= (1 << nbits) - 1;
        }
    }
    Some(out)
}

fn b64_encode(d: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for ch in d.chunks(3) {
        let b = [ch[0], *ch.get(1).unwrap_or(&0), *ch.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if ch.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if ch.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

/// `PEM_write_bio_PUBKEY`.
fn pubkey_pem(spki: &[u8]) -> String {
    let b = b64_encode(spki);
    let mut s = String::from("-----BEGIN PUBLIC KEY-----\n");
    for chunk in b.as_bytes().chunks(64) {
        s.push_str(std::str::from_utf8(chunk).unwrap_or(""));
        s.push('\n');
    }
    s.push_str("-----END PUBLIC KEY-----\n");
    s
}

/// Read a public key from PEM text: a certificate's key, a `PUBLIC KEY`
/// (SPKI) block, or a PKCS#1 `RSA PUBLIC KEY` block.
fn key_from_pem(data: &[u8]) -> Option<PubKey> {
    let blocks = pem_blocks(data);
    // php_openssl_x509_from_str first: the first certificate wins.
    if let Some((_, d)) = blocks
        .iter()
        .find(|(l, _)| l == "CERTIFICATE" || l == "X509 CERTIFICATE" || l == "TRUSTED CERTIFICATE")
    {
        if let Ok((_, cert)) = x509_parser::parse_x509_certificate(d) {
            return parse_spki(cert.tbs_certificate.subject_pki.raw);
        }
        return None;
    }
    for (label, d) in &blocks {
        match label.as_str() {
            "PUBLIC KEY" => return parse_spki(d),
            "RSA PUBLIC KEY" => {
                let mut alg = der(0x06, OID_RSA);
                alg.extend(der(0x05, &[]));
                let mut bits = vec![0u8];
                bits.extend_from_slice(d);
                let mut body = der(0x30, &alg);
                body.extend(der(0x03, &bits));
                return parse_spki(&der(0x30, &body));
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// OpenSSLAsymmetricKey side table
// ---------------------------------------------------------------------------

type Slot = (Weak<RefCell<ObjectData>>, Rc<PubKey>);

thread_local! {
    static KEYS: RefCell<Vec<Slot>> = const { RefCell::new(Vec::new()) };
}

/// Forget every key (a new request).
pub(crate) fn reset_state() {
    KEYS.with(|k| k.borrow_mut().clear());
}

fn key_of(v: &Value) -> Option<Rc<PubKey>> {
    let Value::Object(h) = deref(v) else {
        return None;
    };
    KEYS.with(|k| {
        k.borrow()
            .iter()
            .find(|(w, _)| w.upgrade().is_some_and(|o| Rc::ptr_eq(&o, &h)))
            .map(|(_, key)| key.clone())
    })
}

fn is_key_object(host: &dyn Host, v: &Value) -> bool {
    matches!(deref(v), Value::Object(_))
        && host.object_class_name(&deref(v)).as_deref() == Some(b"OpenSSLAsymmetricKey".as_ref())
}

fn wrap_key(host: &mut dyn Host, key: Rc<PubKey>) -> Result<Value, Thrown> {
    let Some(obj) = host.instantiate(b"OpenSSLAsymmetricKey", Vec::new()) else {
        // a host without native object construction (the oracle)
        return Err(Thrown::unsupported(
            "OpenSSLAsymmetricKey",
            "host cannot instantiate internal classes",
        ));
    };
    if let Value::Object(h) = &obj {
        KEYS.with(|k| {
            let mut k = k.borrow_mut();
            k.retain(|(w, _)| w.strong_count() > 0);
            k.push((Rc::downgrade(h), key));
        });
    }
    Ok(obj)
}

/// `php_openssl_pkey_from_zval(val, public_key = 1, ...)`.
fn pkey_from_zval(
    host: &mut dyn Host,
    v: &Value,
    fname: &str,
) -> Result<Option<Rc<PubKey>>, Thrown> {
    let v = deref(v);
    if let Value::Array(a) = &v {
        let k0 = a.get(&ArrayKey::Int(0)).cloned();
        let has1 = a.get(&ArrayKey::Int(1)).is_some();
        let (Some(k0), true) = (k0, has1) else {
            return Err(Thrown::error(
                "ValueError",
                "Key array must be of the form array(0 => key, 1 => phrase)",
            ));
        };
        return pkey_from_zval(host, &k0, fname);
    }
    if is_key_object(host, &v) {
        return Ok(key_of(&v));
    }
    let data = match &v {
        Value::Str(s) => s.as_bytes().to_vec(),
        Value::Object(_) => {
            // zval_try_get_string: __toString, or the engine's conversion Error
            let cls = host.object_class_name(&v);
            let stringable = cls
                .as_deref()
                .and_then(|c| host.resolve_class(c))
                .is_some_and(|id| host.class_has_method(id, b"__tostring"));
            if !stringable {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Object of class {} could not be converted to string",
                        String::from_utf8_lossy(&cls.unwrap_or_default())
                    ),
                ));
            }
            str_arg(host, std::slice::from_ref(&v), 0, fname, "key")?
        }
        _ => return Ok(None),
    };
    let data = if data.len() > 7 && data.starts_with(b"file://") {
        let path = String::from_utf8_lossy(&data[7..]).into_owned();
        match std::fs::read(&path) {
            Ok(d) => d,
            Err(_) => return Ok(None),
        }
    } else {
        data
    };
    Ok(key_from_pem(&data).map(Rc::new))
}

pub(super) fn fn_pkey_get_public(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = args.first().cloned().unwrap_or(Value::Null);
    if is_key_object(host, &v) {
        // a fresh object sharing the key, like php's EVP_PKEY_up_ref
        return match key_of(&v) {
            Some(k) => wrap_key(host, k),
            None => Ok(Value::Bool(false)),
        };
    }
    match pkey_from_zval(host, &v, "openssl_pkey_get_public")? {
        Some(k) => wrap_key(host, k),
        None => Ok(Value::Bool(false)),
    }
}

fn want_key_object(host: &dyn Host, args: &[Value], fname: &str) -> Result<Rc<PubKey>, Thrown> {
    let v = args.first().map(deref).unwrap_or(Value::Null);
    if is_key_object(host, &v) {
        if let Some(k) = key_of(&v) {
            return Ok(k);
        }
    }
    Err(Thrown::type_error(format!(
        "{fname}(): Argument #1 ($key) must be of type OpenSSLAsymmetricKey, {} given",
        given_type(host, &v)
    )))
}

/// The `#[\Deprecated(since: '8.0', message: ...)]` both free functions carry;
/// php raises it before parsing the arguments.
const FREE_DEPRECATION: &str = "as OpenSSLAsymmetricKey objects are freed automatically";

pub(super) fn fn_pkey_free(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    deprecated_fn(host, "openssl_pkey_free", "8.0", FREE_DEPRECATION)?;
    want_key_object(host, args, "openssl_pkey_free")?;
    Ok(Value::Null)
}

pub(super) fn fn_free_key(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    deprecated_fn(host, "openssl_free_key", "8.0", FREE_DEPRECATION)?;
    want_key_object(host, args, "openssl_free_key")?;
    Ok(Value::Null)
}

pub(super) fn fn_pkey_get_details(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let key = want_key_object(host, args, "openssl_pkey_get_details")?;
    let mut out = PhpArray::new();
    let (bits, ktype, sub_name, sub) = match &key.kind {
        KeyKind::Rsa { n, e } => {
            let bits = BigUint::from_bytes_be(n).bits() as i64;
            let mut a = PhpArray::new();
            a.insert(ArrayKey::from_bytes(b"n"), Value::str(n.clone()));
            a.insert(ArrayKey::from_bytes(b"e"), Value::str(e.clone()));
            (bits, 0, "rsa", a)
        }
        KeyKind::Ec { curve: id, x, y } => {
            let c = curve(*id);
            let mut a = PhpArray::new();
            a.insert(ArrayKey::from_bytes(b"curve_name"), Value::str(c.name));
            a.insert(ArrayKey::from_bytes(b"curve_oid"), Value::str(c.oid_text));
            a.insert(ArrayKey::from_bytes(b"x"), Value::str(x.clone()));
            a.insert(ArrayKey::from_bytes(b"y"), Value::str(y.clone()));
            (c.bits as i64, 3, "ec", a)
        }
    };
    out.insert(ArrayKey::from_bytes(b"bits"), Value::Int(bits));
    out.insert(
        ArrayKey::from_bytes(b"key"),
        Value::str(pubkey_pem(&key.spki).into_bytes()),
    );
    out.insert(ArrayKey::from_bytes(sub_name.as_bytes()), Value::Array(sub));
    out.insert(ArrayKey::from_bytes(b"type"), Value::Int(ktype));
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// digests
// ---------------------------------------------------------------------------

/// A message digest OpenSSL can use for signatures: our hash name plus the
/// DER OID used in an RSA DigestInfo (`None` = no RSA encoding).
struct Md {
    algo: &'static [u8],
    oid: Option<&'static [u8]>,
}

const fn md(algo: &'static [u8], oid: Option<&'static [u8]>) -> Md {
    Md { algo, oid }
}

const NIST: [u8; 8] = [0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02];

fn md_by_name(name: &str) -> Option<Md> {
    const MD5: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x02, 0x05];
    const MD4: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x02, 0x04];
    const SHA1: &[u8] = &[0x2b, 0x0e, 0x03, 0x02, 0x1a];
    const RMD160: &[u8] = &[0x2b, 0x24, 0x03, 0x02, 0x01];
    const fn nist(n: u8) -> [u8; 9] {
        [
            NIST[0], NIST[1], NIST[2], NIST[3], NIST[4], NIST[5], NIST[6], NIST[7], n,
        ]
    }
    static SHA224: [u8; 9] = nist(4);
    static SHA256: [u8; 9] = nist(1);
    static SHA384: [u8; 9] = nist(2);
    static SHA512: [u8; 9] = nist(3);
    static SHA512_224: [u8; 9] = nist(5);
    static SHA512_256: [u8; 9] = nist(6);
    static SHA3_224: [u8; 9] = nist(7);
    static SHA3_256: [u8; 9] = nist(8);
    static SHA3_384: [u8; 9] = nist(9);
    static SHA3_512: [u8; 9] = nist(10);
    let n = name.to_ascii_lowercase();
    Some(match n.as_str() {
        "md4" | "rsa-md4" | "md4withrsaencryption" => md(b"md4", Some(MD4)),
        "md5" | "rsa-md5" | "md5withrsaencryption" | "ssl3-md5" => md(b"md5", Some(MD5)),
        "sha1" | "rsa-sha1" | "sha1withrsaencryption" | "ssl3-sha1" | "rsa-sha1-2" | "sha-1" => {
            md(b"sha1", Some(SHA1))
        }
        "sha224" | "rsa-sha224" | "sha224withrsaencryption" | "sha2-224" => {
            md(b"sha224", Some(&SHA224))
        }
        "sha256" | "rsa-sha256" | "sha256withrsaencryption" | "sha2-256" => {
            md(b"sha256", Some(&SHA256))
        }
        "sha384" | "rsa-sha384" | "sha384withrsaencryption" | "sha2-384" => {
            md(b"sha384", Some(&SHA384))
        }
        "sha512" | "rsa-sha512" | "sha512withrsaencryption" | "sha2-512" => {
            md(b"sha512", Some(&SHA512))
        }
        "sha512-224" | "rsa-sha512/224" | "sha512-224withrsaencryption" | "sha2-512/224" => {
            md(b"sha512/224", Some(&SHA512_224))
        }
        "sha512-256" | "rsa-sha512/256" | "sha512-256withrsaencryption" | "sha2-512/256" => {
            md(b"sha512/256", Some(&SHA512_256))
        }
        "sha3-224" | "rsa-sha3-224" | "id-rsassa-pkcs1-v1_5-with-sha3-224" => {
            md(b"sha3-224", Some(&SHA3_224))
        }
        "sha3-256" | "rsa-sha3-256" | "id-rsassa-pkcs1-v1_5-with-sha3-256" => {
            md(b"sha3-256", Some(&SHA3_256))
        }
        "sha3-384" | "rsa-sha3-384" | "id-rsassa-pkcs1-v1_5-with-sha3-384" => {
            md(b"sha3-384", Some(&SHA3_384))
        }
        "sha3-512" | "rsa-sha3-512" | "id-rsassa-pkcs1-v1_5-with-sha3-512" => {
            md(b"sha3-512", Some(&SHA3_512))
        }
        "ripemd160" | "ripemd" | "rmd160" | "rsa-ripemd160" | "ripemd160withrsa" => {
            md(b"ripemd160", Some(RMD160))
        }
        "whirlpool" => md(b"whirlpool", None),
        _ => return None,
    })
}

fn md_by_algo(a: i64) -> Option<Md> {
    md_by_name(match a {
        1 => "sha1",
        2 => "md5",
        3 => "md4",
        6 => "sha224",
        7 => "sha256",
        8 => "sha384",
        9 => "sha512",
        10 => "ripemd160",
        _ => return None,
    })
}

fn hash(m: &Md, data: &[u8]) -> Vec<u8> {
    crate::ext::hash::digest(m.algo, data).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// verification
// ---------------------------------------------------------------------------

/// `RSASSA-PKCS1-v1_5` / `RSASSA-PSS` verify. `Some(true/false)`, or `None`
/// for an internal error (OpenSSL's -1).
fn rsa_verify(n: &[u8], e: &[u8], m: &Md, data: &[u8], sig: &[u8], pss: bool) -> Option<bool> {
    let n_int = BigUint::from_bytes_be(n);
    let k = n.len();
    if sig.len() != k {
        return Some(false);
    }
    let s = BigUint::from_bytes_be(sig);
    if s >= n_int {
        return Some(false);
    }
    let em = pad(
        &s.modpow(&BigUint::from_bytes_be(e), &n_int).to_bytes_be(),
        k,
    );
    let h = hash(m, data);
    if pss {
        let mod_bits = n_int.bits() as usize;
        return Some(pss_verify(&h, &em, mod_bits - 1, m));
    }
    let oid = m.oid?;
    let mut alg = der(0x06, oid);
    alg.extend(der(0x05, &[]));
    let mut di = der(0x30, &alg);
    di.extend(der(0x04, &h));
    let t = der(0x30, &di);
    if k < t.len() + 11 {
        return Some(false);
    }
    let mut want = vec![0x00, 0x01];
    want.extend(std::iter::repeat_n(0xff, k - t.len() - 3));
    want.push(0x00);
    want.extend(t);
    Some(em == want)
}

fn mgf1(m: &Md, seed: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut counter = 0u32;
    while out.len() < len {
        let mut d = seed.to_vec();
        d.extend_from_slice(&counter.to_be_bytes());
        out.extend(hash(m, &d));
        counter += 1;
    }
    out.truncate(len);
    out
}

/// EMSA-PSS-VERIFY (RFC 8017 §9.1.2) with the salt length recovered.
fn pss_verify(mhash: &[u8], em_full: &[u8], em_bits: usize, m: &Md) -> bool {
    let em_len = em_bits.div_ceil(8);
    // when modBits-1 is a multiple of 8 the encoded message is one byte shorter
    let em = if em_full.len() > em_len {
        if em_full[0] != 0 {
            return false;
        }
        &em_full[1..]
    } else {
        em_full
    };
    let hlen = mhash.len();
    if em_len < hlen + 2 || em[em_len - 1] != 0xbc {
        return false;
    }
    let (masked_db, rest) = em.split_at(em_len - hlen - 1);
    let hh = &rest[..hlen];
    let zero_bits = 8 * em_len - em_bits;
    if zero_bits > 0 && masked_db[0] >> (8 - zero_bits) != 0 {
        return false;
    }
    let mask = mgf1(m, hh, masked_db.len());
    let mut db: Vec<u8> = masked_db.iter().zip(&mask).map(|(a, b)| a ^ b).collect();
    if zero_bits > 0 {
        db[0] &= 0xff >> zero_bits;
    }
    let Some(one) = db.iter().position(|&x| x != 0) else {
        return false;
    };
    if db[one] != 1 {
        return false;
    }
    let salt = &db[one + 1..];
    let mut mp = vec![0u8; 8];
    mp.extend_from_slice(mhash);
    mp.extend_from_slice(salt);
    hash(m, &mp) == hh
}

/// Affine point on a short-Weierstrass curve (None = infinity).
type Pt = Option<(BigUint, BigUint)>;

fn modinv(a: &BigUint, p: &BigUint) -> BigUint {
    a.modpow(&(p - BigUint::from(2u8)), p)
}

fn sub_mod(a: &BigUint, b: &BigUint, p: &BigUint) -> BigUint {
    ((a + p) - (b % p)) % p
}

fn pt_add(p1: &Pt, p2: &Pt, a: &BigUint, p: &BigUint) -> Pt {
    let (Some((x1, y1)), Some((x2, y2))) = (p1, p2) else {
        return if p1.is_none() { p2.clone() } else { p1.clone() };
    };
    let l = if x1 == x2 {
        if ((y1 + y2) % p).is_zero() {
            return None;
        }
        let num = (BigUint::from(3u8) * x1 * x1 + a) % p;
        (num * modinv(&((BigUint::from(2u8) * y1) % p), p)) % p
    } else {
        (sub_mod(y2, y1, p) * modinv(&sub_mod(x2, x1, p), p)) % p
    };
    let x3 = sub_mod(&sub_mod(&((&l * &l) % p), x1, p), x2, p);
    let y3 = sub_mod(&((&l * sub_mod(x1, &x3, p)) % p), y1, p);
    Some((x3, y3))
}

fn pt_mul(k: &BigUint, pt: &Pt, a: &BigUint, p: &BigUint) -> Pt {
    let mut r: Pt = None;
    for i in (0..k.bits()).rev() {
        r = pt_add(&r, &r, a, p);
        if k.bit(i) {
            r = pt_add(&r, pt, a, p);
        }
    }
    r
}

/// ECDSA verify; `None` (OpenSSL's -1) for a signature that is not
/// canonical DER `SEQUENCE { INTEGER r, INTEGER s }`.
fn ecdsa_verify(id: CurveId, x: &[u8], y: &[u8], m: &Md, data: &[u8], sig: &[u8]) -> Option<bool> {
    let c = curve(id);
    let Some((0x30, body, [])) = tlv(sig) else {
        return None;
    };
    let Some((0x02, rb, rest)) = tlv(body) else {
        return None;
    };
    let Some((0x02, sb, [])) = tlv(rest) else {
        return None;
    };
    let mut re = der(0x02, rb);
    re.extend(der(0x02, sb));
    if der(0x30, &re) != sig {
        return None;
    }
    let (r, s) = (BigUint::from_bytes_be(rb), BigUint::from_bytes_be(sb));
    let n = hexn(c.n);
    if r.is_zero() || s.is_zero() || r >= n || s >= n {
        return Some(false);
    }
    let h = hash(m, data);
    let nbits = n.bits() as usize;
    let mut e = BigUint::from_bytes_be(&h);
    if h.len() * 8 > nbits {
        e >>= h.len() * 8 - nbits;
    }
    let p = hexn(c.p);
    let a = hexn(c.a);
    let w = s.modpow(&(&n - BigUint::from(2u8)), &n);
    let u1 = (&e * &w) % &n;
    let u2 = (&r * &w) % &n;
    let g: Pt = Some((hexn(c.gx), hexn(c.gy)));
    let q: Pt = Some((BigUint::from_bytes_be(x), BigUint::from_bytes_be(y)));
    let pt = pt_add(&pt_mul(&u1, &g, &a, &p), &pt_mul(&u2, &q, &a, &p), &a, &p);
    Some(match pt {
        Some((x1, _)) => x1 % &n == r,
        None => false,
    })
}

pub(super) fn fn_verify(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_verify";
    let data = str_arg(host, args, 0, f, "data")?;
    let sig = str_arg(host, args, 1, f, "signature")?;
    let method = args.get(3).map(deref);
    let padding = crate::ext::zpp::int_arg_or(host, args, 4, f, "padding", 0)?;
    let md = match &method {
        None => md_by_algo(1),
        Some(Value::Str(s)) => md_by_name(&String::from_utf8_lossy(s.as_bytes())),
        Some(Value::Int(0)) => md_by_name("sha256"),
        Some(Value::Array(_)) | Some(Value::Object(_)) | Some(Value::Null) => {
            let v = method.clone().unwrap_or(Value::Null);
            return Err(Thrown::type_error(format!(
                "{f}(): Argument #4 ($algorithm) must be of type string|int, {} given",
                given_type(host, &v)
            )));
        }
        Some(v) => md_by_algo(crate::ext::zpp::int_arg(
            host,
            std::slice::from_ref(v),
            0,
            f,
            "algorithm",
        )?),
    };
    let Some(md) = md else {
        warn(host, f, "Unknown digest algorithm")?;
        return Ok(Value::Bool(false));
    };
    let key_arg = args.get(2).cloned().unwrap_or(Value::Null);
    let Some(key) = pkey_from_zval(host, &key_arg, f)? else {
        warn(
            host,
            f,
            "Supplied key param cannot be coerced into a public key",
        )?;
        return Ok(Value::Bool(false));
    };
    // php_openssl_setup_rsa_padding
    let res = match &key.kind {
        KeyKind::Rsa { n, e } => {
            if padding != 0 && padding != 1 && padding != 6 {
                warn(host, f, "Unknown padding type")?;
                None
            } else {
                rsa_verify(n, e, &md, &data, &sig, padding == 6)
            }
        }
        KeyKind::Ec { curve, x, y } => {
            if padding != 0 {
                warn(host, f, "Padding parameter is only supported for RSA keys")?;
                None
            } else {
                ecdsa_verify(*curve, x, y, &md, &data, &sig)
            }
        }
    };
    Ok(Value::Int(match res {
        Some(true) => 1,
        Some(false) => 0,
        None => -1,
    }))
}

/// Verify `sig` over `data` with the `PUBLIC KEY` PEM in `pem` (PKCS#1 v1.5
/// for RSA, ECDSA for EC) under the named digest, for phar's OpenSSL
/// signatures (`PEM_read_bio_PUBKEY` + `EVP_VerifyFinal`). `None` when the key
/// cannot be read; `Some(verified)` otherwise.
pub(crate) fn verify_with_pem(pem: &[u8], digest: &str, data: &[u8], sig: &[u8]) -> Option<bool> {
    let der = pem_blocks(pem)
        .into_iter()
        .find(|(l, _)| l == "PUBLIC KEY")
        .map(|(_, d)| d)?;
    let key = parse_spki(&der)?;
    let Some(md) = md_by_name(digest) else {
        return Some(false);
    };
    let ok = match &key.kind {
        KeyKind::Rsa { n, e } => rsa_verify(n, e, &md, data, sig, false),
        KeyKind::Ec { curve, x, y } => ecdsa_verify(*curve, x, y, &md, data, sig),
    };
    Some(ok == Some(true))
}

const MD_METHODS: &[&str] = &[
    "blake2b512",
    "blake2s256",
    "md4",
    "md5",
    "md5-sha1",
    "mdc2",
    "ripemd160",
    "sha1",
    "sha224",
    "sha256",
    "sha3-224",
    "sha3-256",
    "sha3-384",
    "sha3-512",
    "sha384",
    "sha512",
    "sha512-224",
    "sha512-256",
    "shake128",
    "shake256",
    "sm3",
    "whirlpool",
];

const MD_METHODS_ALIASES: &[&str] = &[
    "RSA-MD4",
    "RSA-MD5",
    "RSA-MDC2",
    "RSA-RIPEMD160",
    "RSA-SHA1",
    "RSA-SHA1-2",
    "RSA-SHA224",
    "RSA-SHA256",
    "RSA-SHA3-224",
    "RSA-SHA3-256",
    "RSA-SHA3-384",
    "RSA-SHA3-512",
    "RSA-SHA384",
    "RSA-SHA512",
    "RSA-SHA512/224",
    "RSA-SHA512/256",
    "RSA-SM3",
    "blake2b512",
    "blake2s256",
    "id-rsassa-pkcs1-v1_5-with-sha3-224",
    "id-rsassa-pkcs1-v1_5-with-sha3-256",
    "id-rsassa-pkcs1-v1_5-with-sha3-384",
    "id-rsassa-pkcs1-v1_5-with-sha3-512",
    "md4",
    "md4WithRSAEncryption",
    "md5",
    "md5-sha1",
    "md5WithRSAEncryption",
    "mdc2",
    "mdc2WithRSA",
    "ripemd",
    "ripemd160",
    "ripemd160WithRSA",
    "rmd160",
    "sha1",
    "sha1WithRSAEncryption",
    "sha224",
    "sha224WithRSAEncryption",
    "sha256",
    "sha256WithRSAEncryption",
    "sha3-224",
    "sha3-256",
    "sha3-384",
    "sha3-512",
    "sha384",
    "sha384WithRSAEncryption",
    "sha512",
    "sha512-224",
    "sha512-224WithRSAEncryption",
    "sha512-256",
    "sha512-256WithRSAEncryption",
    "sha512WithRSAEncryption",
    "shake128",
    "shake256",
    "sm3",
    "sm3WithRSAEncryption",
    "ssl3-md5",
    "ssl3-sha1",
    "whirlpool",
];

pub(super) fn fn_get_md_methods(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let aliases =
        crate::ext::zpp::bool_arg_or(host, args, 0, "openssl_get_md_methods", "aliases", false)?;
    let mut a = PhpArray::new();
    for m in if aliases {
        MD_METHODS_ALIASES
    } else {
        MD_METHODS
    } {
        a.push(Value::str(*m));
    }
    Ok(Value::Array(a))
}

pub const PRELUDE_PHP: &str = r#"
final class OpenSSLAsymmetricKey {
    public function __construct() { throw new Error("Cannot directly construct OpenSSLAsymmetricKey, use openssl_pkey_new() instead"); }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag_test::run;

    const RSA: &str = "-----BEGIN PUBLIC KEY-----
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQC8dJ1adN1iO6N5NkABfiX4aUQe
/4Fwl8ieHHju1OZzTCXeyahCs52FOvFTYOQEl4RbA2iVQx78nXt7iTk+mgZ1+tpr
xBx1QV0LDEcLLd2OaWmhFLAV5dlqnv0sbWyQX6hO4Q+be15nf/bpAo4f2Zuc8x9i
BRhJ8KfkQ/zJ0fiwVQIDAQAB
-----END PUBLIC KEY-----
";
    const EC: &str = "-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEb0juCnk6R2lIWkq0bSWzPBsoImrB
O4bihc90FOdq4LXCZ3weD9YelPP4XpA8W+gswhpe+pBkgCqdPqTf8vN72g==
-----END PUBLIC KEY-----
";
    const SIG: &str = "T4nNTV/m3gKkoAB6TqCQ1G8lI+uDQU/9rIz8nKm26mRirvksHpJ5V4eF0Qt90+vgQRSP0jI52vGQBFAzS4hMZS1EVYIz7+WRV/tHhI8ckGp9Wz/26HgbFMs4em6zGwmway7xSouxOVhVCaa2VTK2E29AUeRk+VVG6ImIfqMKZ7o=";
    const ECSIG: &str = "MEYCIQCeDBjGv00skRrACs9fccJ5XkXhh/dGyigCWCO7BzUEaQIhALlW5o/+01Bp2yU3Msp7fOtrrash+K3/Vk9nqIMKZkww";

    fn b64(s: &str) -> Value {
        Value::str(b64_decode(s).unwrap())
    }

    fn verify(args: Vec<Value>) -> (String, Vec<String>) {
        let (r, d) = run(fn_verify, &args);
        let r = match r {
            Ok(Value::Int(i)) => format!("int({i})"),
            Ok(Value::Bool(b)) => format!("bool({b})"),
            Ok(_) => "other".into(),
            Err(Thrown::Error { class, message }) => format!("{class}: {message}"),
            Err(_) => "thrown".into(),
        };
        (r, d)
    }

    #[test]
    fn verify_warnings() {
        let m = || Value::str("hello world");
        let w = |s: &str| vec![format!("Warning: openssl_verify(): {s}")];
        assert_eq!(
            verify(vec![m(), b64(SIG), Value::str(RSA), Value::str("sha256")]),
            ("int(1)".into(), vec![])
        );
        assert_eq!(
            verify(vec![m(), b64(SIG), Value::str(RSA), Value::str("bogus")]),
            ("bool(false)".into(), w("Unknown digest algorithm"))
        );
        assert_eq!(
            verify(vec![m(), b64(SIG), Value::str(RSA), Value::Int(999)]),
            ("bool(false)".into(), w("Unknown digest algorithm"))
        );
        assert_eq!(
            verify(vec![
                m(),
                b64(SIG),
                Value::str(RSA),
                Value::str("sha256"),
                Value::Int(999)
            ]),
            ("int(-1)".into(), w("Unknown padding type"))
        );
        assert_eq!(
            verify(vec![m(), b64(SIG), Value::str("bogus key")]),
            (
                "bool(false)".into(),
                w("Supplied key param cannot be coerced into a public key")
            )
        );
        assert_eq!(
            verify(vec![m(), b64(SIG), Value::Null]),
            (
                "bool(false)".into(),
                w("Supplied key param cannot be coerced into a public key")
            )
        );
        assert_eq!(
            verify(vec![
                m(),
                b64(ECSIG),
                Value::str(EC),
                Value::str("sha256"),
                Value::Int(6)
            ]),
            (
                "int(-1)".into(),
                w("Padding parameter is only supported for RSA keys")
            )
        );
        assert_eq!(
            verify(vec![m(), b64(ECSIG), Value::str(EC), Value::str("sha256")]),
            ("int(1)".into(), vec![])
        );
    }

    #[test]
    fn free_is_deprecated() {
        for (f, name) in [
            (fn_pkey_free as crate::NativeFn, "openssl_pkey_free"),
            (fn_free_key, "openssl_free_key"),
        ] {
            let (r, d) = run(f, &[Value::Int(1)]);
            assert!(r.is_err());
            assert_eq!(
                d,
                vec![format!(
                    "Deprecated: Function {name}() is deprecated since 8.0, as OpenSSLAsymmetricKey objects are freed automatically"
                )]
            );
        }
    }
}
