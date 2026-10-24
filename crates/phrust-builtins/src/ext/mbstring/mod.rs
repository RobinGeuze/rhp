//! Multibyte string builtins (an `mbstring` subset), focused on **UTF-8**.
//!
//! These functions treat their input bytes according to an *encoding* (default
//! UTF-8) and operate on Unicode **codepoints**. UTF-8 is the dominant encoding
//! and is decoded with php's exact error behaviour (see [`decode_utf8`]); the
//! single-byte encodings `ASCII` and `ISO-8859-1` are also supported (byte ==
//! codepoint). Any other encoding name is reported as invalid, matching php's
//! `ValueError` (phrust does not implement the legacy CJK codecs).
//!
//! Case mapping (`mb_strtoupper` / `mb_strtolower` / `mb_convert_case`) is full
//! Unicode: the per-codepoint mapping tables in [`mb_tables`] were generated from
//! the local php 8.5.8 binary (its exact Unicode version), so casing is
//! byte-identical over the whole range — accented Latin, German ß, the Greek
//! Final_Sigma contextual rule, titlecase digraphs, ligatures, combining marks,
//! CJK and astral emoji. `MB_CASE_*` modes (UPPER/LOWER/TITLE/FOLD and their
//! `_SIMPLE` variants) are all handled.
//!
//! `mb_internal_encoding()` state is a module `thread_local`. Caveat: it lives
//! for the lifetime of the thread, which for the per-run CLI process is exactly
//! one script; it is **not** shared across threads and does not reach the host.
//!
//! Wire into `lib.rs` with `mod mb_fns;` and `mb_fns::register(&mut m);`.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;

use phrust_values::{conv, PhpArray, Value};

use crate::{Builtin, Host, Thrown};

#[path = "mb_tables.rs"]
mod mb_tables;

// ---------------------------------------------------------------------------
// small shared helpers
// ---------------------------------------------------------------------------

/// Deref a (possibly `Ref`) value to an owned value.
fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

/// `(string)` bytes of a value (after deref).
fn bytes_of(v: &Value) -> Vec<u8> {
    conv::to_string_bytes(&deref(v))
}

/// Sentinel codepoint for an illegal input sequence. Encodes to the substitute
/// character `?` (0x3F), matching php's default `mb_substitute_character`.
const ERR: u32 = 0xFFFF_FFFF;

// ---------------------------------------------------------------------------
// encodings
// ---------------------------------------------------------------------------

/// The encodings this subset supports.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Enc {
    Utf8,
    Ascii,
    Latin1,
}

// The `mb_internal_encoding()` state (default UTF-8), per thread.
thread_local! {
    static INTERNAL: RefCell<Enc> = const { RefCell::new(Enc::Utf8) };
}

fn internal_enc() -> Enc {
    INTERNAL.with(|c| *c.borrow())
}

/// Canonical name php reports for an encoding.
fn enc_name(e: Enc) -> &'static str {
    match e {
        Enc::Utf8 => "UTF-8",
        Enc::Ascii => "ASCII",
        Enc::Latin1 => "ISO-8859-1",
    }
}

/// Resolve an encoding name (case-insensitive) to a supported [`Enc`].
fn resolve_enc(name: &[u8]) -> Option<Enc> {
    let lower = name.to_ascii_lowercase();
    match lower.as_slice() {
        b"utf-8" | b"utf8" => Some(Enc::Utf8),
        b"ascii" | b"us-ascii" => Some(Enc::Ascii),
        b"iso-8859-1" | b"iso8859-1" | b"latin1" | b"8859-1" | b"cp819" | b"l1" => Some(Enc::Latin1),
        _ => None,
    }
}

fn enc_error(fname: &str, argnum: usize, name: &[u8]) -> Thrown {
    Thrown::error(
        "ValueError",
        format!(
            "{}(): Argument #{} ($encoding) must be a valid encoding, \"{}\" given",
            fname,
            argnum,
            String::from_utf8_lossy(name)
        ),
    )
}

/// Resolve the optional `$encoding` argument at `idx`, defaulting to the current
/// internal encoding. A present-but-unknown name raises php's `ValueError`.
fn arg_enc(args: &[Value], idx: usize, fname: &str, argnum: usize) -> Result<Enc, Thrown> {
    match args.get(idx).map(deref) {
        None | Some(Value::Null) => Ok(internal_enc()),
        Some(v) => {
            let b = conv::to_string_bytes(&v);
            resolve_enc(&b).ok_or_else(|| enc_error(fname, argnum, &b))
        }
    }
}

// ---------------------------------------------------------------------------
// UTF-8 (and single-byte) decode / encode
// ---------------------------------------------------------------------------

