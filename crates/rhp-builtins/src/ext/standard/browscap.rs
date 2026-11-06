//! ext/standard/browscap.c: `get_browser()` over the browscap ini file
//! (the `browscap` directive), read with the INI scanner in raw mode the
//! first time it is needed (php reads it at startup).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::zend::ini_parser::{Event, Parser};
use crate::zend::ini_scanner::{lexer, MODE_RAW};
use crate::{Host, Thrown};

const NUM_CONTAINS: usize = 5;
const DEFAULT_SECTION_NAME: &[u8] = b"Default Browser Capability Settings";

/// `browscap_entry`.
struct Entry {
    pattern: Vec<u8>,
    parent: Option<Vec<u8>>,
    /// Its keys (lowercased) and values, in file order.
    kv: Vec<(Vec<u8>, Vec<u8>)>,
    /// The leading run with no wildcard (at most 255).
    prefix_len: usize,
    /// Literal runs the agent must contain, in order: (start, len).
    contains: [(usize, usize); NUM_CONTAINS],
}

/// `browser_data`: the sections by name, in file order.
#[derive(Default)]
struct Data {
    entries: Vec<Entry>,
    index: HashMap<Vec<u8>, usize>,
}

thread_local! {
    /// The parsed file and the path it came from.
    static DATA: RefCell<Option<(Vec<u8>, Rc<Data>)>> = const { RefCell::new(None) };
}

fn is_placeholder(c: u8) -> bool {
    c == b'?' || c == b'*'
}

/// browscap_compute_contains: the next literal run of two or more
/// characters from `start`; returns where the scan stopped.
fn compute_contains(p: &[u8], start: usize, out: &mut (usize, usize)) -> usize {
    let mut i = start;
    while i < p.len() {
        if !is_placeholder(p[i]) && i + 1 < p.len() && !is_placeholder(p[i + 1]) {
            break;
        }
        i += 1;
    }
    let cstart = i.min(u16::MAX as usize);
    while i < p.len() && !is_placeholder(p[i]) {
        i += 1;
    }
    *out = (cstart, (i - cstart).min(u8::MAX as usize));
    i
}

impl Entry {
    fn new(pattern: Vec<u8>) -> Entry {
        let prefix = pattern
            .iter()
            .position(|&c| is_placeholder(c))
            .unwrap_or(pattern.len())
            .min(u8::MAX as usize);
        let mut contains = [(0, 0); NUM_CONTAINS];
        let mut pos = prefix;
        for c in contains.iter_mut() {
            pos = compute_contains(&pattern, pos, c);
        }
        Entry {
            pattern,
            parent: None,
            kv: Vec::new(),
            prefix_len: prefix,
            contains,
        }
    }

    fn minimum_length(&self) -> usize {
        self.prefix_len + self.contains.iter().map(|c| c.1).sum::<usize>()
    }
}

/// browscap_convert_pattern: the pattern as a PCRE regex.
fn convert_pattern(p: &[u8]) -> Vec<u8> {
    let mut t = b"~^".to_vec();
    for &c in p {
        match c {
            b'?' => t.push(b'.'),
            b'*' => t.extend_from_slice(b".*"),
            b'.' | b'\\' | b'(' | b')' | b'~' | b'+' => {
                t.push(b'\\');
                t.push(c);
            }
            _ => t.push(c.to_ascii_lowercase()),
        }
    }
    t.extend_from_slice(b"$~");
    t
}

/// The browscap value of an ini value: the booleans as "1" / "".
fn ini_value(v: &[u8]) -> Vec<u8> {
    let l = v.to_ascii_lowercase();
    match l.as_slice() {
        b"on" | b"yes" | b"true" => b"1".to_vec(),
        b"no" | b"off" | b"none" | b"false" => Vec::new(),
        _ => v.to_vec(),
    }
}

