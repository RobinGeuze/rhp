//! Zend/zend_highlight.c: `zend_highlight` (the HTML of highlight_string /
//! highlight_file) and `zend_strip` (php_strip_whitespace), over the
//! tokenizer's port of the Zend scanner.

use crate::ext::tokenizer::ids::*;
use crate::ext::tokenizer::lexer;

/// php's `zend_syntax_highlighter_ini`: the highlight.* colours.
pub struct Colors {
    pub comment: Vec<u8>,
    pub default: Vec<u8>,
    pub html: Vec<u8>,
    pub keyword: Vec<u8>,
    pub string: Vec<u8>,
}

impl Colors {
    /// php_get_highlight_struct: the current highlight.* values.
    pub fn from_ini() -> Colors {
        let get = |n: &[u8]| crate::ini_value(n).flatten().unwrap_or_default();
        Colors {
            comment: get(b"highlight.comment"),
            default: get(b"highlight.default"),
            html: get(b"highlight.html"),
            keyword: get(b"highlight.keyword"),
            string: get(b"highlight.string"),
        }
    }
}

/// zend_html_puts.
fn html_puts(out: &mut Vec<u8>, s: &[u8]) {
    for &c in s {
        match c {
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            b'&' => out.extend_from_slice(b"&amp;"),
            b'\t' => out.extend_from_slice(b"    "),
            _ => out.push(c),
        }
    }
}

/// Which colour a token gets: an index into [`Colors`] order.
#[derive(Clone, Copy, PartialEq)]
enum Color {
    Comment,
    Default,
    Html,
    Keyword,
    String,
}

/// Tokens the scanner gives a value (RETURN_TOKEN_WITH_STR / _WITH_VAL):
/// highlighted in the default colour, the rest as keywords.
fn has_value(id: i64) -> bool {
    matches!(
        id,
        T_STRING
            | T_VARIABLE
            | T_NAME_FULLY_QUALIFIED
            | T_NAME_QUALIFIED
            | T_NAME_RELATIVE
            | T_STRING_VARNAME
            | T_LNUMBER
            | T_DNUMBER
            | T_NUM_STRING
    )
}

/// zend_highlight over `src`.
pub fn highlight(src: &[u8], c: &Colors) -> Vec<u8> {
    let color_of = |k: Color| -> &[u8] {
        match k {
            Color::Comment => &c.comment,
            Color::Default => &c.default,
            Color::Html => &c.html,
            Color::Keyword => &c.keyword,
            Color::String => &c.string,
        }
    };
    let mut out = Vec::with_capacity(src.len() * 2);
    out.extend_from_slice(b"<pre><code style=\"color: ");
    out.extend_from_slice(&c.html);
    out.extend_from_slice(b"\">");
    let mut last = Color::Html;
    for t in lexer::tokenize_scanner(src) {
        let text = t.text(src);
        let next = match t.id {
            T_INLINE_HTML => Color::Html,
            T_COMMENT | T_DOC_COMMENT => Color::Comment,
            T_OPEN_TAG | T_OPEN_TAG_WITH_ECHO | T_CLOSE_TAG | T_LINE | T_FILE | T_DIR
            | T_TRAIT_C | T_METHOD_C | T_FUNC_C | T_PROPERTY_C | T_NS_C | T_CLASS_C => {
                Color::Default
            }
            T_ENCAPSED_AND_WHITESPACE | T_CONSTANT_ENCAPSED_STRING => Color::String,
            id if id == b'"' as i64 => Color::String,
            T_WHITESPACE => {
                html_puts(&mut out, text);
                continue;
            }
            id if has_value(id) => Color::Default,
            _ => Color::Keyword,
        };
        // php compares the colour strings' addresses: two directives with
        // the same value still count as different colours.
        if last != next {
            if last != Color::Html {
                out.extend_from_slice(b"</span>");
            }
            last = next;
            if last != Color::Html {
                out.extend_from_slice(b"<span style=\"color: ");
                out.extend_from_slice(color_of(last));
                out.extend_from_slice(b"\">");
            }
        }
        html_puts(&mut out, text);
    }
    if last != Color::Html {
        out.extend_from_slice(b"</span>");
    }
    out.extend_from_slice(b"</code></pre>");
    out
}

/// zend_strip over `src`: comments dropped, whitespace runs as one space.
pub fn strip(src: &[u8]) -> Vec<u8> {
    let toks = lexer::tokenize_scanner(src);
    let mut out = Vec::with_capacity(src.len());
    let mut prev_space = false;
    let mut it = toks.iter();
    while let Some(t) = it.next() {
        match t.id {
            T_WHITESPACE => {
                if !prev_space {
                    out.push(b' ');
                    prev_space = true;
                }
                continue;
            }
            T_COMMENT | T_DOC_COMMENT => continue,
            T_END_HEREDOC => {
                out.extend_from_slice(t.text(src));
                // The next token: a newline (dropped) or `;` and the like.
                if let Some(n) = it.next() {
                    if n.id != T_WHITESPACE {
                        out.extend_from_slice(n.text(src));
                    }
                }
                out.push(b'\n');
                prev_space = true;
                continue;
            }
            _ => out.extend_from_slice(t.text(src)),
        }
        prev_space = false;
    }
    out
}

/// The scanner's SHEBANG state, on for a file opened for scanning
/// (highlight_file, php_strip_whitespace): a first line `#!...` and its
/// newline are skipped.
pub fn skip_shebang(src: &[u8]) -> &[u8] {
    if !src.starts_with(b"#!") {
        return src;
    }
    // `"#!" .* {NEWLINE}`, longest match: `.` is anything but "\n".
    match src.iter().position(|&c| c == b'\n') {
        Some(i) => &src[i + 1..],
        None => match src.iter().rposition(|&c| c == b'\r') {
            Some(i) => &src[i + 1..],
            None => src,
        },
    }
}