/// Decode UTF-8 into `(codepoint, byte_start, byte_len)` triples, reproducing
/// php's mbstring error handling: each maximal ill-formed subpart becomes one
/// [`ERR`] sentinel (one substitute on re-encode), consuming the valid prefix of
/// a broken sequence and re-reading the offending byte. Matches php's chunking
/// exactly (e.g. `ED A0 80` is three errors, `E2 82 28` is one error then `(`).
fn decode_utf8_full(bytes: &[u8]) -> Vec<(u32, usize, usize)> {
    let mut out = Vec::with_capacity(bytes.len());
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        let b = bytes[i];
        if b < 0x80 {
            out.push((b as u32, i, 1));
            i += 1;
            continue;
        }
        // (total length, valid range for the 2nd byte); 3rd/4th are always 80..BF.
        let (len, lo2, hi2) = match b {
            0xC2..=0xDF => (2, 0x80, 0xBF),
            0xE0 => (3, 0xA0, 0xBF),
            0xE1..=0xEC => (3, 0x80, 0xBF),
            0xED => (3, 0x80, 0x9F),
            0xEE..=0xEF => (3, 0x80, 0xBF),
            0xF0 => (4, 0x90, 0xBF),
            0xF1..=0xF3 => (4, 0x80, 0xBF),
            0xF4 => (4, 0x80, 0x8F),
            _ => {
                // invalid lead byte (incl. lone continuation / overlong C0,C1 / F5+)
                out.push((ERR, i, 1));
                i += 1;
                continue;
            }
        };
        // second byte
        if i + 1 >= n || !(lo2..=hi2).contains(&bytes[i + 1]) {
            out.push((ERR, i, 1));
            i += 1;
            continue;
        }
        // remaining continuation bytes
        let mut consumed = 2;
        let mut ok = true;
        while consumed < len {
            let j = i + consumed;
            if j >= n || !(0x80..=0xBF).contains(&bytes[j]) {
                ok = false;
                break;
            }
            consumed += 1;
        }
        if !ok {
            out.push((ERR, i, consumed)); // drop the valid prefix of the broken sequence
            i += consumed;
            continue;
        }
        let cp = match len {
            2 => ((b as u32 & 0x1F) << 6) | (bytes[i + 1] as u32 & 0x3F),
            3 => {
                ((b as u32 & 0x0F) << 12)
                    | ((bytes[i + 1] as u32 & 0x3F) << 6)
                    | (bytes[i + 2] as u32 & 0x3F)
            }
            _ => {
                ((b as u32 & 0x07) << 18)
                    | ((bytes[i + 1] as u32 & 0x3F) << 12)
                    | ((bytes[i + 2] as u32 & 0x3F) << 6)
                    | (bytes[i + 3] as u32 & 0x3F)
            }
        };
        out.push((cp, i, len));
        i += len;
    }
    out
}

fn decode_utf8(bytes: &[u8]) -> Vec<u32> {
    decode_utf8_full(bytes).into_iter().map(|(cp, _, _)| cp).collect()
}

/// Decode UTF-8 for the **position** functions (`mb_strpos` family), which use a
/// different illegal-byte rule than `mb_strlen`/`mb_substr`: a lone continuation
/// byte (0x80..=0xBF) is **dropped** (contributes no character position), while a
/// lead/invalid-lead error yields one [`ERR`] character. Verified against php.
fn decode_utf8_pos(bytes: &[u8]) -> Vec<u32> {
    let mut out = Vec::with_capacity(bytes.len());
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        let b = bytes[i];
        if b < 0x80 {
            out.push(b as u32);
            i += 1;
            continue;
        }
        if (0x80..=0xBF).contains(&b) {
            i += 1; // lone continuation: no position emitted
            continue;
        }
        let (len, lo2, hi2) = match b {
            0xC2..=0xDF => (2, 0x80, 0xBF),
            0xE0 => (3, 0xA0, 0xBF),
            0xE1..=0xEC => (3, 0x80, 0xBF),
            0xED => (3, 0x80, 0x9F),
            0xEE..=0xEF => (3, 0x80, 0xBF),
            0xF0 => (4, 0x90, 0xBF),
            0xF1..=0xF3 => (4, 0x80, 0xBF),
            0xF4 => (4, 0x80, 0x8F),
            _ => {
                out.push(ERR); // invalid lead (C0,C1,F5+): one position
                i += 1;
                continue;
            }
        };
        if i + 1 >= n || !(lo2..=hi2).contains(&bytes[i + 1]) {
            out.push(ERR);
            i += 1;
            continue;
        }
        let mut consumed = 2;
        let mut ok = true;
        while consumed < len {
            let j = i + consumed;
            if j >= n || !(0x80..=0xBF).contains(&bytes[j]) {
                ok = false;
                break;
            }
            consumed += 1;
        }
        if !ok {
            out.push(ERR);
            i += consumed;
            continue;
        }
        let cp = match len {
            2 => ((b as u32 & 0x1F) << 6) | (bytes[i + 1] as u32 & 0x3F),
            3 => {
                ((b as u32 & 0x0F) << 12)
                    | ((bytes[i + 1] as u32 & 0x3F) << 6)
                    | (bytes[i + 2] as u32 & 0x3F)
            }
            _ => {
                ((b as u32 & 0x07) << 18)
                    | ((bytes[i + 1] as u32 & 0x3F) << 12)
                    | ((bytes[i + 2] as u32 & 0x3F) << 6)
                    | (bytes[i + 3] as u32 & 0x3F)
            }
        };
        out.push(cp);
        i += len;
    }
    out
}

/// Decode for the position functions under `enc` (see [`decode_utf8_pos`]).
fn decode_pos(enc: Enc, bytes: &[u8]) -> Vec<u32> {
    match enc {
        Enc::Utf8 => decode_utf8_pos(bytes),
        Enc::Ascii | Enc::Latin1 => bytes.iter().map(|&b| b as u32).collect(),
    }
}