/// browscap_read_file: parse `src` the way php_browscap_parser_cb builds
/// its table.
fn read(host: &mut dyn Host, src: &[u8]) -> Result<Data, Thrown> {
    let mut lx = lexer(src, MODE_RAW);
    let first = lx.next();
    let mut p = Parser {
        lx,
        tok: first,
        host: &*host,
    };
    let mut events = Vec::new();
    // What came before a syntax error is kept, as php's callbacks ran.
    let _ = p.statement_list(&mut events);
    let mut d = Data::default();
    let mut current: Option<usize> = None;
    for ev in events {
        match ev {
            Event::Section(name) => {
                let name = name.into_bytes();
                if name.len() > u16::MAX as usize {
                    let m = format!("Skipping excessively long pattern of length {}", name.len());
                    crate::warn(host, "get_browser", &m)?;
                    continue;
                }
                let e = Entry::new(name.clone());
                let idx = match d.index.get(&name) {
                    Some(&i) => {
                        d.entries[i] = e;
                        i
                    }
                    None => {
                        d.entries.push(e);
                        d.index.insert(name, d.entries.len() - 1);
                        d.entries.len() - 1
                    }
                };
                current = Some(idx);
            }
            Event::Entry(k, Some(v)) => {
                let Some(cur) = current else {
                    continue;
                };
                let (k, v) = (k.into_bytes(), v.into_bytes());
                let value = ini_value(&v);
                let entry = &mut d.entries[cur];
                if k.eq_ignore_ascii_case(b"parent") {
                    if entry.pattern.eq_ignore_ascii_case(&v) {
                        // php: an E_CORE_ERROR; the entry keeps its parent.
                        let m = format!(
                            "Invalid browscap ini file: 'Parent' value cannot be same as the section name: {} (in file {})",
                            String::from_utf8_lossy(&entry.pattern),
                            String::from_utf8_lossy(&crate::ini_value(b"browscap").flatten().unwrap_or_default())
                        );
                        host.emit_diagnostic(crate::E_CORE_ERROR, m.as_bytes())?;
                        continue;
                    }
                    entry.parent = Some(value);
                } else {
                    entry.kv.push((k.to_ascii_lowercase(), value));
                }
            }
            _ => {}
        }
    }
    Ok(d)
}

/// The parsed browscap file, read on first use.
fn data(host: &mut dyn Host) -> Result<Option<Rc<Data>>, Thrown> {
    let Some(file) = crate::ini_value(b"browscap")
        .flatten()
        .filter(|f| !f.is_empty())
    else {
        return Ok(None);
    };
    if let Some(d) = DATA.with(|d| {
        d.borrow()
            .as_ref()
            .filter(|(f, _)| *f == file)
            .map(|(_, d)| d.clone())
    }) {
        return Ok(Some(d));
    }
    let Ok(src) = std::fs::read(crate::zend::virtual_cwd::os_path(&file)) else {
        let m = format!(
            "Cannot open \"{}\" for reading",
            String::from_utf8_lossy(&file)
        );
        host.emit_diagnostic(crate::E_CORE_WARNING, m.as_bytes())?;
        return Ok(None);
    };
    let d = Rc::new(read(host, &src)?);
    DATA.with(|c| *c.borrow_mut() = Some((file, d.clone())));
    Ok(Some(d))
}

/// browscap_match_string_wildcard: `*` any run, `?` any character.
fn match_wildcard(s: &[u8], pat: &[u8]) -> bool {
    let at = |i: usize| pat.get(i).copied().unwrap_or(0);
    let (mut pc, mut sc) = (0usize, 0usize);
    let mut restore: Option<(usize, usize)> = None;
    while sc < s.len() {
        let p = at(pc);
        if p == b'*' {
            pc += 1;
            while pc < pat.len() && pat[pc] == b'*' {
                pc += 1;
            }
            if pc == pat.len() {
                return true;
            }
            if pat[pc] != b'?' {
                while sc < s.len() && s[sc] != pat[pc] {
                    sc += 1;
                }
            }
            restore = Some((pc, sc));
            continue;
        } else if p == s[sc] || p == b'?' {
            pc += 1;
            sc += 1;
            if pc != pat.len() {
                continue;
            }
            if sc == s.len() {
                return true;
            }
        }
        match &mut restore {
            Some((rp, rs)) => {
                pc = *rp;
                *rs += 1;
                sc = *rs;
            }
            None => return false,
        }
    }
    while pc < pat.len() && pat[pc] == b'*' {
        pc += 1;
    }
    pc == pat.len()
}

/// php_memnstr from `from`: where `needle` next occurs in `hay`.
fn memnstr(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from > hay.len() || needle.len() > hay.len() - from {
        return None;
    }
    hay[from..]
        .windows(needle.len().max(1))
        .position(|w| w == needle)
        .map(|i| from + i)
}

