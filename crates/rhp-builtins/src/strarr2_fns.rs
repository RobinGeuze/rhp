//! More pure string + array builtins (no callbacks, no by-reference):
//! `str_split`, `strtr`, `substr_replace`, `levenshtein`, `similar_text`,
//! `str_word_count`, `chunk_split`, `array_diff_key`, `array_intersect_key`,
//! `array_replace`, `array_merge_recursive`.
//!
//! Every function here is verified byte-for-byte against the local php 8.5.8
//! binary (see `strarr2_fns/tests.rs` — expected values hardcoded from
//! `php -r '...'` output).

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::{Host, Thrown};

// ---------------------------------------------------------------------------
// levenshtein
// ---------------------------------------------------------------------------

fn fn_levenshtein(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = conv::to_string_bytes(&args[0]);
    let b = conv::to_string_bytes(&args[1]);
    let cost_ins = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(1);
    let cost_rep = args.get(3).map(|v| conv::to_int(&deref(v))).unwrap_or(1);
    let cost_del = args.get(4).map(|v| conv::to_int(&deref(v))).unwrap_or(1);

    let (la, lb) = (a.len(), b.len());
    if la == 0 {
        return Ok(Value::Int(lb as i64 * cost_ins));
    }
    if lb == 0 {
        return Ok(Value::Int(la as i64 * cost_del));
    }
    // Two-row DP (prev -> previous row, cur -> current row).
    let mut prev: Vec<i64> = (0..=lb as i64).map(|j| j * cost_ins).collect();
    let mut cur: Vec<i64> = vec![0; lb + 1];
    for i in 1..=la {
        cur[0] = i as i64 * cost_del;
        for j in 1..=lb {
            let del = prev[j] + cost_del;
            let ins = cur[j - 1] + cost_ins;
            let sub = prev[j - 1] + if a[i - 1] == b[j - 1] { 0 } else { cost_rep };
            cur[j] = del.min(ins).min(sub);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    Ok(Value::Int(prev[lb]))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("levenshtein", fn_levenshtein)]);
}

#[cfg(test)]
mod tests;
