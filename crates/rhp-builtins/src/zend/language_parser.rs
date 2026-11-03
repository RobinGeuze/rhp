//! php's parse errors, exactly as php reports them: rhp's port of the Zend
//! scanner feeds php's own LALR automaton (Zend/zend_language_parser.y, Bison
//! tables from the release tarball), with the scanner's parser-mode errors
//! (bracket nesting, `(real)`, octal literals, unterminated comments, `\u{}`
//! escapes, heredoc indentation) raised when php's scanner raises them.

use super::bison::{self, Outcome};
use super::language_parser_tables::TABLES;
use crate::ext::tokenizer::ids::*;
use crate::ext::tokenizer::lexer::{self, Tok};

/// The first error php raises compiling `src`, as `(message, line)`: a
/// syntax error (`syntax error, unexpected …, expecting …`) or a scanner
/// error. `None` when php's grammar accepts the file. Line 0: php raises the
/// error outside the file (the includer's line for include/eval, no file
/// at all for the main script).
pub fn syntax_error(src: &[u8]) -> Option<(String, u32)> {
    let toks = lexer::tokenize(src);
    let mut feed = Feed {
        src,
        toks: &toks,
        i: 0,
        nesting: Vec::new(),
        halt: None,
        last: None,
        strings: Vec::new(),
        error: None,
        final_line: 1 + count_newlines(src),
    };
    let outcome = TABLES.parse(|| feed.next());
    if let Some(e) = feed.error.take() {
        return Some(e);
    }
    match outcome {
        Outcome::Accept => None,
        Outcome::MemoryExhausted => Some(("memory exhausted".into(), feed.line())),
        Outcome::SyntaxError { args } => {
            let at_end = args.first() == Some(&0);
            let msg = bison::syntax_error_message(&TABLES, &args, |name, unexpected| {
                if unexpected {
                    match feed.last {
                        Some(i) if !at_end => unexpected_name(name, toks[i].text(src)),
                        _ => b"end of file".to_vec(),
                    }
                } else {
                    expected_name(name)
                }
            });
            Some((String::from_utf8_lossy(&msg).into_owned(), feed.line()))
        }
    }
}

/// The scanner as the parser pulls tokens from it (`zendlex` in parser
/// mode): whitespace, comments and open tags are skipped, `<?=` is `echo`,
/// `?>` is `;`, and lexing stops after `__halt_compiler();`.
struct Feed<'a> {
    src: &'a [u8],
    toks: &'a [Tok],
    i: usize,
    /// The scanner's nest_location_stack: (bracket, line).
    nesting: Vec<(u8, u32)>,
    /// Significant tokens still to come after T_HALT_COMPILER.
    halt: Option<u8>,
    /// The token last handed to the parser (None: end of file).
    last: Option<usize>,
    /// The strings being scanned, innermost last (code inside `{$ }` is
    /// `Ctx::Code`).
    strings: Vec<Ctx>,
    error: Option<(String, u32)>,
    final_line: u32,
}

