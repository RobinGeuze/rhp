//! Zend/zend_ini_scanner.l: php's ini scanner (re2c longest-match rules per
//! lexer state), which [`super::ini_parser`] drives for `parse_ini_string`
//! and `parse_ini_file` (ext/standard/basic_functions.rs, with the
//! array-building callbacks).

use rhp_values::Value;

pub(crate) const MODE_NORMAL: i64 = 0;
pub(crate) const MODE_RAW: i64 = 1;
pub(crate) const MODE_TYPED: i64 = 2;

/// A scanner/parser value (`zval` with the `INI_ZVAL_IS_NUMBER` extra flag).
#[derive(Clone, Debug)]
pub(crate) enum IV {
    Str(Vec<u8>, bool),
    Bool(bool),
    Null,
    Long(i64),
    Double(f64),
}

impl IV {
    pub(crate) fn s(b: &[u8]) -> IV {
        IV::Str(b.to_vec(), false)
    }
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        match self {
            IV::Str(s, _) => s,
            IV::Bool(true) => b"1".to_vec(),
            IV::Bool(false) | IV::Null => Vec::new(),
            IV::Long(n) => n.to_string().into_bytes(),
            IV::Double(f) => rhp_values::fmt::float_echo(f),
        }
    }
    pub(crate) fn is_number(&self) -> bool {
        matches!(self, IV::Str(_, true))
    }
    pub(crate) fn into_value(self) -> Value {
        match self {
            IV::Str(s, _) => Value::str(s),
            IV::Bool(b) => Value::Bool(b),
            IV::Null => Value::Null,
            IV::Long(n) => Value::Int(n),
            IV::Double(f) => Value::Float(f),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Tok {
    End,
    Section,
    Raw(IV),
    Constant(IV),
    Number(IV),
    Str(IV),
    Ws(IV),
    Label(IV),
    Offset(IV),
    DollarCurly,
    Varname(IV),
    Quoted(IV),
    Fallback,
    BoolTrue(IV),
    BoolFalse(IV),
    Null(IV),
    Eol,
    Ch(u8),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum St {
    Initial,
    Offset,
    SectionValue,
    Value,
    SectionRaw,
    DoubleQuotes,
    Varname,
    VarFallback,
    Raw,
}

pub(crate) struct Lexer<'a> {
    s: &'a [u8],
    pos: usize,
    state: St,
    stack: Vec<St>,
    pub(crate) mode: i64,
    /// SCNG(lineno), counted where php's scanner counts it.
    pub(crate) lineno: u32,
}

fn is_label_char(c: u8) -> bool {
    !matches!(
        c,
        b'=' | b'\n'
            | b'\r'
            | b'\t'
            | b';'
            | b'&'
            | b'|'
            | b'^'
            | b'$'
            | b'~'
            | b'('
            | b')'
            | b'{'
            | b'}'
            | b'!'
            | b'"'
            | b'['
            | b']'
            | 0
    )
}

fn is_ts(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

impl<'a> Lexer<'a> {
    fn at(&self, i: usize) -> Option<u8> {
        self.s.get(i).copied()
    }

    /// Like `at`, but the C string's terminating NUL is there too: re2c's
    /// DFA reads it like any byte (see `next`).
    fn at0(&self, i: usize) -> Option<u8> {
        self.at(i).or((i == self.s.len()).then_some(0))
    }

    /// `[ \t]*` from `i`; returns the end position.
    fn ts(&self, mut i: usize) -> usize {
        while self.at(i).is_some_and(is_ts) {
            i += 1;
        }
        i
    }

    /// NEWLINE at `i`: returns its length (1 or 2) or 0.
    fn newline(&self, i: usize) -> usize {
        match self.at(i) {
            Some(b'\r') => {
                if self.at(i + 1) == Some(b'\n') {
                    2
                } else {
                    1
                }
            }
            Some(b'\n') => 1,
            _ => 0,
        }
    }

    /// Max end of `(unit)+` starting at `i` (0 = no match), where `unit`
    /// returns the possible unit lengths at a position.
    fn plus(&self, i: usize, unit: &dyn Fn(&Self, usize) -> Vec<usize>) -> usize {
        let mut reach = vec![false; self.s.len() + 2];
        let mut best = 0;
        let mut stack = vec![i];
        while let Some(p) = stack.pop() {
            for l in unit(self, p) {
                let e = p + l;
                if e <= self.s.len() + 1 && !reach[e] {
                    reach[e] = true;
                    best = best.max(e);
                    stack.push(e);
                }
            }
        }
        if best == 0 {
            0
        } else {
            best - i
        }
    }

    /// LITERAL_DOLLAR at `p`: `$` + ([^{\0] | `\` ANY).
    fn literal_dollar(&self, p: usize) -> Vec<usize> {
        let mut v = Vec::new();
        if self.at(p) == Some(b'$') {
            if let Some(n) = self.at(p + 1) {
                if n != b'{' && n != 0 {
                    v.push(2);
                }
                if n == b'\\' && self.at0(p + 2).is_some() {
                    v.push(3);
                }
            }
        }
        v
    }

    fn value_chars(&self, p: usize) -> Vec<usize> {
        match self.at(p) {
            Some(b'$') => self.literal_dollar(p),
            Some(c)
                if !matches!(
                    c,
                    b'=' | b' '
                        | b'\t'
                        | b'\n'
                        | b'\r'
                        | b';'
                        | b'&'
                        | b'|'
                        | b'^'
                        | b'~'
                        | b'('
                        | b')'
                        | b'!'
                        | b'"'
                        | b'\''
                        | 0
                ) =>
            {
                vec![1]
            }
            _ => Vec::new(),
        }
    }

    fn fallback_chars(&self, p: usize) -> Vec<usize> {
        match self.at0(p) {
            Some(b'$') => self.literal_dollar(p),
            Some(b'\\') => {
                if self.at0(p + 1).is_some() {
                    vec![2]
                } else {
                    Vec::new()
                }
            }
            Some(c) if !matches!(c, b'\n' | b'\r' | b';' | b'"' | b'\'' | b'}') => vec![1],
            _ => Vec::new(),
        }
    }

    fn section_value_chars(&self, p: usize) -> Vec<usize> {
        match self.at0(p) {
            Some(b'$') => self.literal_dollar(p),
            Some(b'\\') => {
                if self.at0(p + 1).is_some() {
                    vec![2]
                } else {
                    Vec::new()
                }
            }
            Some(c) if !matches!(c, b'\n' | b'\r' | b';' | b'"' | b'\'' | b']') => vec![1],
            _ => Vec::new(),
        }
    }

    fn label_len(&self, i: usize) -> usize {
        let mut j = i;
        while self.at(j).is_some_and(is_label_char) {
            j += 1;
        }
        j - i
    }

    fn constant_len(&self, i: usize) -> usize {
        match self.at(i) {
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => {
                let mut j = i + 1;
                while self
                    .at(j)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_')
                {
                    j += 1;
                }
                j - i
            }
            _ => 0,
        }
    }

    fn number_len(&self, i: usize) -> usize {
        let digits = |mut j: usize| {
            let st = j;
            while self.at(j).is_some_and(|c| c.is_ascii_digit()) {
                j += 1;
            }
            j - st
        };
        // [-]?LNUM
        let mut a = 0;
        let sign = usize::from(self.at(i) == Some(b'-'));
        let d = digits(i + sign);
        if d > 0 {
            a = sign + d;
        }
        // DNUM: [0-9]*[.][0-9]+ | [0-9]+[.][0-9]*
        let mut b = 0;
        let d1 = digits(i);
        if self.at(i + d1) == Some(b'.') {
            let d2 = digits(i + d1 + 1);
            if d2 > 0 || d1 > 0 {
                b = d1 + 1 + d2;
            }
        }
        a.max(b)
    }

    /// `("true"|"on"|"yes")[ \t]*`-style keyword rule.
    fn keyword_len(&self, i: usize, words: &[&[u8]]) -> usize {
        for w in words {
            // php builds the scanner with `re2c --case-inverted`, so quoted
            // keyword rules match case-insensitively.
            if self.s.len() >= i + w.len() && self.s[i..i + w.len()].eq_ignore_ascii_case(w) {
                return self.ts(i + w.len()) - i;
            }
        }
        0
    }

    /// `[ \t]*[;][^\r\n]*NEWLINE` comment rule.
    fn comment_len(&self, i: usize) -> usize {
        let j = self.ts(i);
        if self.at(j) != Some(b';') {
            return 0;
        }
        let mut k = j + 1;
        while self.at0(k).is_some_and(|c| c != b'\r' && c != b'\n') {
            k += 1;
        }
        if k > self.s.len() {
            return k - i;
        }
        let nl = self.newline(k);
        if nl == 0 {
            return 0;
        }
        k + nl - i
    }

    fn push(&mut self, st: St) {
        self.stack.push(self.state);
        self.state = st;
    }

    fn pop(&mut self) {
        self.state = self.stack.pop().unwrap_or(St::Initial);
    }

    /// RETURN_TOKEN for the bool/null tokens: typed values in TYPED mode while
    /// scanning a value.
    fn bool_tok(&self, kind: u8) -> Tok {
        let typed = self.mode == MODE_TYPED && matches!(self.state, St::Value | St::Raw);
        match kind {
            b't' => Tok::BoolTrue(if typed { IV::Bool(true) } else { IV::s(b"1") }),
            b'f' => Tok::BoolFalse(if typed { IV::Bool(false) } else { IV::s(b"") }),
            _ => Tok::Null(if typed { IV::Null } else { IV::s(b"") }),
        }
    }

    pub(crate) fn next(&mut self) -> Tok {
        loop {
            let i = self.pos;
            if i >= self.s.len() {
                if matches!(self.state, St::Value | St::Raw) {
                    self.state = St::Initial;
                    return Tok::Eol;
                }
                return Tok::End;
            }
            if i == 0 && self.s.len() > 3 && self.s.starts_with(b"\xef\xbb\xbf") {
                self.pos = 3;
                continue;
            }
            // Candidate rules for this state, in file order: (len, rule id).
            let st = self.state;
            let mut cands: Vec<(usize, u8)> = Vec::new();
            let c0 = self.s[i];
            use St::*;
            // R1 "["
            if st == Initial && c0 == b'[' {
                cands.push((1, 1));
            }
            // R2 '...'
            if matches!(st, Value | SectionValue | Offset) && c0 == b'\'' {
                match self.s[i + 1..].iter().position(|&c| c == b'\'') {
                    Some(0) => {}
                    Some(off) => cands.push((off + 2, 2)),
                    None => cands.push((self.s.len() + 1 - i, 2)),
                }
            }
            // R3 "]"[ \t]*NEWLINE?
            if matches!(st, SectionRaw | SectionValue) && c0 == b']' {
                let j = self.ts(i + 1);
                cands.push((j + self.newline(j) - i, 3));
            }
            // R4 LABEL"["[ \t]*
            if st == Initial {
                let l = self.label_len(i);
                if l > 0 && self.at(i + l) == Some(b'[') {
                    cands.push((self.ts(i + l + 1) - i, 4));
                }
            }
            // R5 [ \t]*"]"
            if st == Offset {
                let j = self.ts(i);
                if self.at(j) == Some(b']') {
                    cands.push((j + 1 - i, 5));
                }
            }
            // R6 "${"
            if matches!(
                st,
                DoubleQuotes | SectionValue | Value | Offset | VarFallback
            ) && self.s[i..].starts_with(b"${")
            {
                cands.push((2, 6));
            }
            if st == Varname {
                // R7 ":-"
                if self.s[i..].starts_with(b":-") {
                    cands.push((2, 7));
                }
                // R8 LABEL_CHAR (custom-extended)
                if is_label_char(c0) {
                    cands.push((1, 8));
                }
            }
            // R9 "}"
            if matches!(st, Varname | VarFallback) && c0 == b'}' {
                cands.push((1, 9));
            }
            if matches!(st, Initial | Value) {
                let l = self.keyword_len(i, &[b"true", b"on", b"yes"]);
                if l > 0 {
                    cands.push((l, 10));
                }
                let l = self.keyword_len(i, &[b"false", b"off", b"none", b"no"]);
                if l > 0 {
                    cands.push((l, 11));
                }
                let l = self.keyword_len(i, &[b"null"]);
                if l > 0 {
                    cands.push((l, 12));
                }
            }
            if st == Initial {
                // R13 LABEL
                let l = self.label_len(i);
                if l > 0 {
                    cands.push((l, 13));
                }
                // R14 [ \t]*[=][ \t]*
                let j = self.ts(i);
                if self.at(j) == Some(b'=') {
                    cands.push((self.ts(j + 1) - i, 14));
                }
            }
            // R15 RAW_VALUE_CHARS (custom-extended)
            if st == Raw && !matches!(c0, b'\n' | b'\r' | b';' | 0) {
                cands.push((1, 15));
            }
            // R16 SECTION_RAW_CHARS+
            if st == SectionRaw {
                let mut j = i;
                while self
                    .at0(j)
                    .is_some_and(|c| !matches!(c, b']' | b'\n' | b'\r'))
                {
                    j += 1;
                }
                if j > i {
                    cands.push((j - i, 16));
                }
            }
            // R17 [ \t]*NEWLINE
            if matches!(st, Value | Raw) {
                let j = self.ts(i);
                let nl = self.newline(j);
                if nl > 0 {
                    cands.push((j + nl - i, 17));
                }
            }
            if matches!(st, SectionValue | Value | VarFallback | Offset) {
                let l = self.constant_len(i);
                if l > 0 {
                    cands.push((l, 18));
                }
                let l = self.number_len(i);
                if l > 0 {
                    cands.push((l, 19));
                }
            }
            // R20 TOKENS
            if st == Initial && b":,.[]\"'()&|^+-/*=%$!~<>?@{}".contains(&c0) {
                cands.push((1, 20));
            }
            if st == Value {
                // R21 OPERATORS[ \t]*
                if b"&|^~()!".contains(&c0) {
                    cands.push((self.ts(i + 1) - i, 21));
                }
                // R22 [=]
                if c0 == b'=' {
                    cands.push((1, 22));
                }
                // R23 VALUE_CHARS+
                let l = self.plus(i, &|lx, p| lx.value_chars(p));
                if l > 0 {
                    cands.push((l, 23));
                }
            }
            if st == VarFallback {
                let l = self.plus(i, &|lx, p| lx.fallback_chars(p));
                if l > 0 {
                    cands.push((l, 24));
                }
            }
            if matches!(st, SectionValue | Offset) {
                let l = self.plus(i, &|lx, p| lx.section_value_chars(p));
                if l > 0 {
                    cands.push((l, 25));
                }
            }
            // R26 [ \t]*["]
            if matches!(st, SectionValue | Value | VarFallback | Offset) {
                let j = self.ts(i);
                if self.at(j) == Some(b'"') {
                    cands.push((j + 1 - i, 26));
                }
            }
            if st == DoubleQuotes {
                // R27 ["][ \t]*
                if c0 == b'"' {
                    cands.push((self.ts(i + 1) - i, 27));
                }
                // R28 [^]
                cands.push((1, 28));
            }
            // R29 [ \t]+ (whitespace token)
            if matches!(st, SectionValue | Value | Offset | VarFallback) && is_ts(c0) {
                cands.push((self.ts(i) - i, 29));
            }
            // R30 [ \t]+ (skipped)
            if matches!(st, Initial | Raw) && is_ts(c0) {
                cands.push((self.ts(i) - i, 30));
            }
            // R31 [ \t]*NEWLINE
            if st == Initial {
                let j = self.ts(i);
                let nl = self.newline(j);
                if nl > 0 {
                    cands.push((j + nl - i, 31));
                }
            }
            // R32 comment
            if matches!(st, Initial | Value | Raw) {
                let l = self.comment_len(i);
                if l > 0 {
                    cands.push((l, 32));
                }
            }
            // R33 [^]
            if matches!(st, Value | Raw) {
                cands.push((1, 33));
            }
            // R34 <*>[^]
            cands.push((1, 34));
            // A rule that matched on past the terminating NUL: re2c's DFA
            // read beyond YYLIMIT, and php's YYFILL returns 0 (end of input)
            // without a token. That is an unclosed quote, a comment with no
            // newline, or a trailing `\` that takes the NUL as ANY_CHAR.
            if cands.iter().any(|&(len, _)| i + len > self.s.len()) {
                return Tok::End;
            }

            // Longest match; ties go to the earliest rule.
            let (len, rule) = cands.iter().copied().fold((0usize, 255u8), |best, c| {
                if c.0 > best.0 || (c.0 == best.0 && c.1 < best.1) {
                    c
                } else {
                    best
                }
            });
            let text = &self.s[i..i + len];
            self.pos = i + len;
            match rule {
                1 => {
                    self.state = if self.mode == MODE_RAW {
                        SectionRaw
                    } else {
                        SectionValue
                    };
                    return Tok::Section;
                }
                2 => return Tok::Raw(IV::s(&text[1..text.len() - 1])),
                3 => {
                    self.state = Initial;
                    self.lineno += 1;
                    return Tok::Ch(b']');
                }
                4 => {
                    let mut t = text;
                    while let [b' ' | b'\t', rest @ ..] = t {
                        t = rest;
                    }
                    while let [rest @ .., c] = t {
                        if matches!(c, b'[' | b'\n' | b'\r' | b'\t' | b' ') {
                            t = rest;
                        } else {
                            break;
                        }
                    }
                    self.state = Offset;
                    return Tok::Offset(IV::s(t));
                }
                5 => {
                    self.state = Initial;
                    return Tok::Ch(b']');
                }
                6 => {
                    self.push(Varname);
                    return Tok::DollarCurly;
                }
                7 => {
                    self.pop();
                    self.push(VarFallback);
                    return Tok::Fallback;
                }
                8 => {
                    // php's custom scan (including its single-char ':-' quirk).
                    if self.at(self.pos) == Some(b':') && self.at(self.pos + 1) == Some(b'-') {
                        self.pos += 1;
                        self.pop();
                        self.push(VarFallback);
                        return Tok::Fallback;
                    }
                    while self.pos < self.s.len() {
                        let c = self.s[self.pos];
                        self.pos += 1;
                        let stop = match c {
                            b'=' | b'\n' | b'\r' | b'\t' | b';' | b'&' | b'|' | b'^' | b'$'
                            | b'~' | b'(' | b')' | b'{' | b'}' | b'!' | b'"' | b'[' | b']' => true,
                            b':' => self.at(self.pos) == Some(b'-'),
                            _ => false,
                        };
                        if stop {
                            self.pos -= 1;
                            break;
                        }
                    }
                    let t = trim_ws(&self.s[i..self.pos]);
                    return Tok::Varname(IV::s(t));
                }
                9 => {
                    self.pop();
                    return Tok::Ch(b'}');
                }
                10 => return self.bool_tok(b't'),
                11 => return self.bool_tok(b'f'),
                12 => return self.bool_tok(b'n'),
                13 => return Tok::Label(IV::s(trim_ws(text))),
                14 => {
                    self.state = if self.mode == MODE_RAW { Raw } else { Value };
                    return Tok::Ch(b'=');
                }
                15 => {
                    // Raw value: up to EOL, cutting at an unquoted ';'.
                    let start = {
                        let mut s = i;
                        while self.at(s).is_some_and(is_ts) {
                            s += 1;
                        }
                        s
                    };
                    let first = self.at(start);
                    let mut sc: Option<usize> = None;
                    while self.pos < self.s.len() {
                        match self.s[self.pos] {
                            b'\n' | b'\r' => break,
                            b';' => {
                                if sc.is_none() {
                                    sc = Some(self.pos);
                                }
                                self.pos += 1;
                            }
                            b'"' => {
                                if first == Some(b'"') {
                                    sc = None;
                                }
                                self.pos += 1;
                            }
                            _ => self.pos += 1,
                        }
                    }
                    let end = sc.unwrap_or(self.pos);
                    let mut t = &self.s[start..end.max(start)];
                    while let [rest @ .., b'\n' | b'\r' | b'\t' | b' '] = t {
                        t = rest;
                    }
                    if t.len() > 1 && t[0] == b'"' && t[t.len() - 1] == b'"' {
                        t = &t[1..t.len() - 1];
                    }
                    return Tok::Raw(IV::s(t));
                }
                16 => return Tok::Raw(IV::s(text)),
                17 => {
                    self.state = Initial;
                    self.lineno += 1;
                    return Tok::Eol;
                }
                18 => return Tok::Constant(IV::s(text)),
                19 => return Tok::Number(IV::s(text)),
                20 => return Tok::Ch(c0),
                21 => return Tok::Ch(c0),
                22 => {
                    self.pos = i; // yyless(0)
                    self.state = Initial;
                    return Tok::Eol;
                }
                23..=25 => return Tok::Str(IV::s(text)),
                26 => {
                    self.push(DoubleQuotes);
                    return Tok::Ch(b'"');
                }
                27 => {
                    self.pop();
                    return Tok::Ch(b'"');
                }
                28 => {
                    let lim = self.s.len();
                    let mut s = i;
                    while s < lim {
                        let c = self.s[s];
                        s += 1;
                        match c {
                            b'"' => {}
                            b'$' => {
                                if s < lim && self.s[s] == b'{' {
                                } else {
                                    continue;
                                }
                            }
                            b'\\' => {
                                if s < lim {
                                    let esc = self.s[s];
                                    s += 1;
                                    if esc == b'"'
                                        && (s >= lim || self.s[s] == b'\n' || self.s[s] == b'\r')
                                    {
                                        s -= 1;
                                        break;
                                    }
                                }
                                continue;
                            }
                            _ => continue,
                        }
                        s -= 1;
                        break;
                    }
                    self.pos = s;
                    self.lineno += count_newlines(&self.s[i..s]);
                    return Tok::Quoted(IV::s(&escape_string(&self.s[i..s])));
                }
                29 => return Tok::Ws(IV::s(text)),
                30 => continue,
                31 | 32 => {
                    self.state = Initial;
                    self.lineno += 1;
                    return Tok::Eol;
                }
                33 => {
                    self.state = Initial;
                    return Tok::Eol;
                }
                _ => return Tok::End,
            }
        }
    }
}

/// Newlines as zend_ini_escape_string counts them (`\n`, lone `\r`).
fn count_newlines(t: &[u8]) -> u32 {
    let mut n = 0;
    for (i, &c) in t.iter().enumerate() {
        if c == b'\n' || (c == b'\r' && t.get(i + 1) != Some(&b'\n')) {
            n += 1;
        }
    }
    n
}

pub(crate) fn trim_ws(mut t: &[u8]) -> &[u8] {
    while let [b' ' | b'\t', rest @ ..] = t {
        t = rest;
    }
    while let [rest @ .., b'\n' | b'\r' | b'\t' | b' '] = t {
        t = rest;
    }
    t
}

/// `zend_ini_escape_string` for double-quoted content.
fn escape_string(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'\\' {
            i += 1;
            if i >= s.len() {
                out.push(b'\\');
                continue;
            }
            match s[i] {
                b'"' | b'\\' | b'$' => out.push(s[i]),
                c => {
                    out.push(b'\\');
                    out.push(c);
                }
            }
        } else {
            out.push(s[i]);
        }
        i += 1;
    }
    out
}

/// The scanner sees a C string.
fn c_string(src: &[u8]) -> &[u8] {
    match src.iter().position(|&b| b == 0) {
        Some(p) => &src[..p],
        None => src,
    }
}

pub(crate) fn lexer(src: &[u8], mode: i64) -> Lexer<'_> {
    Lexer {
        s: c_string(src),
        pos: 0,
        state: St::Initial,
        stack: Vec::new(),
        mode,
        lineno: 1,
    }
}