/// Append the UTF-8 encoding of one codepoint to `out` ([`ERR`] / out-of-range →
/// substitute `?`).
fn encode_cp(cp: u32, out: &mut Vec<u8>) {
    if cp == ERR || cp > 0x10FFFF {
        out.push(b'?');
    } else if cp < 0x80 {
        out.push(cp as u8);
    } else if cp < 0x800 {
        out.push(0xC0 | (cp >> 6) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    } else if cp < 0x10000 {
        out.push(0xE0 | (cp >> 12) as u8);
        out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    } else {
        out.push(0xF0 | (cp >> 18) as u8);
        out.push(0x80 | ((cp >> 12) & 0x3F) as u8);
        out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    }
}

/// Decode a byte string into codepoints under `enc`.
fn decode(enc: Enc, bytes: &[u8]) -> Vec<u32> {
    match enc {
        Enc::Utf8 => decode_utf8(bytes),
        // single-byte: byte value is the codepoint (ISO-8859-1 is pure Latin-1).
        Enc::Ascii | Enc::Latin1 => bytes.iter().map(|&b| b as u32).collect(),
    }
}

/// Encode codepoints back to bytes under `enc` (unmappable → substitute `?`).
fn encode(enc: Enc, cps: &[u32]) -> Vec<u8> {
    match enc {
        Enc::Utf8 => {
            let mut out = Vec::with_capacity(cps.len());
            for &cp in cps {
                encode_cp(cp, &mut out);
            }
            out
        }
        Enc::Ascii => cps
            .iter()
            .map(|&cp| if cp < 0x80 { cp as u8 } else { b'?' })
            .collect(),
        Enc::Latin1 => cps
            .iter()
            .map(|&cp| if cp < 0x100 { cp as u8 } else { b'?' })
            .collect(),
    }
}

/// Per-character byte spans `(start, end)` under `enc`, for functions that slice
/// the **original** bytes (preserving illegal sequences verbatim, as php does for
/// `mb_substr` / `mb_str_pad`). Single-byte encodings yield one span per byte.
fn char_spans(enc: Enc, bytes: &[u8]) -> Vec<(usize, usize)> {
    match enc {
        Enc::Utf8 => decode_utf8_full(bytes).into_iter().map(|(_, s, l)| (s, s + l)).collect(),
        Enc::Ascii | Enc::Latin1 => (0..bytes.len()).map(|i| (i, i + 1)).collect(),
    }
}

/// Per-character byte spans used by `mb_str_split`, which chunks UTF-8 purely by
/// the lead byte's expected length (ignoring continuation validity) and keeps the
/// original bytes — a php quirk distinct from the strict chunking above.
fn split_spans(enc: Enc, bytes: &[u8]) -> Vec<(usize, usize)> {
    match enc {
        Enc::Utf8 => {
            let n = bytes.len();
            let mut out = Vec::new();
            let mut i = 0;
            while i < n {
                let len = match bytes[i] {
                    0x00..=0x7F => 1,
                    0xC2..=0xDF => 2,
                    0xE0..=0xEF => 3,
                    0xF0..=0xF4 => 4,
                    _ => 1, // lone continuation, C0/C1, F5+
                };
                let end = (i + len).min(n);
                out.push((i, end));
                i = end;
            }
            out
        }
        Enc::Ascii | Enc::Latin1 => (0..bytes.len()).map(|i| (i, i + 1)).collect(),
    }
}

// ---------------------------------------------------------------------------
// case mapping (tables generated from php 8.5.8)
// ---------------------------------------------------------------------------

/// Greek capital sigma; the only codepoint with a default (non-locale)
/// context-sensitive lowercase mapping (Final_Sigma).
const SIGMA: u32 = 0x3A3;
const FINAL_SIGMA: u32 = 0x3C2;

fn lookup_full(table: &[(u32, u32, u32, u32)], cp: u32) -> Option<(u32, u32, u32)> {
    table
        .binary_search_by(|e| e.0.cmp(&cp))
        .ok()
        .map(|i| (table[i].1, table[i].2, table[i].3))
}

fn lookup_simple(table: &[(u32, u32)], cp: u32) -> Option<u32> {
    table.binary_search_by(|e| e.0.cmp(&cp)).ok().map(|i| table[i].1)
}

fn in_ranges(table: &[(u32, u32)], cp: u32) -> bool {
    table
        .binary_search_by(|&(s, e)| {
            if cp < s {
                Ordering::Greater
            } else if cp > e {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        })
        .is_ok()
}

fn is_cased(cp: u32) -> bool {
    in_ranges(mb_tables::CASED, cp)
}
fn is_case_ignorable(cp: u32) -> bool {
    // php's case converter treats an illegal-input marker as transparent to word
    // boundaries (unlike a literal '?'), so a cased letter after an error keeps
    // the current word state.
    cp == ERR || in_ranges(mb_tables::CASE_IGNORABLE, cp)
}

/// Push a full-mapping result (or the codepoint itself when unmapped).
fn push_full(out: &mut Vec<u32>, table: &[(u32, u32, u32, u32)], cp: u32) {
    match lookup_full(table, cp) {
        Some((a, b, c)) => {
            out.push(a);
            if b != 0 {
                out.push(b);
            }
            if c != 0 {
                out.push(c);
            }
        }
        None => out.push(cp),
    }
}

/// Is the Σ at `cps[i]` a *final* sigma? True iff preceded by a cased char
/// (skipping case-ignorable) and **not** followed by one.
fn final_sigma(cps: &[u32], i: usize) -> bool {
    let before = cps[..i]
        .iter()
        .rev()
        .find(|&&c| !is_case_ignorable(c))
        .map(|&c| is_cased(c))
        .unwrap_or(false);
    if !before {
        return false;
    }
    let after = cps[i + 1..]
        .iter()
        .find(|&&c| !is_case_ignorable(c))
        .map(|&c| is_cased(c))
        .unwrap_or(false);
    !after
}

/// Lowercase one codepoint at index `i` (full mapping + Final_Sigma).
fn push_lower_full(out: &mut Vec<u32>, cps: &[u32], i: usize) {
    let cp = cps[i];
    if cp == SIGMA && final_sigma(cps, i) {
        out.push(FINAL_SIGMA);
    } else {
        push_full(out, mb_tables::MAP_LOWER, cp);
    }
}

fn upper_full(cps: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(cps.len());
    for &cp in cps {
        push_full(&mut out, mb_tables::MAP_UPPER, cp);
    }
    out
}

fn lower_full(cps: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(cps.len());
    for i in 0..cps.len() {
        push_lower_full(&mut out, cps, i);
    }
    out
}

fn fold_full(cps: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(cps.len());
    for &cp in cps {
        push_full(&mut out, mb_tables::MAP_FOLD, cp);
    }
    out
}

fn title_full(cps: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(cps.len());
    let mut in_word = false;
    for i in 0..cps.len() {
        let cp = cps[i];
        if is_cased(cp) {
            if in_word {
                push_lower_full(&mut out, cps, i);
            } else {
                push_full(&mut out, mb_tables::MAP_TITLE, cp);
                in_word = true;
            }
        } else if is_case_ignorable(cp) {
            out.push(cp);
        } else {
            out.push(cp);
            in_word = false;
        }
    }
    out
}

fn map_simple(cps: &[u32], table: &[(u32, u32)]) -> Vec<u32> {
    cps.iter().map(|&cp| lookup_simple(table, cp).unwrap_or(cp)).collect()
}

fn title_simple(cps: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(cps.len());
    let mut in_word = false;
    for &cp in cps {
        if is_cased(cp) {
            let t = if in_word {
                lookup_simple(mb_tables::MAP_LOWER_SIMPLE, cp)
            } else {
                in_word = true;
                lookup_simple(mb_tables::MAP_TITLE_SIMPLE, cp)
            };
            out.push(t.unwrap_or(cp));
        } else if is_case_ignorable(cp) {
            out.push(cp);
        } else {
            out.push(cp);
            in_word = false;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// mb_strlen / mb_substr
// ---------------------------------------------------------------------------

fn fn_mb_strlen(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let enc = arg_enc(args, 1, "mb_strlen", 2)?;
    Ok(Value::Int(decode(enc, &bytes_of(&args[0])).len() as i64))
}

/// Clamp a php start/offset against codepoint `len`, returning an index 0..=len.
fn clamp_start(start: i64, len: usize) -> usize {
    let len = len as i64;
    if start < 0 {
        (len + start).max(0) as usize
    } else {
        start.min(len) as usize
    }
}

fn fn_mb_substr(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let enc = arg_enc(args, 3, "mb_substr", 4)?;
    let cps = decode(enc, &bytes_of(&args[0]));
    let len = cps.len();
    let start = clamp_start(conv::to_int(&deref(&args[1])), len);
    let rest = len - start;
    let take = match args.get(2).map(deref) {
        None | Some(Value::Null) => rest,
        Some(v) => {
            let l = conv::to_int(&v);
            if l < 0 {
                rest.saturating_sub((-l) as usize)
            } else {
                (l as usize).min(rest)
            }
        }
    };
    // Re-encoded from codepoints, so illegal sequences become the substitute `?`
    // (php's behaviour for mb_substr; mb_str_pad / mb_str_split keep raw bytes).
    Ok(Value::str(encode(enc, &cps[start..start + take])))
}

// ---------------------------------------------------------------------------
// mb_strtoupper / mb_strtolower / mb_convert_case
// ---------------------------------------------------------------------------

fn fn_mb_strtoupper(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let enc = arg_enc(args, 1, "mb_strtoupper", 2)?;
    let cps = decode(enc, &bytes_of(&args[0]));
    Ok(Value::str(encode(enc, &upper_full(&cps))))
}

fn fn_mb_strtolower(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let enc = arg_enc(args, 1, "mb_strtolower", 2)?;
    let cps = decode(enc, &bytes_of(&args[0]));
    Ok(Value::str(encode(enc, &lower_full(&cps))))
}

fn fn_mb_convert_case(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mode = conv::to_int(&deref(&args[1]));
    let enc = arg_enc(args, 2, "mb_convert_case", 3)?;
    let cps = decode(enc, &bytes_of(&args[0]));
    // MB_CASE_*: UPPER=0 LOWER=1 TITLE=2 FOLD=3, and the _SIMPLE variants 4..7.
    let out = match mode {
        0 => upper_full(&cps),
        1 => lower_full(&cps),
        2 => title_full(&cps),
        3 => fold_full(&cps),
        4 => map_simple(&cps, mb_tables::MAP_UPPER_SIMPLE),
        5 => map_simple(&cps, mb_tables::MAP_LOWER_SIMPLE),
        6 => title_simple(&cps),
        7 => map_simple(&cps, mb_tables::MAP_FOLD_SIMPLE),
        _ => {
            return Err(Thrown::error(
                "ValueError",
                "mb_convert_case(): Argument #2 ($mode) must be one of the MB_CASE_* constants",
            ))
        }
    };
    Ok(Value::str(encode(enc, &out)))
}

// ---------------------------------------------------------------------------
// mb_str_split
// ---------------------------------------------------------------------------

fn fn_mb_str_split(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let chunk = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(1);
    let enc = arg_enc(args, 2, "mb_str_split", 3)?;
    if chunk < 1 {
        return Err(Thrown::error(
            "ValueError",
            "mb_str_split(): Argument #2 ($length) must be greater than 0",
        ));
    }
    let bytes = bytes_of(&args[0]);
    let spans = split_spans(enc, &bytes);
    let chunk = chunk as usize;
    let mut out = PhpArray::new();
    let mut i = 0;
    while i < spans.len() {
        let end = (i + chunk).min(spans.len());
        // one element spans from the first char's start to the last char's end,
        // taken from the original bytes (illegal sequences kept verbatim).
        out.push(Value::str(bytes[spans[i].0..spans[end - 1].1].to_vec()));
        i = end;
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// mb_strpos family
// ---------------------------------------------------------------------------

/// First occurrence of `needle` in `haystack` at or after `start` (codepoints).
fn find_cps(haystack: &[u32], needle: &[u32], start: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(start.min(haystack.len()));
    }
    if needle.len() > haystack.len() {
        return None;
    }
    (start..=haystack.len() - needle.len()).find(|&i| haystack[i..i + needle.len()] == *needle)
}

/// Last occurrence of `needle` with its start in `lo..=hi` (inclusive).
fn rfind_cps(haystack: &[u32], needle: &[u32], lo: usize, hi: usize) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    let max_start = hi.min(haystack.len() - needle.len());
    if max_start < lo {
        return None;
    }
    (lo..=max_start).rev().find(|&i| haystack[i..i + needle.len()] == *needle)
}

/// Shared forward search for `mb_strpos` / `mb_stripos`. The case-insensitive
/// variants fold with the strict decoder (continuations counted); the
/// case-sensitive ones use the position decoder (continuations dropped) — a php
/// quirk reproduced here.
fn strpos_impl(args: &[Value], fname: &str, fold: bool) -> Result<Value, Thrown> {
    let enc = arg_enc(args, 3, fname, 4)?;
    let (mut haystack, mut needle) = if fold {
        (decode(enc, &bytes_of(&args[0])), decode(enc, &bytes_of(&args[1])))
    } else {
        (decode_pos(enc, &bytes_of(&args[0])), decode_pos(enc, &bytes_of(&args[1])))
    };
    if fold {
        haystack = map_simple(&haystack, mb_tables::MAP_FOLD_SIMPLE);
        needle = map_simple(&needle, mb_tables::MAP_FOLD_SIMPLE);
    }
    let len = haystack.len() as i64;
    let off = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if off < -len || off > len {
        return Err(Thrown::error(
            "ValueError",
            format!("{fname}(): Argument #3 ($offset) must be contained in argument #1 ($haystack)"),
        ));
    }
    let start = if off < 0 { (len + off) as usize } else { off as usize };
    match find_cps(&haystack, &needle, start) {
        Some(p) => Ok(Value::Int(p as i64)),
        None => Ok(Value::Bool(false)),
    }
}

/// Shared reverse search for `mb_strrpos` / `mb_strripos` (see [`strpos_impl`]
/// for the case-sensitive vs case-insensitive decoder quirk).
fn strrpos_impl(args: &[Value], fname: &str, fold: bool) -> Result<Value, Thrown> {
    let enc = arg_enc(args, 3, fname, 4)?;
    let (mut haystack, mut needle) = if fold {
        (decode(enc, &bytes_of(&args[0])), decode(enc, &bytes_of(&args[1])))
    } else {
        (decode_pos(enc, &bytes_of(&args[0])), decode_pos(enc, &bytes_of(&args[1])))
    };
    if fold {
        haystack = map_simple(&haystack, mb_tables::MAP_FOLD_SIMPLE);
        needle = map_simple(&needle, mb_tables::MAP_FOLD_SIMPLE);
    }
    let len = haystack.len() as i64;
    let off = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if off < -len || off > len {
        return Err(Thrown::error(
            "ValueError",
            format!("{fname}(): Argument #3 ($offset) must be contained in argument #1 ($haystack)"),
        ));
    }
    // offset >= 0 restricts the match start to [offset, len]; offset < 0 restricts
    // it to [0, len + offset].
    let (lo, hi) = if off < 0 {
        (0usize, (len + off) as usize)
    } else {
        (off as usize, haystack.len())
    };
    match rfind_cps(&haystack, &needle, lo, hi) {
        Some(p) => Ok(Value::Int(p as i64)),
        None => Ok(Value::Bool(false)),
    }
}

fn fn_mb_strpos(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strpos_impl(args, "mb_strpos", false)
}
fn fn_mb_stripos(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strpos_impl(args, "mb_stripos", true)
}
fn fn_mb_strrpos(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strrpos_impl(args, "mb_strrpos", false)
}
fn fn_mb_strripos(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strrpos_impl(args, "mb_strripos", true)
}

// ---------------------------------------------------------------------------
// mb_substr_count
// ---------------------------------------------------------------------------

fn fn_mb_substr_count(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let enc = arg_enc(args, 2, "mb_substr_count", 3)?;
    let haystack = decode(enc, &bytes_of(&args[0]));
    let needle = decode(enc, &bytes_of(&args[1]));
    if needle.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            "mb_substr_count(): Argument #2 ($needle) must not be empty",
        ));
    }
    let mut count = 0i64;
    let mut i = 0;
    while i + needle.len() <= haystack.len() {
        if haystack[i..i + needle.len()] == *needle {
            count += 1;
            i += needle.len(); // non-overlapping, as php counts
        } else {
            i += 1;
        }
    }
    Ok(Value::Int(count))
}

// ---------------------------------------------------------------------------
// mb_strwidth
// ---------------------------------------------------------------------------

fn fn_mb_strwidth(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let enc = arg_enc(args, 1, "mb_strwidth", 2)?;
    let cps = decode(enc, &bytes_of(&args[0]));
    let width: i64 = cps
        .iter()
        .map(|&cp| if in_ranges(mb_tables::WIDTH2, cp) { 2 } else { 1 })
        .sum();
    Ok(Value::Int(width))
}

// ---------------------------------------------------------------------------
// mb_ord / mb_chr
// ---------------------------------------------------------------------------

fn fn_mb_ord(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let enc = arg_enc(args, 1, "mb_ord", 2)?;
    let raw = bytes_of(&args[0]);
    // php 8 throws on a truly empty string (but returns false for a non-empty
    // string that merely begins with an illegal sequence).
    if raw.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            "mb_ord(): Argument #1 ($string) must not be empty",
        ));
    }
    let cps = decode(enc, &raw);
    match cps.first() {
        Some(&cp) if cp != ERR => Ok(Value::Int(cp as i64)),
        _ => Ok(Value::Bool(false)),
    }
}