impl Feed<'_> {
    /// CG(zend_lineno) once the last token was scanned.
    fn line(&self) -> u32 {
        let Some(i) = self.last else {
            return self.final_line;
        };
        let t = &self.toks[i];
        let mut line = t.line as u32 + count_newlines(t.text(self.src));
        if defers_newline(self.src, self.toks, i) {
            line -= 1;
        }
        line
    }

    fn fail(&mut self, msg: String, line: u32) -> i32 {
        self.error = Some((msg, line));
        SCANNER_ERROR
    }

    fn next(&mut self) -> i32 {
        if self.error.is_some() {
            return SCANNER_ERROR;
        }
        if self.halt == Some(0) {
            return self.end();
        }
        while let Some(t) = self.toks.get(self.i) {
            let i = self.i;
            self.i += 1;
            let line = t.line as u32;
            match t.id {
                T_WHITESPACE | T_OPEN_TAG => continue,
                T_COMMENT | T_DOC_COMMENT => {
                    let text = t.text(self.src);
                    if text.starts_with(b"/*") && (text.len() < 4 || !text.ends_with(b"*/")) {
                        return self
                            .fail(format!("Unterminated comment starting line {line}"), line);
                    }
                    continue;
                }
                _ => {}
            }
            self.last = Some(i);
            if let Some(e) = self.string_context(i) {
                return self.fail(e.0, e.1);
            }
            let nowdoc = matches!(self.strings.last(), Some(Ctx::Heredoc(h)) if h.nowdoc);
            if let Some(e) = scanner_error(t, t.text(self.src), nowdoc) {
                return self.fail(e.0, e.1);
            }
            // bracket nesting (enter_nesting / exit_nesting)
            let opening = match t.id {
                T_CURLY_OPEN | T_DOLLAR_OPEN_CURLY_BRACES => Some(b'{'),
                T_ATTRIBUTE => Some(b'['),
                c if t.scripting && c < 256 && matches!(c as u8, b'{' | b'[' | b'(') => {
                    Some(c as u8)
                }
                _ => None,
            };
            if let Some(o) = opening {
                self.nesting.push((o, line));
            } else if t.scripting && matches!(t.id, 0x7D | 0x5D | 0x29) {
                let closing = t.id as u8;
                match self.nesting.last() {
                    None => return self.fail(format!("Unmatched '{}'", closing as char), line),
                    Some(&(o, ol)) => {
                        let ok = matches!((o, closing), (b'{', b'}') | (b'[', b']') | (b'(', b')'));
                        if !ok {
                            return self.fail(bad_nesting(o, ol, line, Some(closing)), line);
                        }
                        self.nesting.pop();
                    }
                }
            }
            let id = match t.id {
                T_OPEN_TAG_WITH_ECHO => T_ECHO,
                T_CLOSE_TAG => b';' as i64,
                id => id,
            };
            match self.halt {
                Some(n) => self.halt = Some(n - 1),
                None if id == T_HALT_COMPILER && self.nesting.is_empty() => self.halt = Some(3),
                None => {}
            }
            return id as i32;
        }
        self.end()
    }

    /// Tracks which string token `i` is in, and raises the scanner's heredoc
    /// indentation errors.
    fn string_context(&mut self, i: usize) -> Option<(String, u32)> {
        let (src, toks) = (self.src, self.toks);
        let t = &toks[i];
        let in_code = !matches!(
            self.strings.last(),
            Some(Ctx::Dq | Ctx::Bq | Ctx::Heredoc(_))
        );
        match t.id {
            T_START_HEREDOC => {
                let h = heredoc_info(src, toks, i);
                self.strings.push(Ctx::Heredoc(h));
                // an empty heredoc: the label is checked right away
                let empty = toks.get(i + 1).filter(|n| n.id == T_END_HEREDOC);
                if empty.is_some_and(|n| label_indent(src, n).2) {
                    let line = t.line as u32 + count_newlines(t.text(src));
                    return Some((MIXED.into(), line));
                }
                // The scan-ahead's check on a body that starts with a
                // variable. php raises it outside any compiled file: line 0
                // (see `syntax_error`).
                let first = toks.get(i + 1).map(|n| n.id);
                let var = matches!(
                    first,
                    Some(T_VARIABLE | T_DOLLAR_OPEN_CURLY_BRACES | T_CURLY_OPEN)
                );
                if !h.nowdoc && h.indentation > 0 && var {
                    return Some((body_indentation(h.indentation), 0));
                }
            }
            T_END_HEREDOC => {
                self.strings.pop();
            }
            0x22 | 0x60 if in_code => {
                self.strings
                    .push(if t.id == 0x22 { Ctx::Dq } else { Ctx::Bq })
            }
            0x22 | 0x60 => {
                self.strings.pop();
            }
            T_CURLY_OPEN | T_DOLLAR_OPEN_CURLY_BRACES => self.strings.push(Ctx::Code),
            0x7B if in_code && !self.strings.is_empty() => self.strings.push(Ctx::Code),
            0x7D if matches!(self.strings.last(), Some(Ctx::Code)) => {
                self.strings.pop();
            }
            T_ENCAPSED_AND_WHITESPACE => {
                if let Some(&Ctx::Heredoc(h)) = self.strings.last() {
                    return heredoc_body_error(src, toks, i, h);
                }
            }
            _ => {}
        }
        None
    }

    /// END, or the scanner's check_nesting_at_end error.
    fn end(&mut self) -> i32 {
        if self.halt != Some(0) {
            self.last = None;
        }
        if let Some(&(o, ol)) = self.nesting.last() {
            let line = self.line();
            return self.fail(bad_nesting(o, ol, line, None), line);
        }
        0
    }
}

