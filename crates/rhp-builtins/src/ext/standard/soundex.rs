//! ext/standard/soundex.c: `soundex`.

use rhp_values::{conv, Value};

use crate::{Host, Thrown};

// ---------------------------------------------------------------------------
// soundex
// ---------------------------------------------------------------------------

/// `soundex($string)`: the four-character American Soundex code (a faithful port
/// of php's `php_soundex`). Non-letters are skipped; the empty / letterless input
/// yields `"0000"`.
pub(crate) fn fn_soundex(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // soundex_table[c-'A'] for A..Z; '0' means "no code" (vowel / h / w / y).
    const TABLE: &[u8; 26] = b"01230120022455012623010202";
    let s = conv::to_string_bytes(&args[0]);
    let mut out = Vec::with_capacity(4);
    let mut last = 0u8;
    for &b in &s {
        if out.len() >= 4 {
            break;
        }
        let up = b.to_ascii_uppercase();
        if !up.is_ascii_uppercase() {
            continue;
        }
        let code = TABLE[(up - b'A') as usize];
        if out.is_empty() {
            out.push(up);
            last = code;
        } else if code != last {
            if code != b'0' {
                out.push(code);
            }
            last = code;
        }
    }
    while out.len() < 4 {
        out.push(b'0');
    }
    Ok(Value::str(out))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("soundex", fn_soundex)]);
}