fn fn_mb_chr(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cp = conv::to_int(&deref(&args[0]));
    let enc = arg_enc(args, 1, "mb_chr", 2)?;
    // out of range, or a UTF-16 surrogate, yields false
    if !(0..=0x10FFFF).contains(&cp) || (0xD800..=0xDFFF).contains(&cp) {
        return Ok(Value::Bool(false));
    }
    Ok(Value::str(encode(enc, &[cp as u32])))
}

// ---------------------------------------------------------------------------
// mb_str_pad
// ---------------------------------------------------------------------------

fn fn_mb_str_pad(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let enc = arg_enc(args, 4, "mb_str_pad", 5)?;
    let input = bytes_of(&args[0]);
    let in_len = char_spans(enc, &input).len();
    let target = conv::to_int(&deref(&args[1]));
    let pad_bytes = match args.get(2).map(deref) {
        None => b" ".to_vec(),
        Some(v) => conv::to_string_bytes(&v),
    };
    let pad_type = args.get(3).map(|v| conv::to_int(&deref(v))).unwrap_or(1); // STR_PAD_RIGHT
    if !(0..=2).contains(&pad_type) {
        return Err(Thrown::error(
            "ValueError",
            "mb_str_pad(): Argument #4 ($pad_type) must be STR_PAD_LEFT, STR_PAD_RIGHT, or STR_PAD_BOTH",
        ));
    }
    let need = (target.max(0) as usize).saturating_sub(in_len);
    if need == 0 {
        return Ok(Value::str(input));
    }
    let pad_spans = char_spans(enc, &pad_bytes);
    if pad_spans.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            "mb_str_pad(): Argument #3 ($pad_string) must not be empty",
        ));
    }
    // Build `count` pad characters by cycling the pad string's characters, each
    // emitted as its original bytes.
    let make = |count: usize| -> Vec<u8> {
        let mut v = Vec::new();
        for k in 0..count {
            let (s, e) = pad_spans[k % pad_spans.len()];
            v.extend_from_slice(&pad_bytes[s..e]);
        }
        v
    };
    let out = match pad_type {
        0 => {
            let mut v = make(need);
            v.extend_from_slice(&input);
            v
        }
        2 => {
            let left = need / 2; // STR_PAD_BOTH: left gets the smaller half
            let mut v = make(left);
            v.extend_from_slice(&input);
            v.extend(make(need - left));
            v
        }
        _ => {
            let mut v = input.clone();
            v.extend(make(need));
            v
        }
    };
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// mb_internal_encoding / mb_check_encoding / mb_convert_encoding
// ---------------------------------------------------------------------------