/// `report_bad_nesting`.
fn bad_nesting(opening: u8, open_line: u32, line: u32, closing: Option<u8>) -> String {
    let mut m = format!("Unclosed '{}'", opening as char);
    if open_line != line {
        m.push_str(&format!(" on line {open_line}"));
    }
    if let Some(c) = closing {
        m.push_str(&format!(" does not match '{}'", c as char));
    }
    m
}

#[derive(Clone, Copy)]
enum Ctx {
    Code,
    Dq,
    Bq,
    Heredoc(Heredoc),
}

/// A heredoc's closing label indentation.
#[derive(Clone, Copy)]
struct Heredoc {
    nowdoc: bool,
    indentation: usize,
    uses_spaces: bool,
}

const MIXED: &str = "Invalid indentation - tabs and spaces cannot be mixed";

fn body_indentation(n: usize) -> String {
    format!("Invalid body indentation level (expecting an indentation level of at least {n})")
}

/// A closing label's leading whitespace: (length, spaces only, mixed).
fn label_indent(src: &[u8], t: &Tok) -> (usize, bool, bool) {
    let ws = t.text(src).iter().take_while(|&&c| c == b' ' || c == b'\t');
    let (mut tabs, mut spaces, mut n) = (false, false, 0);
    for &c in ws {
        n += 1;
        tabs |= c == b'\t';
        spaces |= c == b' ';
    }
    (n, spaces && !tabs, tabs && spaces)
}

