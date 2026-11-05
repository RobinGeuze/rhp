//! What php's own argument parsing does with the parameters a stub cannot
//! describe, read from each function's C body: its one
//! `ZEND_PARSE_PARAMETERS_START … END` block or `zend_parse_parameters`
//! format string. Only three parses matter to the VM's zpp pass:
//! `Z_PARAM_ZVAL` / `z` (anything goes), `Z_PARAM_RESOURCE` / `r` (a
//! resource) and `PHP_Z_PARAM_STREAM` (an open stream). A function whose
//! parse is in a helper, split over several blocks, not one entry per
//! stub parameter, or after code that may raise or return (assert's
//! early return, stream_context_set_option's deprecation) gets nothing,
//! and the VM leaves its checks to the body.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One parameter's parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Parse {
    /// `Z_PARAM_ZVAL*`, `z`: any value.
    Zval,
    /// `Z_PARAM_RESOURCE`, `r`; `true`: or null.
    Resource(bool),
    /// `PHP_Z_PARAM_STREAM`; `true`: or null.
    Stream(bool),
    /// Anything else.
    Other,
}

/// The parse of each parameter, by C function name.
pub(super) type Parses = BTreeMap<String, Vec<Parse>>;

/// Scan the `.c` files below `dirs` (relative to `php_src`; a file entry
/// means the `.c` files next to it).
pub(super) fn scan(php_src: &Path, dirs: &[&str]) -> Result<Parses, String> {
    let mut files = Vec::new();
    for rel in dirs {
        let p = php_src.join(rel);
        if p.is_dir() {
            walk(&p, true, &mut files);
        } else if let Some(dir) = p.parent() {
            walk(dir, false, &mut files);
        }
    }
    files.sort();
    files.dedup();
    let mut seen: BTreeMap<String, Option<Vec<Parse>>> = BTreeMap::new();
    for path in files {
        let src = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let src = strip_comments(&src);
        for (name, body) in functions(&src) {
            let parse = parses(body);
            // Defined twice (#if branches): keep it only when both agree.
            seen.entry(name)
                .and_modify(|old| {
                    if *old != parse {
                        *old = None;
                    }
                })
                .or_insert(parse);
        }
    }
    let out: Parses = seen
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
        .collect();
    // A stub-only checkout has no C sources: say so instead of generating
    // a table without them.
    if !out.contains_key("fwrite") {
        return Err(format!(
            "no PHP_FUNCTION(fwrite) under {}: the php-src checkout needs its .c files",
            php_src.display()
        ));
    }
    Ok(out)
}

fn walk(dir: &Path, deep: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if deep && p.file_name().is_some_and(|n| n != "tests") {
                walk(&p, deep, out);
            }
        } else if p.extension().is_some_and(|x| x == "c") {
            out.push(p);
        }
    }
}