fn fn_mb_internal_encoding(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match args.first().map(deref) {
        None | Some(Value::Null) => Ok(Value::str(enc_name(internal_enc()))),
        Some(v) => {
            let b = conv::to_string_bytes(&v);
            match resolve_enc(&b) {
                Some(e) => {
                    INTERNAL.with(|c| *c.borrow_mut() = e);
                    Ok(Value::Bool(true))
                }
                None => Err(enc_error("mb_internal_encoding", 1, &b)),
            }
        }
    }
}

/// Is `bytes` valid in `enc`?
fn is_valid(enc: Enc, bytes: &[u8]) -> bool {
    match enc {
        Enc::Utf8 => std::str::from_utf8(bytes).is_ok(),
        Enc::Ascii => bytes.iter().all(|&b| b < 0x80),
        Enc::Latin1 => true, // every byte sequence is valid Latin-1
    }
}

fn fn_mb_check_encoding(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // No value given: php returns true.
    let value = match args.first().map(deref) {
        None | Some(Value::Null) => return Ok(Value::Bool(true)),
        Some(v) => conv::to_string_bytes(&v),
    };
    let enc = arg_enc(args, 1, "mb_check_encoding", 2)?;
    Ok(Value::Bool(is_valid(enc, &value)))
}

fn fn_mb_convert_encoding(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let to_name = conv::to_string_bytes(&deref(&args[1]));
    let to = resolve_enc(&to_name).ok_or_else(|| enc_error("mb_convert_encoding", 2, &to_name))?;
    let from = arg_enc(args, 2, "mb_convert_encoding", 3)?;
    let cps = decode(from, &bytes_of(&args[0]));
    Ok(Value::str(encode(to, &cps)))
}