/// The indentation php strips from the heredoc T_START_HEREDOC at `i`
/// opens. A heredoc's comes from its scan-ahead, which keeps the last
/// closing label it meets after a heredoc body (an inner one's too, which
/// counts when the heredoc's own label is missing) and stops at a label
/// that mixes tabs and spaces; a nowdoc's is its own.
fn heredoc_info(src: &[u8], toks: &[Tok], i: usize) -> Heredoc {
    let is_nowdoc = |t: &Tok| t.text(src).contains(&b'\'');
    let nowdoc = is_nowdoc(&toks[i]);
    let mut open = vec![nowdoc];
    let mut label = (0, false);
    for (k, t) in toks.iter().enumerate().skip(i + 1) {
        match t.id {
            T_START_HEREDOC => open.push(is_nowdoc(t)),
            T_END_HEREDOC => {
                let inner_nowdoc = open.pop().unwrap_or(false);
                let with_body = toks[k - 1].id != T_START_HEREDOC;
                let (n, spaces, mixed) = label_indent(src, t);
                if open.is_empty() {
                    if nowdoc || with_body {
                        label = (n, spaces);
                    }
                    break;
                }
                if !nowdoc && with_body {
                    if !inner_nowdoc {
                        label = (n, spaces);
                    }
                    if mixed {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    Heredoc {
        nowdoc,
        indentation: label.0,
        uses_spaces: label.1,
    }
}

/// The scanner's checks on a heredoc's text token `i`: a mixed closing label
/// (raised when the label is found), then strip_multiline_string_indentation.
fn heredoc_body_error(src: &[u8], toks: &[Tok], i: usize, h: Heredoc) -> Option<(String, u32)> {
    let t = &toks[i];
    let line = t.line as u32;
    let label = toks.get(i + 1).filter(|n| n.id == T_END_HEREDOC);
    let at_label = label.is_some();
    if label.is_some_and(|n| label_indent(src, n).2) {
        return Some((MIXED.into(), line));
    }
    let mut value = t.text(src);
    let (mut indentation, mut uses_spaces) = (h.indentation, h.uses_spaces);
    if h.nowdoc && !at_label {
        // an unterminated nowdoc: the whitespace after its last newline,
        // unless that runs to the end of the file
        let nl = value.iter().rposition(|&c| c == b'\n' || c == b'\r')?;
        let ws = &value[nl + 1..];
        let n = ws.iter().take_while(|&&c| c == b' ' || c == b'\t').count();
        if n == ws.len() {
            return None;
        }
        indentation = n;
        uses_spaces = !ws[..n].contains(&b'\t');
    }
    if indentation == 0 {
        return None;
    }
    if at_label {
        let n = if value.ends_with(b"\r\n") { 2 } else { 1 };
        value = &value[..value.len().saturating_sub(n)];
    }
    let newline_at_start = t.pos > 0
        && toks.get(i.wrapping_sub(1)).is_some_and(|p| {
            p.text(src)
                .last()
                .is_some_and(|&c| c == b'\n' || c == b'\r')
        });
    strip_indentation_error(value, indentation, uses_spaces, newline_at_start, at_label)
        .map(|(m, n)| (m, line + n))
}

/// strip_multiline_string_indentation's errors, with the number of newlines
/// before the offending line.
fn strip_indentation_error(
    v: &[u8],
    indentation: usize,
    using_spaces: bool,
    newline_at_start: bool,
    newline_at_end: bool,
) -> Option<(String, u32)> {
    let next_newline = |from: usize| -> Option<(usize, usize)> {
        let k = v[from..].iter().position(|&c| c == b'\r' || c == b'\n')? + from;
        let len = if v[k] == b'\r' && v.get(k + 1) == Some(&b'\n') {
            2
        } else {
            1
        };
        Some((k, len))
    };
    let end = v.len();
    let mut s = 0;
    let mut count = 0;
    let mut nl = Some(0);
    if !newline_at_start {
        let (k, len) = next_newline(0)?;
        s = k + len;
        count += 1;
    }
    while s <= end && nl.is_some() {
        let found = next_newline(s);
        let newline_len = found.map_or(0, |f| f.1);
        nl = found.map(|f| f.0);
        if nl.is_none() && newline_at_end {
            nl = Some(end);
        }
        for _ in 0..indentation {
            if Some(s) == nl {
                break;
            }
            if s == end || (v[s] != b' ' && v[s] != b'\t') {
                return Some((body_indentation(indentation), count));
            }
            if (!using_spaces && v[s] == b' ') || (using_spaces && v[s] == b'\t') {
                return Some((MIXED.into(), count));
            }
            s += 1;
        }
        if s == end {
            break;
        }
        s += match nl {
            Some(k) => k - s + newline_len,
            None => end - s,
        };
        count += 1;
    }
    None
}

/// An undefined token (YYUNDEF): the parser stops at it, as php's parser
/// stops at the T_ERROR its scanner returns after raising an error.
const SCANNER_ERROR: i32 = i32::MAX;

/// The parser-mode errors php's scanner raises on a token it returns.
fn scanner_error(t: &Tok, s: &[u8], nowdoc: bool) -> Option<(String, u32)> {
    let line = t.line as u32;
    match t.id {
        T_DOUBLE_CAST if s.iter().any(|c| c.eq_ignore_ascii_case(&b'r')) => Some((
            "The (real) cast has been removed, use (float) instead".into(),
            line,
        )),
        // an octal literal (a decimal one that overflowed is T_DNUMBER)
        T_LNUMBER | T_DNUMBER
            if s.len() > 1
                && s[0] == b'0'
                && s.iter().all(|&c| c.is_ascii_digit() || c == b'_')
                && s.iter().any(|&c| c == b'8' || c == b'9') =>
        {
            Some(("Invalid numeric literal".into(), line))
        }
        T_CONSTANT_ENCAPSED_STRING | T_ENCAPSED_AND_WHITESPACE => {
            // single-quoted strings and nowdocs have no escapes
            let single = matches!(s.first(), Some(b'\''))
                || (matches!(s.first(), Some(b'b' | b'B')) && s.get(1) == Some(&b'\''));
            if single || (nowdoc && t.id == T_ENCAPSED_AND_WHITESPACE) {
                return None;
            }
            unicode_escape_error(s).map(|(m, nl)| (m, line + nl))
        }
        _ => None,
    }
}

/// zend_scan_escape_string's `\u{…}` errors: the message and the number of
/// newlines before the bad escape.
fn unicode_escape_error(s: &[u8]) -> Option<(String, u32)> {
    let mut i = 0;
    let mut newlines = 0;
    while i < s.len() {
        match s[i] {
            b'\\' if i + 1 < s.len() => {
                if s[i + 1] == b'u' && s.get(i + 2) == Some(&b'{') {
                    let mut j = i + 3;
                    while j < s.len() && s[j].is_ascii_hexdigit() {
                        j += 1;
                    }
                    if s.get(j) != Some(&b'}') || j == i + 3 {
                        return Some(("Invalid UTF-8 codepoint escape sequence".into(), newlines));
                    }
                    let digits = std::str::from_utf8(&s[i + 3..j]).unwrap_or("");
                    let too_large = u64::from_str_radix(digits, 16).map_or(true, |v| v > 0x10FFFF);
                    if too_large {
                        return Some((
                            "Invalid UTF-8 codepoint escape sequence: Codepoint too large".into(),
                            newlines,
                        ));
                    }
                    i = j + 1;
                } else {
                    i += 2;
                }
            }
            b'\n' => {
                newlines += 1;
                i += 1;
            }
            b'\r' => {
                if s.get(i + 1) != Some(&b'\n') {
                    newlines += 1;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

/// Newlines the way HANDLE_NEWLINES counts them (`\n`, lone `\r`).
fn count_newlines(t: &[u8]) -> u32 {
    let mut n = 0;
    for (i, &c) in t.iter().enumerate() {
        if c == b'\n' || (c == b'\r' && t.get(i + 1) != Some(&b'\n')) {
            n += 1;
        }
    }
    n
}

/// Whether the scanner leaves the token's last newline for the next scan
/// (CG(increment_lineno)): `?>` plus a newline, and a heredoc's text before
/// its closing label.
fn defers_newline(src: &[u8], toks: &[Tok], i: usize) -> bool {
    let t = &toks[i];
    let ends_nl = t
        .text(src)
        .last()
        .is_some_and(|&c| c == b'\n' || c == b'\r');
    ends_nl
        && (t.id == T_CLOSE_TAG
            || (t.id == T_ENCAPSED_AND_WHITESPACE
                && toks.get(i + 1).is_some_and(|n| n.id == T_END_HEREDOC)))
}

/// zend_yytnamerr for the unexpected token (not the end of file).
fn unexpected_name(name: &str, text: &[u8]) -> Vec<u8> {
    match name {
        "\"'\\\\'\"" => return b"token \"\\\"".to_vec(),
        "\"amp\"" => return b"token \"&\"".to_vec(),
        "'\"'" => return b"double-quote mark".to_vec(),
        _ => {}
    }
    let mut toktype = name.as_bytes();
    if toktype.len() >= 2 && toktype[0] == b'"' {
        toktype = &toktype[1..toktype.len() - 1];
    }
    if toktype.first() == Some(&b'\'') {
        let mut m = b"token \"".to_vec();
        m.extend_from_slice(&toktype[1..toktype.len() - 1]);
        m.push(b'"');
        return m;
    }
    let mut content = text;
    if content.len() == 1 && name == "\"invalid character\"" {
        return format!("character 0x{:02X}", content[0]).into_bytes();
    }
    if let Some(nl) = content.iter().position(|&c| c == b'\n') {
        content = &content[..nl];
    }
    if !content.is_empty() && name == "\"quoted string\"" {
        if content[0] == b'"' {
            toktype = b"double-quoted string";
        } else if content[0] == b'\'' {
            toktype = b"single-quoted string";
        }
    }
    if matches!(content.first(), Some(b'\'' | b'"')) {
        content = &content[1..];
    }
    if matches!(content.last(), Some(b'\'' | b'"')) {
        content = &content[..content.len() - 1];
    }
    let mut m = toktype.to_vec();
    m.extend_from_slice(b" \"");
    if content.len() > 33 {
        m.extend_from_slice(&content[..30]);
        m.extend_from_slice(b"...");
    } else {
        m.extend_from_slice(content);
    }
    m.push(b'"');
    m
}

/// zend_yytnamerr for an expected token.
fn expected_name(name: &str) -> Vec<u8> {
    match name {
        "\"'\\\\'\"" => return b"\"\\\"".to_vec(),
        "\"amp\"" => return b"token \"&\"".to_vec(),
        _ => {}
    }
    let mut t = name.as_bytes();
    if t.len() >= 2 && t[0] == b'"' {
        t = &t[1..t.len() - 1];
    }
    t.iter()
        .map(|&c| if c == b'\'' { b'"' } else { c })
        .collect()
}
