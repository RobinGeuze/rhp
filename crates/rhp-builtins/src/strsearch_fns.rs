//! Substring-returning search builtins from `ext/standard/string.c`:
//! `strstr` / `strchr`, `stristr`, `strrchr`, and the stateful `strtok`.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::argparse::{bool_param, str_or_null_param, str_param};
use crate::{Builtin, Host, Thrown};
use rhp_values::Value;

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    if n.is_empty() {
        return Some(0);
    }
    if n.len() > h.len() {
        return None;
    }
    h.windows(n.len()).position(|w| w == n)
}

fn find_ci(h: &[u8], n: &[u8]) -> Option<usize> {
    if n.is_empty() {
        return Some(0);
    }
    if n.len() > h.len() {
        return None;
    }
    h.windows(n.len()).position(|w| w.eq_ignore_ascii_case(n))
}

fn split_result(h: &[u8], at: Option<usize>, before: bool) -> Value {
    match at {
        None => Value::Bool(false),
        Some(p) if before => Value::str(h[..p].to_vec()),
        Some(p) => Value::str(h[p..].to_vec()),
    }
}

fn fn_strstr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = str_param(host, args, 0, "strstr", "haystack")?;
    let n = str_param(host, args, 1, "strstr", "needle")?;
    let before = bool_param(host, args, 2, "strstr", "before_needle", false)?;
    Ok(split_result(&h, find(&h, &n), before))
}

fn fn_strchr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = str_param(host, args, 0, "strchr", "haystack")?;
    let n = str_param(host, args, 1, "strchr", "needle")?;
    let before = bool_param(host, args, 2, "strchr", "before_needle", false)?;
    Ok(split_result(&h, find(&h, &n), before))
}

fn fn_stristr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = str_param(host, args, 0, "stristr", "haystack")?;
    let n = str_param(host, args, 1, "stristr", "needle")?;
    let before = bool_param(host, args, 2, "stristr", "before_needle", false)?;
    Ok(split_result(&h, find_ci(&h, &n), before))
}

/// `strrchr`: only the needle's first byte is searched for (an empty needle
/// searches for its NUL terminator, as php's `*ZSTR_VAL(needle)` does).
fn fn_strrchr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = str_param(host, args, 0, "strrchr", "haystack")?;
    let n = str_param(host, args, 1, "strrchr", "needle")?;
    let before = bool_param(host, args, 2, "strrchr", "before_needle", false)?;
    let c = n.first().copied().unwrap_or(0);
    Ok(split_result(&h, h.iter().rposition(|&b| b == c), before))
}

/// strtok's per-request state (`BG(strtok_string)` / `BG(strtok_last)`).
struct TokState {
    string: Option<Vec<u8>>,
    last: usize,
}

thread_local! {
    static STRTOK: RefCell<TokState> = const { RefCell::new(TokState { string: None, last: 0 }) };
}

pub(crate) fn reset_state() {
    STRTOK.with(|t| {
        *t.borrow_mut() = TokState {
            string: None,
            last: 0,
        }
    });
}

fn fn_strtok(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = str_param(host, args, 0, "strtok", "string")?;
    let tok = str_or_null_param(host, args, 1, "strtok", "token")?;
    STRTOK.with(|st| {
        let mut st = st.borrow_mut();
        let tok = match tok {
            None => s,
            Some(t) => {
                st.string = Some(s);
                st.last = 0;
                t
            }
        };
        let Some(string) = st.string.as_ref() else {
            // php: Warning "Both arguments must be provided when starting tokenization"
            return Ok(Value::Bool(false));
        };
        let pe = string.len();
        let mut p = st.last;
        if p >= pe {
            return Ok(Value::Bool(false));
        }
        let mut table = [false; 256];
        for &b in &tok {
            table[b as usize] = true;
        }
        let mut skipped = 0;
        while table[string[p] as usize] {
            p += 1;
            if p >= pe {
                st.string = None;
                return Ok(Value::Bool(false));
            }
            skipped += 1;
        }
        loop {
            p += 1;
            if p >= pe || table[string[p] as usize] {
                break;
            }
        }
        if p - st.last > 0 {
            let out = string[st.last + skipped..p].to_vec();
            st.last = p + 1;
            Ok(Value::str(out))
        } else {
            st.string = None;
            Ok(Value::Bool(false))
        }
    })
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    let mut put = |name: &'static str, func: crate::NativeFn, min: usize, max: usize| {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: Some(max),
            },
        );
    };
    put("strstr", fn_strstr, 2, 3);
    put("strchr", fn_strchr, 2, 3);
    put("stristr", fn_stristr, 2, 3);
    put("strrchr", fn_strrchr, 2, 3);
    put("strtok", fn_strtok, 1, 2);
}
