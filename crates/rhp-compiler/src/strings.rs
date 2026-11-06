//! Scanner details of double-quoted strings and heredocs that the parser's
//! decoded values miss: the octal-overflow warning, heredoc's `\"` and the
//! `"${name}"` form.

/// The `${` interpolations: the scanner's T_STRING_VARNAME form (a label
/// right after `${`, then `}` or `[`) reads `$name` / `$name[...]`;
/// anything else is a variable variable. The name, when it is the first.
pub(crate) fn dollar_brace_varname(src: &[u8], after_brace: usize) -> Option<&[u8]> {
    let rest = src.get(after_brace..)?;
    let first = *rest.first()?;
    if !(first.is_ascii_alphabetic() || first == b'_' || first >= 0x80) {
        return None;
    }
    let n = rest
        .iter()
        .take_while(|&&c| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80)
        .count();
    matches!(rest.get(n), Some(b'}' | b'[')).then_some(&rest[..n])
}

/// The octal escapes above `\377` in a double-quoted string or heredoc
/// body (zend_scan_escape_string's E_COMPILE_WARNING), with the byte
/// offset of each in `raw`.
pub(crate) fn octal_overflows(raw: &[u8]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] != b'\\' || i + 1 >= raw.len() {
            i += 1;
            continue;
        }
        let digits = raw[i + 1..]
            .iter()
            .take(3)
            .take_while(|c| (b'0'..=b'7').contains(c))
            .count();
        if digits == 3 && raw[i + 1] > b'3' {
            out.push((i, String::from_utf8_lossy(&raw[i + 1..i + 4]).into_owned()));
        }
        i += 1 + digits.max(1);
    }
    out
}

/// Decode the escapes of a heredoc body as php does: like a double-quoted
/// string, except that `\"` stays as written. `None` for anything this
/// does not handle.
fn decode(raw: &[u8], keep_quote: bool) -> Option<Vec<u8>> {
    let hex = |c: u8| (c as char).to_digit(16);
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        if c != b'\\' || i + 1 >= raw.len() {
            out.push(c);
            i += 1;
            continue;
        }
        let n = raw[i + 1];
        i += 2;
        match n {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'v' => out.push(0x0b),
            b'e' => out.push(0x1b),
            b'f' => out.push(0x0c),
            b'\\' => out.push(b'\\'),
            b'$' => out.push(b'$'),
            b'"' if !keep_quote => out.push(b'"'),
            b'0'..=b'7' => {
                let mut v = (n - b'0') as u32;
                let mut k = 0;
                while k < 2 && i < raw.len() && (b'0'..=b'7').contains(&raw[i]) {
                    v = v * 8 + (raw[i] - b'0') as u32;
                    i += 1;
                    k += 1;
                }
                out.push(v as u8);
            }
            b'x' if i < raw.len() && hex(raw[i]).is_some() => {
                let mut v = hex(raw[i])?;
                i += 1;
                if i < raw.len() {
                    if let Some(d) = hex(raw[i]) {
                        v = v * 16 + d;
                        i += 1;
                    }
                }
                out.push(v as u8);
            }
            b'u' => return None,
            other => {
                out.push(b'\\');
                out.push(other);
            }
        }
    }
    Some(out)
}

/// A heredoc literal part's value with php's `\"` (kept as written), when
/// the parser's `value` decoded it. Only for an unindented heredoc, and only
/// when decoding `raw` the parser's way reproduces `value`.
pub(crate) fn heredoc_value(raw: &[u8], value: &[u8]) -> Option<Vec<u8>> {
    if !raw.windows(2).any(|w| w == b"\\\"") {
        return None;
    }
    let theirs = decode(raw, false)?;
    let ours = decode(raw, true)?;
    // The parser may drop the newline before the closing label.
    let cut = theirs.len().checked_sub(value.len())?;
    if theirs[..value.len()] != *value
        || !theirs[value.len()..]
            .iter()
            .all(|&b| b == b'\n' || b == b'\r')
    {
        return None;
    }
    Some(ours[..ours.len() - cut].to_vec())
}
