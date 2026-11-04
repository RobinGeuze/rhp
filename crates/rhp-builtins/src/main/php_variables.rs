//! main/php_variables.c: `php_default_treat_data` and
//! `php_register_variable_ex`, with php's exact key mangling, for the
//! request's variables and `parse_str`.

use rhp_values::{ArrayKey, PhpArray, Value};

// ---------------------------------------------------------------------------
// parse_str
// ---------------------------------------------------------------------------

fn hexv(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// `php_url_decode`: `+` → space, `%XX` → byte, malformed escapes kept.
pub fn url_decode(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let b = s[i];
        if b == b'+' {
            out.push(b' ');
        } else if b == b'%' && i + 2 < s.len() {
            if let (Some(h), Some(l)) = (hexv(s[i + 1]), hexv(s[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
            out.push(b);
        } else {
            out.push(b);
        }
        i += 1;
    }
    out
}

/// `ZEND_HANDLE_NUMERIC_STR` + string key: canonical decimal ints become int keys.
fn sym_key(k: &[u8]) -> ArrayKey {
    ArrayKey::from_bytes(k)
}

fn is_forbidden(mangled: &[u8], pre: &[u8]) -> bool {
    (mangled.starts_with(b"__Host-") && !pre.starts_with(b"__Host-"))
        || (mangled.starts_with(b"__Secure-") && !pre.starts_with(b"__Secure-"))
}

/// `max_input_nesting_level` (default 64).
fn max_nesting() -> i64 {
    crate::main::ini::ini_value(b"max_input_nesting_level")
        .flatten()
        .map_or(64, |v| crate::main::ini::c_atol(&v))
}

/// Walk to `path` inside `root`, creating/replacing arrays as php does, and
/// return the target array. Each path element is `Some(key)` or `None` (`[]`
/// append). Returns `None` when an append fails.
fn descend<'a>(arr: &'a mut PhpArray, key: Option<&[u8]>) -> Option<&'a mut PhpArray> {
    let k = match key {
        None => {
            arr.push(Value::Array(PhpArray::new()));
            // the just-pushed element is the last one
            let last = arr.iter().last().map(|(k, _)| k.clone())?;
            last
        }
        Some(k) => {
            let k = sym_key(k);
            match arr.get(&k) {
                Some(Value::Array(_)) => {}
                _ => {
                    arr.insert(k.clone(), Value::Array(PhpArray::new()));
                }
            }
            k
        }
    };
    match arr.get_mut(&k) {
        Some(Value::Array(a)) => Some(a),
        _ => None,
    }
}

/// `php_register_variable_ex(var_name, val, array)`.
pub(crate) fn register_variable(root: &mut PhpArray, var_name: &[u8], val: Vec<u8>) {
    register_variable_ex(root, var_name, val, false)
}

/// [`register_variable`]; `cookie`: the target is `$_COOKIE`, where a
/// duplicate top-level name keeps the first value (rfc2965 ordering).
pub(crate) fn register_variable_ex(
    root: &mut PhpArray,
    var_name: &[u8],
    val: Vec<u8>,
    cookie: bool,
) {
    register_variable_value(root, var_name, Value::str(val), cookie)
}

/// [`register_variable_ex`] with any value (the upload parser registers
/// `$_FILES[..][error]` / `[size]` as ints).
pub(crate) fn register_variable_value(
    root: &mut PhpArray,
    var_name: &[u8],
    value: Value,
    cookie: bool,
) {
    // C string: the name stops at its first NUL.
    let var_name = match var_name.iter().position(|&b| b == 0) {
        Some(p) => &var_name[..p],
        None => var_name,
    };
    let mut start = 0;
    while start < var_name.len() && var_name[start] == b' ' {
        start += 1;
    }
    let var_name = &var_name[start..];
    let mut var = var_name.to_vec();
    // Mangle ' ' and '.' up to the first '['.
    let mut bracket = None;
    for (i, c) in var.iter_mut().enumerate() {
        if *c == b' ' || *c == b'.' {
            *c = b'_';
        } else if *c == b'[' {
            bracket = Some(i);
            break;
        }
    }
    let var_len = bracket.unwrap_or(var.len());
    if var_len == 0 {
        return;
    }
    let Some(mut ip) = bracket else {
        // plain var
        let index = &var[..var_len];
        if is_forbidden(index, var_name) {
            return;
        }
        if cookie && root.get(&sym_key(index)).is_some() {
            return;
        }
        root.insert(sym_key(index), value);
        return;
    };

    // Array form: collect path segments.
    let mut index: Option<Vec<u8>> = Some(var[..var_len].to_vec());
    let top = sym_key(&var[..var_len]);
    let max_nest = max_nesting();
    let mut cur: &mut PhpArray = root;
    let mut nest = 0i64;
    loop {
        nest += 1;
        if nest > max_nest {
            break;
        }
        // var[ip] == '['
        let mut p = ip + 1;
        let index_s_start = p;
        if p < var.len() && matches!(var[p], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
            p += 1;
        }
        let new_index: Option<Vec<u8>>;
        let close;
        if p < var.len() && var[p] == b']' {
            new_index = None;
            close = p;
        } else {
            match var[p.min(var.len())..].iter().position(|&b| b == b']') {
                Some(off) => {
                    close = p + off;
                    new_index = Some(var[index_s_start..close].to_vec());
                }
                None => {
                    // Not an index: php un-terminates the '[' — at the top
                    // level that splices the mangled rest onto the name; deeper
                    // down the previous ']' NUL still ends the name.
                    if index.is_none() {
                        cur.push(value);
                        return;
                    }
                    let mut name = index.clone().unwrap_or_default();
                    if nest == 1 {
                        name.push(b'_');
                        for &c in &var[index_s_start..] {
                            name.push(if matches!(c, b' ' | b'.' | b'[') {
                                b'_'
                            } else {
                                c
                            });
                        }
                    }
                    if is_forbidden(&name, var_name) {
                        return;
                    }
                    cur.insert(sym_key(&name), value);
                    return;
                }
            }
        }
        // Step into `index` (create/replace as array).
        if let Some(ix) = &index {
            if is_forbidden(ix, var_name) {
                return;
            }
        }
        cur = match descend(cur, index.as_deref()) {
            Some(a) => a,
            None => return,
        };
        index = new_index;
        let after = close + 1;
        if after < var.len() && var[after] == b'[' {
            ip = after;
            continue;
        }
        // plain_var with the current index
        match index {
            None => {
                cur.push(value);
            }
            Some(ix) => {
                if is_forbidden(&ix, var_name) {
                    return;
                }
                cur.insert(sym_key(&ix), value);
            }
        }
        return;
    }
    // Too deeply nested: php drops the whole top-level variable.
    root.remove(&top);
}

/// `php_raw_url_decode`: `%XX` → byte (no `+` → space).
pub(crate) fn raw_url_decode(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%' && i + 2 < s.len() {
            if let (Some(h), Some(l)) = (hexv(s[i + 1]), hexv(s[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

/// Which request input `php_default_treat_data` parses.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TreatData {
    /// The query string (`$_GET`) or an urlencoded body (`$_POST`):
    /// split on any `arg_separator.input` byte, names and values
    /// `php_url_decode`d.
    Form,
    /// The `Cookie` header (`$_COOKIE`): split on `;`, leading blanks
    /// dropped from names, values raw-url-decoded, first duplicate wins.
    Cookie,
}

/// `php_default_treat_data`: parse request input into a superglobal array.
/// Returns the array and whether `max_input_vars` cut the input short.
pub fn treat_data(
    kind: TreatData,
    data: &[u8],
    separators: &[u8],
    max_vars: i64,
) -> (PhpArray, bool) {
    let mut out = PhpArray::new();
    let data = match data.iter().position(|&b| b == 0) {
        Some(p) => &data[..p],
        None => data,
    };
    let seps: &[u8] = match kind {
        TreatData::Form => separators,
        TreatData::Cookie => b";",
    };
    let mut count = 0i64;
    for var in data.split(|b| seps.contains(b)).filter(|t| !t.is_empty()) {
        let mut var = var;
        if kind == TreatData::Cookie {
            let lead = var
                .iter()
                .take_while(|&&b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
                .count();
            var = &var[lead..];
            if var.is_empty() || var[0] == b'=' {
                continue;
            }
        }
        let eq = var.iter().position(|&b| b == b'=');
        count += 1;
        if count > max_vars {
            return (out, true);
        }
        let (name, val) = match eq {
            Some(p) => {
                let raw = &var[p + 1..];
                let v = match kind {
                    TreatData::Cookie => raw_url_decode(raw),
                    TreatData::Form => url_decode(raw),
                };
                (&var[..p], v)
            }
            None => (var, Vec::new()),
        };
        let name = match kind {
            TreatData::Cookie => name.to_vec(),
            TreatData::Form => url_decode(name),
        };
        register_variable_ex(&mut out, &name, val, kind == TreatData::Cookie);
    }
    (out, false)
}

/// `php_std_post_handler` (`add_post_vars`): an urlencoded request body
/// into `$_POST`. Unlike [`treat_data`] it splits on `&` only (never
/// `arg_separator.input`), counts empty pairs, and checks
/// `max_input_vars` after registering (so one variable more is kept).
/// Returns the array and whether the limit was exceeded.
pub fn post_form_vars(body: &[u8], max_vars: i64) -> (PhpArray, bool) {
    let mut out = PhpArray::new();
    let mut ptr = 0;
    let mut count = 0i64;
    while ptr < body.len() {
        let vsep = body[ptr..]
            .iter()
            .position(|&b| b == b'&')
            .map_or(body.len(), |p| ptr + p);
        let pair = &body[ptr..vsep];
        let (name, val) = match pair.iter().position(|&b| b == b'=') {
            Some(k) => (url_decode(&pair[..k]), url_decode(&pair[k + 1..])),
            None => (url_decode(pair), Vec::new()),
        };
        register_variable_ex(&mut out, &name, val, false);
        ptr = if vsep == body.len() { vsep } else { vsep + 1 };
        count += 1;
        if count > max_vars {
            return (out, true);
        }
    }
    (out, false)
}