// ---------------------------------------------------------------------------
// mb_detect_encoding
// ---------------------------------------------------------------------------

/// `mb_detect_encoding($string, $encodings = null, $strict = false)`.
///
/// Pragmatic detector covering the common case: returns `"UTF-8"` for valid
/// (non-ASCII-containing) UTF-8, `"ASCII"` for a 7-bit string, and `false` for
/// bytes that are neither. The full php candidate-list machinery is not modelled.
fn fn_mb_detect_encoding(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    if s.iter().all(|&b| b < 0x80) {
        return Ok(Value::str("ASCII"));
    }
    if std::str::from_utf8(&s).is_ok() {
        Ok(Value::str("UTF-8"))
    } else {
        Ok(Value::Bool(false))
    }
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// Register all multibyte builtins into the shared table.
pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(name, Builtin { name, func, min_args: min, max_args: max });
    }
    put(m, "mb_strlen", fn_mb_strlen, 1, Some(2));
    put(m, "mb_substr", fn_mb_substr, 2, Some(4));
    put(m, "mb_strtoupper", fn_mb_strtoupper, 1, Some(2));
    put(m, "mb_strtolower", fn_mb_strtolower, 1, Some(2));
    put(m, "mb_convert_case", fn_mb_convert_case, 2, Some(3));
    put(m, "mb_str_split", fn_mb_str_split, 1, Some(3));
    put(m, "mb_strpos", fn_mb_strpos, 2, Some(4));
    put(m, "mb_stripos", fn_mb_stripos, 2, Some(4));
    put(m, "mb_strrpos", fn_mb_strrpos, 2, Some(4));
    put(m, "mb_strripos", fn_mb_strripos, 2, Some(4));
    put(m, "mb_substr_count", fn_mb_substr_count, 2, Some(3));
    put(m, "mb_strwidth", fn_mb_strwidth, 1, Some(2));
    put(m, "mb_ord", fn_mb_ord, 1, Some(2));
    put(m, "mb_chr", fn_mb_chr, 1, Some(2));
    put(m, "mb_str_pad", fn_mb_str_pad, 2, Some(5));
    put(m, "mb_internal_encoding", fn_mb_internal_encoding, 0, Some(1));
    put(m, "mb_check_encoding", fn_mb_check_encoding, 0, Some(2));
    put(m, "mb_convert_encoding", fn_mb_convert_encoding, 2, Some(3));
    put(m, "mb_detect_encoding", fn_mb_detect_encoding, 1, Some(3));
}