/// browser_reg_compare: true on an exact match (the search ends);
/// otherwise a wildcard match replaces `found` when it keeps more literal
/// characters.
fn reg_compare(
    i: usize,
    e: &Entry,
    agent: &[u8],
    found: &mut Option<usize>,
    prev_len: &mut usize,
) -> bool {
    let lc = e.pattern.to_ascii_lowercase();
    let mut cur = e.prefix_len;
    for &(start, len) in &e.contains {
        if len != 0 {
            let needle = &lc[start..start + len];
            match memnstr(agent, cur, needle) {
                Some(at) => cur = at + len,
                None => return false,
            }
        }
    }
    if agent == lc.as_slice() {
        *found = Some(i);
        return true;
    }
    if match_wildcard(&agent[e.prefix_len.min(agent.len())..], &lc[e.prefix_len..]) {
        let curr_len = e.prefix_len
            + e.pattern[e.prefix_len..]
                .iter()
                .filter(|&&c| !is_placeholder(c))
                .count();
        if found.is_none() || *prev_len < curr_len {
            *found = Some(i);
            *prev_len = curr_len;
        }
    }
    false
}

fn fn_get_browser(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let agent = match args.first().map(deref) {
        None | Some(Value::Null) => None,
        Some(v) => Some(conv::to_string_bytes(&v)),
    };
    let return_array = args.get(1).is_some_and(|v| conv::to_bool(&deref(v)));
    let Some(d) = data(host)? else {
        crate::warn(host, "get_browser", "browscap ini directive not set")?;
        return Ok(Value::Bool(false));
    };
    let agent = match agent {
        Some(a) => a,
        None => {
            let ua = match host.global_var(b"_SERVER") {
                Some(Value::Array(s)) => s
                    .get(&ArrayKey::from_bytes(b"HTTP_USER_AGENT"))
                    .map(|v| conv::to_string_bytes(&deref(v))),
                _ => None,
            };
            let Some(ua) = ua else {
                crate::warn(
                    host,
                    "get_browser",
                    "HTTP_USER_AGENT variable is not set, cannot determine user agent name",
                )?;
                return Ok(Value::Bool(false));
            };
            ua
        }
    };
    let lookup = agent.to_ascii_lowercase();
    let mut found = d.index.get(&lookup).copied();
    if found.is_none() {
        let mut prev_len = 0usize;
        for (i, e) in d.entries.iter().enumerate() {
            if lookup.len() < e.minimum_length() {
                continue;
            }
            let prefix_ok = (0..e.prefix_len)
                .all(|j| lookup.get(j) == Some(&e.pattern[j].to_ascii_lowercase()));
            if !prefix_ok {
                continue;
            }
            if reg_compare(i, e, &lookup, &mut found, &mut prev_len) {
                break;
            }
        }
        if found.is_none() {
            found = d.index.get(DEFAULT_SECTION_NAME).copied();
        }
    }
    let Some(mut idx) = found else {
        return Ok(Value::Bool(false));
    };
    let e = &d.entries[idx];
    let mut a = PhpArray::new();
    let k = |s: &[u8]| ArrayKey::from_bytes(s);
    a.insert(
        k(b"browser_name_regex"),
        Value::str(convert_pattern(&e.pattern)),
    );
    a.insert(k(b"browser_name_pattern"), Value::str(e.pattern.clone()));
    if let Some(p) = &e.parent {
        a.insert(k(b"parent"), Value::str(p.clone()));
    }
    // zend_hash_add: an earlier key (the entry's, then its nearer parents')
    // wins.
    let add = |a: &mut PhpArray, e: &Entry| {
        for (key, v) in &e.kv {
            let key = k(key);
            if a.get(&key).is_none() {
                a.insert(key, Value::str(v.clone()));
            }
        }
    };
    add(&mut a, e);
    while let Some(parent) = &d.entries[idx].parent {
        let Some(&p) = d.index.get(parent) else {
            break;
        };
        idx = p;
        add(&mut a, &d.entries[idx]);
    }
    if return_array {
        return Ok(Value::Array(a));
    }
    let props = a
        .iter()
        .map(|(key, v)| (conv::to_string_bytes(&key.to_value()), v.clone()))
        .collect();
    Ok(host.make_object(props))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("get_browser", fn_get_browser)]);
}