/// The source with comments replaced by a space; string and character
/// literals are kept.
fn strip_comments(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'/' if src.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < src.len() && !(src[i] == b'*' && src.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i += 2;
                out.push(b' ');
            }
            b'/' if src.get(i + 1) == Some(&b'/') => {
                while i < src.len() && src[i] != b'\n' {
                    i += 1;
                }
            }
            q @ (b'"' | b'\'') => {
                out.push(q);
                i += 1;
                while i < src.len() && src[i] != q && src[i] != b'\n' {
                    if src[i] == b'\\' && i + 1 < src.len() {
                        out.push(src[i]);
                        i += 1;
                    }
                    out.push(src[i]);
                    i += 1;
                }
                if i < src.len() {
                    out.push(src[i]);
                    i += 1;
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Each `PHP_FUNCTION(name) {…}` / `ZEND_FUNCTION(name) {…}` at the start
/// of a line (optionally `PHPAPI` or `static`), with its body.
fn functions(src: &[u8]) -> Vec<(String, &[u8])> {
    let mut out = Vec::new();
    let mut line = 0;
    while line < src.len() {
        let end = src[line..]
            .iter()
            .position(|&c| c == b'\n')
            .map_or(src.len(), |n| line + n);
        let mut l = &src[line..end];
        for pre in [&b"PHPAPI "[..], b"static "] {
            if let Some(rest) = l.strip_prefix(pre) {
                l = rest;
            }
        }
        let name = [&b"PHP_FUNCTION("[..], b"ZEND_FUNCTION("]
            .iter()
            .find_map(|m| l.strip_prefix(*m))
            .and_then(|rest| {
                let close = rest.iter().position(|&c| c == b')')?;
                let n = &rest[..close];
                n.iter()
                    .all(|c| c.is_ascii_alphanumeric() || *c == b'_')
                    .then(|| String::from_utf8_lossy(n).into_owned())
            });
        if let Some(name) = name {
            if let Some(body) = body_after(src, end) {
                out.push((name, body));
            }
        }
        line = end + 1;
    }
    out
}

/// The `{…}` that starts after `at` (only whitespace before it).
fn body_after(src: &[u8], at: usize) -> Option<&[u8]> {
    let open = at
        + src
            .get(at..)?
            .iter()
            .position(|c| !c.is_ascii_whitespace())?;
    if src[open] != b'{' {
        return None;
    }
    let close = matching(src, open, b'{', b'}')?;
    Some(&src[open + 1..close])
}

/// The index of the bracket closing the one at `open`, skipping literals.
fn matching(src: &[u8], open: usize, l: u8, r: u8) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < src.len() {
        match src[i] {
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < src.len() && src[i] != q {
                    if src[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            c if c == l => depth += 1,
            c if c == r => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|n| from + n)
}

/// Every start of `needle` in `hay` as a whole identifier.
fn idents(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = find(hay, needle, from) {
        let before = at.checked_sub(1).map(|i| hay[i]);
        let after = hay.get(at + needle.len()).copied();
        if !before.is_some_and(ident) && !after.is_some_and(ident) {
            out.push(at);
        }
        from = at + 1;
    }
    out
}

/// The parse of each parameter of one body, when it has exactly one.
fn parses(body: &[u8]) -> Option<Vec<Parse>> {
    let starts: Vec<usize> = [
        &b"ZEND_PARSE_PARAMETERS_START"[..],
        b"ZEND_PARSE_PARAMETERS_START_EX",
    ]
    .iter()
    .flat_map(|m| idents(body, m))
    .collect();
    let none = [
        &b"ZEND_PARSE_PARAMETERS_NONE"[..],
        b"zend_parse_parameters_none",
    ]
    .iter()
    .map(|m| idents(body, m).len())
    .sum::<usize>();
    let calls: Vec<usize> = [
        &b"zend_parse_parameters"[..],
        b"zend_parse_parameters_ex",
        b"zend_parse_parameters_throw",
        b"zend_parse_method_parameters",
    ]
    .iter()
    .flat_map(|m| idents(body, m))
    .collect();
    if starts.len() + none + calls.len() != 1 {
        return None;
    }
    if none == 1 {
        return Some(Vec::new());
    }
    let first = starts.iter().chain(&calls).copied().min()?;
    let raises = [
        &b"zend_error"[..],
        b"php_error_docref",
        b"zend_throw",
        b"zend_argument_",
        b"RETURN_",
    ];
    if raises.iter().any(|m| find(&body[..first], m, 0).is_some()) {
        return None;
    }
    if let Some(&at) = starts.first() {
        let end = find(body, b"ZEND_PARSE_PARAMETERS_END", at)?;
        return Some(block(&body[at..end]));
    }
    let at = calls[0];
    if body[at..].starts_with(b"zend_parse_method_parameters") {
        return None;
    }
    let open = at + body[at..].iter().position(|&c| c == b'(')?;
    let close = matching(body, open, b'(', b')')?;
    let args = &body[open + 1..close];
    let q = args.iter().position(|&c| c == b'"')?;
    let len = args[q + 1..].iter().position(|&c| c == b'"')?;
    Some(format(&args[q + 1..q + 1 + len]))
}

/// The `Z_PARAM_*` entries of a fast-ZPP block, one per parameter.
fn block(b: &[u8]) -> Vec<Parse> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(at) = find(b, b"Z_PARAM_", i) {
        let start = if b[..at].ends_with(b"PHP_") {
            at - 4
        } else {
            at
        };
        let len = b[start..]
            .iter()
            .position(|c| !(c.is_ascii_alphanumeric() || *c == b'_'))
            .unwrap_or(b.len() - start);
        let name = &b[start..start + len];
        i = start + len;
        let ident_before =
            start > 0 && (b[start - 1].is_ascii_alphanumeric() || b[start - 1] == b'_');
        if ident_before || name == b"Z_PARAM_OPTIONAL" {
            continue;
        }
        out.push(match name {
            b"Z_PARAM_ZVAL"
            | b"Z_PARAM_ZVAL_OR_NULL"
            | b"Z_PARAM_ZVAL_EX"
            | b"Z_PARAM_ZVAL_EX2" => Parse::Zval,
            b"Z_PARAM_RESOURCE" => Parse::Resource(false),
            b"Z_PARAM_RESOURCE_OR_NULL" => Parse::Resource(true),
            b"PHP_Z_PARAM_STREAM" => Parse::Stream(false),
            b"PHP_Z_PARAM_STREAM_OR_NULL" => Parse::Stream(true),
            _ => Parse::Other,
        });
    }
    out
}

/// The parameters of a `zend_parse_parameters` format: one letter each,
/// `!` (or null) and `/` (separate) modifying the one before.
fn format(f: &[u8]) -> Vec<Parse> {
    let mut out: Vec<Parse> = Vec::new();
    for &c in f {
        match c {
            b'|' | b'/' => {}
            b'!' => {
                if let Some(Parse::Resource(n)) = out.last_mut() {
                    *n = true;
                }
            }
            b'z' => out.push(Parse::Zval),
            b'r' => out.push(Parse::Resource(false)),
            _ => out.push(Parse::Other),
        }
    }
    out
}

#[cfg(test)]
mod tests;