// ---------------------------------------------------------------------------
// tests — expected values captured from php 8.5.8
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct NullHost;
    impl Host for NullHost {
        fn write_out(&mut self, _: &[u8]) {}
    }

    fn s(b: &str) -> Value {
        Value::str(b)
    }
    fn call(f: crate::NativeFn, args: &[Value]) -> Value {
        f(&mut NullHost, args).expect("no throw")
    }
    fn calls(f: crate::NativeFn, args: &[Value]) -> String {
        String::from_utf8(bytes_of(&call(f, args))).unwrap()
    }
    fn call_hex(f: crate::NativeFn, args: &[Value]) -> String {
        bytes_of(&call(f, args)).iter().map(|b| format!("{b:02x}")).collect()
    }
    fn call_int(f: crate::NativeFn, args: &[Value]) -> i64 {
        match call(f, args) {
            Value::Int(n) => n,
            other => panic!("expected int, got {}", other.type_name()),
        }
    }
    fn call_pos(f: crate::NativeFn, args: &[Value]) -> Option<i64> {
        match call(f, args) {
            Value::Int(n) => Some(n),
            Value::Bool(false) => None,
            other => panic!("expected int|false, got {}", other.type_name()),
        }
    }
    fn is_false(f: crate::NativeFn, args: &[Value]) -> bool {
        matches!(call(f, args), Value::Bool(false))
    }
    fn split_vec(f: crate::NativeFn, args: &[Value]) -> Vec<String> {
        match call(f, args) {
            Value::Array(a) => {
                a.iter().map(|(_, v)| String::from_utf8(bytes_of(v)).unwrap()).collect()
            }
            other => panic!("expected array, got {}", other.type_name()),
        }
    }

    #[test]
    fn strlen_cases() {
        assert_eq!(call_int(fn_mb_strlen, &[s("héllo")]), 5);
        assert_eq!(call_int(fn_mb_strlen, &[s("")]), 0);
        assert_eq!(call_int(fn_mb_strlen, &[s("€uro")]), 4);
        assert_eq!(call_int(fn_mb_strlen, &[s("日本語")]), 3);
        // invalid UTF-8 chunking (ED A0 80 → 3 errors)
        assert_eq!(call_int(fn_mb_strlen, &[Value::str(vec![0xED, 0xA0, 0x80])]), 3);
        assert_eq!(call_int(fn_mb_strlen, &[Value::str(vec![0xC3, 0x28])]), 2);
    }

    #[test]
    fn substr_cases() {
        assert_eq!(calls(fn_mb_substr, &[s("héllo"), Value::Int(1), Value::Int(3)]), "éll");
        assert_eq!(calls(fn_mb_substr, &[s("héllo"), Value::Int(-2)]), "lo");
        assert_eq!(calls(fn_mb_substr, &[s("héllo"), Value::Int(1), Value::Int(-1)]), "éll");
        assert_eq!(calls(fn_mb_substr, &[s("héllo"), Value::Int(-10)]), "héllo");
        // invalid lead byte → substitute
        assert_eq!(call_hex(fn_mb_substr, &[Value::str(vec![0xC3, 0x28, 0x61, 0x62]), Value::Int(0), Value::Int(2)]), "3f28");
    }

    #[test]
    fn case_cases() {
        assert_eq!(calls(fn_mb_strtoupper, &[s("héllo wörld")]), "HÉLLO WÖRLD");
        assert_eq!(calls(fn_mb_strtolower, &[s("HÉLLO WÖRLD")]), "héllo wörld");
        assert_eq!(calls(fn_mb_strtoupper, &[s("straße")]), "STRASSE");
        assert_eq!(calls(fn_mb_strtoupper, &[s("µ")]), "Μ");
        assert_eq!(calls(fn_mb_strtoupper, &[s("×÷")]), "×÷"); // math signs unchanged
    }

    #[test]
    fn final_sigma_cases() {
        // "ΟΣΟΣ" → "οσος" with the LAST sigma as final sigma ς (cf82)
        assert_eq!(call_hex(fn_mb_strtolower, &[s("ΟΣΟΣ")]), "cebfcf83cebfcf82");
        // isolated Σ (no preceding cased) → non-final σ
        assert_eq!(call_hex(fn_mb_strtolower, &[s("Σ")]), "cf83");
    }

    #[test]
    fn convert_case_cases() {
        assert_eq!(calls(fn_mb_convert_case, &[s("héllo wörld"), Value::Int(2)]), "Héllo Wörld");
        assert_eq!(calls(fn_mb_convert_case, &[s("it's a test"), Value::Int(2)]), "It's A Test");
        assert_eq!(calls(fn_mb_convert_case, &[s("a.b a,b"), Value::Int(2)]), "A.b A,B");
        assert_eq!(calls(fn_mb_convert_case, &[s("abc123def"), Value::Int(2)]), "Abc123Def");
        assert_eq!(calls(fn_mb_convert_case, &[s("ßtra"), Value::Int(2)]), "Sstra");
        // FOLD: ß → ss, µ → μ
        assert_eq!(call_hex(fn_mb_convert_case, &[s("ß"), Value::Int(3)]), "7373");
        assert_eq!(call_hex(fn_mb_convert_case, &[s("µ"), Value::Int(3)]), "cebc");
        // SIMPLE upper keeps ß
        assert_eq!(call_hex(fn_mb_convert_case, &[s("ß"), Value::Int(4)]), "c39f");
    }

    #[test]
    fn str_split_cases() {
        assert_eq!(split_vec(fn_mb_str_split, &[s("héllo")]), vec!["h", "é", "l", "l", "o"]);
        assert_eq!(split_vec(fn_mb_str_split, &[s("héllo"), Value::Int(2)]), vec!["hé", "ll", "o"]);
        assert!(fn_mb_str_split(&mut NullHost, &[s("abc"), Value::Int(0)]).is_err());
    }

    #[test]
    fn pos_cases() {
        assert_eq!(call_pos(fn_mb_strpos, &[s("héllo"), s("llo")]), Some(2));
        assert_eq!(call_pos(fn_mb_strpos, &[s("aéaéa"), s("a"), Value::Int(-1)]), Some(4));
        assert_eq!(call_pos(fn_mb_stripos, &[s("Héllo WÖRLD"), s("wörld")]), Some(6));
        assert!(is_false(fn_mb_stripos, &[s("maße"), s("MASSE")])); // simple fold: ß ≠ ss
        assert_eq!(call_pos(fn_mb_strrpos, &[s("aéaéa"), s("a")]), Some(4));
        assert_eq!(call_pos(fn_mb_strrpos, &[s("aéaéa"), s("a"), Value::Int(-2)]), Some(2));
        assert_eq!(call_pos(fn_mb_strrpos, &[s("abcabc"), s("bc"), Value::Int(-3)]), Some(1));
        assert_eq!(call_pos(fn_mb_strripos, &[s("AéAéA"), s("a")]), Some(4));
    }

    #[test]
    fn count_width_cases() {
        assert_eq!(call_int(fn_mb_substr_count, &[s("ababab"), s("ab")]), 3);
        assert_eq!(call_int(fn_mb_substr_count, &[s("aaa"), s("aa")]), 1); // non-overlapping
        assert!(fn_mb_substr_count(&mut NullHost, &[s("abc"), s("")]).is_err());
        assert_eq!(call_int(fn_mb_strwidth, &[s("日本語abc")]), 9);
        assert_eq!(call_int(fn_mb_strwidth, &[s("😀")]), 2);
    }

    #[test]
    fn ord_chr_cases() {
        assert_eq!(call_int(fn_mb_ord, &[s("é")]), 233);
        assert_eq!(call_int(fn_mb_ord, &[s("€")]), 8364);
        assert!(fn_mb_ord(&mut NullHost, &[s("")]).is_err()); // php throws on empty
        assert_eq!(calls(fn_mb_chr, &[Value::Int(233)]), "é");
        assert!(is_false(fn_mb_chr, &[Value::Int(0x110000)]));
    }

    #[test]
    fn str_pad_cases() {
        assert_eq!(calls(fn_mb_str_pad, &[s("héllo"), Value::Int(10), s("-*"), Value::Int(1)]), "héllo-*-*-");
        assert_eq!(calls(fn_mb_str_pad, &[s("héllo"), Value::Int(10), s("xy"), Value::Int(2)]), "xyhélloxyx");
        assert_eq!(calls(fn_mb_str_pad, &[s("héllo"), Value::Int(3)]), "héllo");
    }

    #[test]
    fn encoding_cases() {
        // mb_convert_encoding($str, $to, $from): café UTF-8 → ISO-8859-1 (é → 0xe9)
        assert_eq!(
            call_hex(fn_mb_convert_encoding, &[s("café"), s("ISO-8859-1"), s("UTF-8")]),
            "636166e9"
        );
        // reinterpreting those UTF-8 bytes as Latin-1 → UTF-8 (Ã,©)
        assert_eq!(
            call_hex(fn_mb_convert_encoding, &[s("café"), s("UTF-8"), s("ISO-8859-1")]),
            "636166c383c2a9"
        );
        // "latin1" is an accepted alias for "ISO-8859-1"
        assert_eq!(
            call_hex(fn_mb_convert_encoding, &[s("café"), s("UTF-8"), s("latin1")]),
            "636166c383c2a9"
        );
        // unmappable → '?'
        assert_eq!(call_hex(fn_mb_convert_encoding, &[s("café"), s("ASCII"), s("UTF-8")]), "6361663f");
        assert!(matches!(call(fn_mb_check_encoding, &[s("héllo"), s("UTF-8")]), Value::Bool(true)));
        assert!(is_false(fn_mb_check_encoding, &[Value::str(vec![0xC3, 0x28]), s("UTF-8")]));
        assert!(matches!(call(fn_mb_check_encoding, &[]), Value::Bool(true)));
        // invalid encoding name → ValueError
        assert!(fn_mb_strlen(&mut NullHost, &[s("x"), s("bogus")]).is_err());
    }

    #[test]
    fn internal_encoding_roundtrip() {
        assert_eq!(calls(fn_mb_internal_encoding, &[]), "UTF-8");
        assert!(matches!(call(fn_mb_internal_encoding, &[s("ISO-8859-1")]), Value::Bool(true)));
        assert_eq!(calls(fn_mb_internal_encoding, &[]), "ISO-8859-1");
        // restore so other tests on this thread see the default
        let _ = call(fn_mb_internal_encoding, &[s("UTF-8")]);
        assert!(fn_mb_internal_encoding(&mut NullHost, &[s("bogus")]).is_err());
    }
}
