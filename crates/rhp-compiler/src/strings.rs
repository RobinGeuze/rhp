//! Scanner details of double-quoted strings and heredocs that the parser's
//! decoded values miss: the octal-overflow warning and the `"${name}"`
//! form. (The escapes themselves are decoded in vendor/mago-syntax,
//! parser/internal/escape.rs.)

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
