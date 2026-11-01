//! A port of the Zend scanner (Zend/zend_language_scanner.l) as `token_get_all`
//! sees it: same states (INITIAL, IN_SCRIPTING, LOOKING_FOR_PROPERTY,
//! DOUBLE_QUOTES, BACKQUOTE, HEREDOC, NOWDOC, END_HEREDOC, VAR_OFFSET,
//! LOOKING_FOR_VARNAME) with php's state stack, token ids and token texts.
//! Short open tags are on (php's built-in default).

use super::ids::*;

#[derive(Clone, Debug)]
pub struct Tok {
    /// A `T_*` id (>= 256) or, for a single-character token, the byte itself.
    pub id: i64,
    pub text: Vec<u8>,
    pub line: i64,
    pub pos: usize,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum St {
    Initial,
    Scripting,
    LookingForProperty,
    DoubleQuotes,
    Backquote,
    Heredoc,
    Nowdoc,
    EndHeredoc,
    VarOffset,
    LookingForVarname,
}

struct Label {
    label: Vec<u8>,
    indentation: usize,
}

fn is_label_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}
fn is_label_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}
fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\n' | b'\r' | b'\t')
}

struct Lexer<'a> {
    s: &'a [u8],
    p: usize,
    state: St,
    stack: Vec<St>,
    labels: Vec<Label>,
    line: i64,
    out: Vec<Tok>,
}

/// Newlines in `t` the way HANDLE_NEWLINES counts them (`\n`, lone `\r`).
fn count_newlines(t: &[u8]) -> i64 {
    let mut n = 0;
    for (i, &c) in t.iter().enumerate() {
        if c == b'\n' || (c == b'\r' && t.get(i + 1) != Some(&b'\n')) {
            n += 1;
        }
    }
    n
}

const KEYWORDS: &[(&str, i64)] = &[
    ("exit", T_EXIT),
    ("die", T_EXIT),
    ("fn", T_FN),
    ("function", T_FUNCTION),
    ("const", T_CONST),
    ("return", T_RETURN),
    ("yield", T_YIELD),
    ("try", T_TRY),
    ("catch", T_CATCH),
    ("finally", T_FINALLY),
    ("throw", T_THROW),
    ("if", T_IF),
    ("elseif", T_ELSEIF),
    ("endif", T_ENDIF),
    ("else", T_ELSE),
    ("while", T_WHILE),
    ("endwhile", T_ENDWHILE),
    ("do", T_DO),
    ("for", T_FOR),
    ("endfor", T_ENDFOR),
    ("foreach", T_FOREACH),
    ("endforeach", T_ENDFOREACH),
    ("declare", T_DECLARE),
    ("enddeclare", T_ENDDECLARE),
    ("instanceof", T_INSTANCEOF),
    ("as", T_AS),
    ("switch", T_SWITCH),
    ("match", T_MATCH),
    ("endswitch", T_ENDSWITCH),
    ("case", T_CASE),
    ("default", T_DEFAULT),
    ("break", T_BREAK),
    ("continue", T_CONTINUE),
    ("goto", T_GOTO),
    ("echo", T_ECHO),
    ("print", T_PRINT),
    ("class", T_CLASS),
    ("interface", T_INTERFACE),
    ("trait", T_TRAIT),
    ("extends", T_EXTENDS),
    ("implements", T_IMPLEMENTS),
    ("new", T_NEW),
    ("clone", T_CLONE),
    ("var", T_VAR),
    ("eval", T_EVAL),
    ("include", T_INCLUDE),
    ("include_once", T_INCLUDE_ONCE),
    ("require", T_REQUIRE),
    ("require_once", T_REQUIRE_ONCE),
    ("namespace", T_NAMESPACE),
    ("use", T_USE),
    ("insteadof", T_INSTEADOF),
    ("global", T_GLOBAL),
    ("isset", T_ISSET),
    ("empty", T_EMPTY),
    ("__halt_compiler", T_HALT_COMPILER),
    ("static", T_STATIC),
    ("abstract", T_ABSTRACT),
    ("final", T_FINAL),
    ("private", T_PRIVATE),
    ("protected", T_PROTECTED),
    ("public", T_PUBLIC),
    ("readonly", T_READONLY),
    ("unset", T_UNSET),
    ("list", T_LIST),
    ("array", T_ARRAY),
    ("callable", T_CALLABLE),
    ("or", T_LOGICAL_OR),
    ("and", T_LOGICAL_AND),
    ("xor", T_LOGICAL_XOR),
    ("__class__", T_CLASS_C),
    ("__trait__", T_TRAIT_C),
    ("__function__", T_FUNC_C),
    ("__property__", T_PROPERTY_C),
    ("__method__", T_METHOD_C),
    ("__line__", T_LINE),
    ("__file__", T_FILE),
    ("__dir__", T_DIR),
    ("__namespace__", T_NS_C),
];

pub fn keyword_id(label: &[u8]) -> Option<i64> {
    let l = label.to_ascii_lowercase();
    KEYWORDS
        .iter()
        .find(|(k, _)| k.as_bytes() == l.as_slice())
        .map(|(_, id)| *id)
}

/// Every keyword token id (the "semi-reserved" set TOKEN_PARSE may turn into T_STRING).
pub fn is_keyword_id(id: i64) -> bool {
    KEYWORDS.iter().any(|(_, k)| *k == id)
        || matches!(
            id,
            T_ENUM | T_YIELD_FROM | T_PUBLIC_SET | T_PROTECTED_SET | T_PRIVATE_SET
        )
}

const OPERATORS: &[(&str, i64)] = &[
    ("<<=", T_SL_EQUAL),
    (">>=", T_SR_EQUAL),
    ("**=", T_POW_EQUAL),
    ("...", T_ELLIPSIS),
    ("??=", T_COALESCE_EQUAL),
    ("===", T_IS_IDENTICAL),
    ("!==", T_IS_NOT_IDENTICAL),
    ("<=>", T_SPACESHIP),
    ("?->", T_NULLSAFE_OBJECT_OPERATOR),
    ("++", T_INC),
    ("--", T_DEC),
    ("==", T_IS_EQUAL),
    ("!=", T_IS_NOT_EQUAL),
    ("<>", T_IS_NOT_EQUAL),
    ("<=", T_IS_SMALLER_OR_EQUAL),
    (">=", T_IS_GREATER_OR_EQUAL),
    ("+=", T_PLUS_EQUAL),
    ("-=", T_MINUS_EQUAL),
    ("*=", T_MUL_EQUAL),
    ("**", T_POW),
    ("/=", T_DIV_EQUAL),
    (".=", T_CONCAT_EQUAL),
    ("%=", T_MOD_EQUAL),
    ("&=", T_AND_EQUAL),
    ("|=", T_OR_EQUAL),
    ("^=", T_XOR_EQUAL),
    ("|>", T_PIPE),
    ("||", T_BOOLEAN_OR),
    ("&&", T_BOOLEAN_AND),
    ("<<", T_SL),
    (">>", T_SR),
    ("->", T_OBJECT_OPERATOR),
    ("=>", T_DOUBLE_ARROW),
    ("::", T_PAAMAYIM_NEKUDOTAYIM),
    ("??", T_COALESCE),
];

const CASTS: &[(&str, i64)] = &[
    ("int", T_INT_CAST),
    ("integer", T_INT_CAST),
    ("float", T_DOUBLE_CAST),
    ("double", T_DOUBLE_CAST),
    ("real", T_DOUBLE_CAST),
    ("string", T_STRING_CAST),
    ("binary", T_STRING_CAST),
    ("array", T_ARRAY_CAST),
    ("object", T_OBJECT_CAST),
    ("bool", T_BOOL_CAST),
    ("boolean", T_BOOL_CAST),
    ("unset", T_UNSET_CAST),
    ("void", T_VOID_CAST),
];

impl<'a> Lexer<'a> {
    fn at(&self, i: usize) -> u8 {
        self.s.get(i).copied().unwrap_or(0)
    }
    fn starts_ci(&self, i: usize, kw: &[u8]) -> bool {
        self.s.len() >= i + kw.len() && self.s[i..i + kw.len()].eq_ignore_ascii_case(kw)
    }
    fn push(&mut self, id: i64, end: usize) {
        let text = self.s[self.p..end].to_vec();
        let line = self.line;
        self.line += count_newlines(&text);
        self.out.push(Tok {
            id,
            text,
            line,
            pos: self.p,
        });
        self.p = end;
    }
    fn push_state(&mut self, st: St) {
        self.stack.push(self.state);
        self.state = st;
    }
    fn pop_state(&mut self) {
        if let Some(s) = self.stack.pop() {
            self.state = s;
        }
    }

    fn label_end(&self, mut i: usize) -> usize {
        while i < self.s.len() && is_label_char(self.s[i]) {
            i += 1;
        }
        i
    }

    /// Length of a run of whitespace / comments starting at `i` (WHITESPACE_OR_COMMENTS).
    fn ws_or_comments(&self, mut i: usize) -> usize {
        let start = i;
        loop {
            let c = self.at(i);
            if i < self.s.len() && is_ws(c) {
                i += 1;
            } else if c == b'/' && self.at(i + 1) == b'*' {
                match find(&self.s[i + 2..], b"*/") {
                    Some(k) => i = i + 2 + k + 2,
                    None => return i - start,
                }
            } else if (c == b'/' && self.at(i + 1) == b'/') || (c == b'#' && self.at(i + 1) != b'[')
            {
                let mut j = i + if c == b'#' { 1 } else { 2 };
                while j < self.s.len() && !matches!(self.s[j], b'\n' | b'\r') {
                    j += 1;
                }
                if j >= self.s.len() {
                    return i - start; // needs a terminating newline
                }
                i = j + 1;
            } else {
                return i - start;
            }
        }
    }

    fn initial(&mut self) {
        let p = self.p;
        if self.s[p..].starts_with(b"<?=") {
            self.state = St::Scripting;
            return self.push(T_OPEN_TAG_WITH_ECHO, p + 3);
        }
        if self.starts_ci(p, b"<?php") {
            let c = self.at(p + 5);
            if p + 5 == self.s.len() {
                self.state = St::Scripting;
                return self.push(T_OPEN_TAG, p + 5);
            }
            if c == b' ' || c == b'\t' || c == b'\n' {
                self.state = St::Scripting;
                return self.push(T_OPEN_TAG, p + 6);
            }
            if c == b'\r' {
                let e = if self.at(p + 6) == b'\n' {
                    p + 7
                } else {
                    p + 6
                };
                self.state = St::Scripting;
                return self.push(T_OPEN_TAG, e);
            }
            // <?phpX → <? then phpX (short tags)
            self.state = St::Scripting;
            return self.push(T_OPEN_TAG, p + 2);
        }
        if self.s[p..].starts_with(b"<?") {
            self.state = St::Scripting;
            return self.push(T_OPEN_TAG, p + 2);
        }
        // inline HTML up to the next "<?"
        let mut i = p + 1;
        loop {
            match self.s[i.min(self.s.len())..]
                .iter()
                .position(|&c| c == b'<')
            {
                None => {
                    i = self.s.len();
                    break;
                }
                Some(k) => {
                    i += k;
                    if self.at(i + 1) == b'?' {
                        break;
                    }
                    i += 1;
                }
            }
        }
        self.push(T_INLINE_HTML, i)
    }

    fn scripting(&mut self) {
        let s = self.s;
        let p = self.p;
        let c = s[p];
        // whitespace
        if is_ws(c) {
            let mut e = p;
            while e < s.len() && is_ws(s[e]) {
                e += 1;
            }
            return self.push(T_WHITESPACE, e);
        }
        // close tag
        if c == b'?' && self.at(p + 1) == b'>' {
            let mut e = p + 2;
            if self.at(e) == b'\n' {
                e += 1;
            } else if self.at(e) == b'\r' {
                e += 1;
                if self.at(e) == b'\n' {
                    e += 1;
                }
            }
            self.state = St::Initial;
            return self.push(T_CLOSE_TAG, e);
        }
        // comments
        if c == b'#' && self.at(p + 1) == b'[' {
            return self.push(T_ATTRIBUTE, p + 2);
        }
        if c == b'#' || (c == b'/' && self.at(p + 1) == b'/') {
            return self.line_comment();
        }
        if c == b'/' && self.at(p + 1) == b'*' {
            return self.block_comment();
        }
        // variables
        if c == b'$' && is_label_start(self.at(p + 1)) {
            let e = self.label_end(p + 1);
            return self.push(T_VARIABLE, e);
        }
        // strings
        if c == b'\'' || ((c == b'b' || c == b'B') && self.at(p + 1) == b'\'') {
            return self.single_quoted();
        }
        if c == b'"' || ((c == b'b' || c == b'B') && self.at(p + 1) == b'"') {
            return self.double_quoted();
        }
        if (c == b'<' || ((c == b'b' || c == b'B') && self.at(p + 1) == b'<'))
            && self.try_heredoc_start()
        {
            return;
        }
        if c == b'`' {
            self.state = St::Backquote;
            return self.push(b'`' as i64, p + 1);
        }
        // numbers
        if c.is_ascii_digit() || (c == b'.' && self.at(p + 1).is_ascii_digit()) {
            return self.number();
        }
        // names / keywords
        if c == b'\\' {
            if is_label_start(self.at(p + 1)) {
                let e = self.qualified_end(p + 1);
                return self.push(T_NAME_FULLY_QUALIFIED, e);
            }
            return self.push(T_NS_SEPARATOR, p + 1);
        }
        if is_label_start(c) {
            return self.label();
        }
        // casts
        if c == b'(' {
            if let Some((id, e)) = self.cast(p) {
                return self.push(id, e);
            }
        }
        // `&` followed by a variable or `...`
        if c == b'&' && !matches!(self.at(p + 1), b'&' | b'=') {
            let w = self.ws_or_comments(p + 1);
            let n = p + 1 + w;
            let id = if (self.at(n) == b'$') || s[n.min(s.len())..].starts_with(b"...") {
                T_AMPERSAND_FOLLOWED_BY_VAR_OR_VARARG
            } else {
                T_AMPERSAND_NOT_FOLLOWED_BY_VAR_OR_VARARG
            };
            return self.push(id, p + 1);
        }
        for (op, id) in OPERATORS {
            if s[p..].starts_with(op.as_bytes()) {
                let id = *id;
                if id == T_OBJECT_OPERATOR || id == T_NULLSAFE_OBJECT_OPERATOR {
                    self.push_state(St::LookingForProperty);
                }
                return self.push(id, p + op.len());
            }
        }
        match c {
            b'{' => {
                self.push_state(St::Scripting);
                self.push(b'{' as i64, p + 1)
            }
            b'}' => {
                if !self.stack.is_empty() {
                    self.pop_state();
                }
                self.push(b'}' as i64, p + 1)
            }
            b';' | b':' | b',' | b'.' | b'|' | b'^' | b'&' | b'+' | b'-' | b'/' | b'*' | b'='
            | b'%' | b'!' | b'~' | b'$' | b'<' | b'>' | b'?' | b'@' | b'[' | b']' | b'(' | b')'
            | b'"' | b'\'' | b'`' => self.push(c as i64, p + 1),
            _ => self.push(T_BAD_CHARACTER, p + 1),
        }
    }

    fn cast(&self, p: usize) -> Option<(i64, usize)> {
        let mut i = p + 1;
        while matches!(self.at(i), b' ' | b'\t') {
            i += 1;
        }
        let start = i;
        while self.at(i).is_ascii_alphabetic() {
            i += 1;
        }
        let word = self.s[start..i].to_ascii_lowercase();
        let (_, id) = CASTS
            .iter()
            .find(|(w, _)| w.as_bytes() == word.as_slice())?;
        while matches!(self.at(i), b' ' | b'\t') {
            i += 1;
        }
        if self.at(i) == b')' {
            Some((*id, i + 1))
        } else {
            None
        }
    }

    fn qualified_end(&self, mut i: usize) -> usize {
        // LABEL ("\" LABEL)*
        i = self.label_end(i);
        while self.at(i) == b'\\' && is_label_start(self.at(i + 1)) {
            i = self.label_end(i + 1);
        }
        i
    }

    fn label(&mut self) {
        let p = self.p;
        let e = self.label_end(p);
        let word = &self.s[p..e];
        // qualified names
        if self.at(e) == b'\\' && is_label_start(self.at(e + 1)) {
            let qe = self.qualified_end(p);
            let id = if word.eq_ignore_ascii_case(b"namespace") {
                T_NAME_RELATIVE
            } else {
                T_NAME_QUALIFIED
            };
            return self.push(id, qe);
        }
        let lower = word.to_ascii_lowercase();
        match lower.as_slice() {
            b"yield" => {
                let w = self.ws_or_comments(e);
                // `"from"[^a-zA-Z0-9_\x80-\xff]`: at end of input the class
                // matches re2c's NUL sentinel.
                if w > 0
                    && self.starts_ci(e + w, b"from")
                    && (e + w + 4 >= self.s.len() || !is_label_char(self.s[e + w + 4]))
                {
                    return self.push(T_YIELD_FROM, e + w + 4);
                }
            }
            b"enum" => {
                let w = self.ws_or_comments(e);
                if w > 0 {
                    let n = e + w;
                    // re2c matches "extends"/"implements" as a literal prefix here
                    let is_ext = self.starts_ci(n, b"extends") || self.starts_ci(n, b"implements");
                    if is_ext {
                        return self.push(T_STRING, e);
                    }
                    if is_label_start(self.at(n)) {
                        return self.push(T_ENUM, e);
                    }
                }
                return self.push(T_STRING, e);
            }
            b"public" | b"protected" | b"private" if self.starts_ci(e, b"(set)") => {
                let id = match lower.as_slice() {
                    b"public" => T_PUBLIC_SET,
                    b"protected" => T_PROTECTED_SET,
                    _ => T_PRIVATE_SET,
                };
                return self.push(id, e + 5);
            }
            _ => {}
        }
        let id = keyword_id(word).unwrap_or(T_STRING);
        self.push(id, e)
    }

    fn line_comment(&mut self) {
        let mut i = self.p + if self.s[self.p] == b'#' { 1 } else { 2 };
        while i < self.s.len() {
            match self.s[i] {
                b'\r' | b'\n' => break,
                b'?' if self.at(i + 1) == b'>' => break,
                _ => i += 1,
            }
        }
        self.push(T_COMMENT, i)
    }

    fn block_comment(&mut self) {
        let p = self.p;
        let doc = self.s[p..].starts_with(b"/**") && is_ws(self.at(p + 3)) && p + 3 < self.s.len();
        let start = if doc { p + 4 } else { p + 2 };
        // php scans from after "/*" (or "/**" + one whitespace char)
        let e = match find(&self.s[start..], b"*/") {
            Some(k) => start + k + 2,
            None => self.s.len(),
        };
        self.push(if doc { T_DOC_COMMENT } else { T_COMMENT }, e)
    }

    fn single_quoted(&mut self) {
        let mut i = self.p + if self.s[self.p] == b'\'' { 1 } else { 2 };
        while i < self.s.len() {
            match self.s[i] {
                b'\'' => return self.push(T_CONSTANT_ENCAPSED_STRING, i + 1),
                b'\\' if i + 1 < self.s.len() => i += 2,
                _ => i += 1,
            }
        }
        self.push(T_ENCAPSED_AND_WHITESPACE, self.s.len())
    }

    fn double_quoted(&mut self) {
        let qstart = self.p + if self.s[self.p] == b'"' { 1 } else { 2 };
        let mut i = qstart;
        while i < self.s.len() {
            let c = self.s[i];
            i += 1;
            match c {
                b'"' => return self.push(T_CONSTANT_ENCAPSED_STRING, i),
                b'$' if is_label_start(self.at(i)) || self.at(i) == b'{' => break,
                b'{' if self.at(i) == b'$' => break,
                b'\\' if i < self.s.len() => i += 1,
                _ => {}
            }
        }
        self.state = St::DoubleQuotes;
        self.push(b'"' as i64, qstart)
    }

    fn try_heredoc_start(&mut self) -> bool {
        let p = self.p;
        let mut i = p;
        if matches!(self.s[i], b'b' | b'B') {
            i += 1;
        }
        if !self.s[i..].starts_with(b"<<<") {
            return false;
        }
        i += 3;
        while matches!(self.at(i), b' ' | b'\t') {
            i += 1;
        }
        let quote = match self.at(i) {
            q @ (b'\'' | b'"') => {
                i += 1;
                Some(q)
            }
            _ => None,
        };
        if !is_label_start(self.at(i)) {
            return false;
        }
        let ls = i;
        let le = self.label_end(i);
        i = le;
        if let Some(q) = quote {
            if self.at(i) != q {
                return false;
            }
            i += 1;
        }
        // NEWLINE
        match self.at(i) {
            b'\n' => i += 1,
            b'\r' => {
                i += 1;
                if self.at(i) == b'\n' {
                    i += 1;
                }
            }
            _ => return false,
        }
        let label = self.s[ls..le].to_vec();
        let nowdoc = quote == Some(b'\'');
        // closing label right away?
        let mut j = i;
        let mut ind = 0;
        while matches!(self.at(j), b' ' | b'\t') {
            j += 1;
            ind += 1;
        }
        let immediate = j < self.s.len()
            && self.s[j..].starts_with(&label)
            && label.len() < self.s.len() - j
            && !is_label_char(self.at(j + label.len()));
        self.labels.push(Label {
            label,
            indentation: if immediate { ind } else { 0 },
        });
        self.state = if immediate {
            St::EndHeredoc
        } else if nowdoc {
            St::Nowdoc
        } else {
            St::Heredoc
        };
        self.push(T_START_HEREDOC, i);
        true
    }

    fn end_heredoc(&mut self) {
        let l = self.labels.pop();
        let n = l.map(|l| l.indentation + l.label.len()).unwrap_or(1);
        self.state = St::Scripting;
        let e = (self.p + n).min(self.s.len());
        self.push(T_END_HEREDOC, e)
    }

    /// Variable forms shared by "…", `…` and heredoc bodies. Returns true if a
    /// token was produced.
    fn encaps_var(&mut self) -> bool {
        let p = self.p;
        let c = self.s[p];
        if c == b'$' && is_label_start(self.at(p + 1)) {
            let e = self.label_end(p + 1);
            if (self.s[e..].starts_with(b"->") && is_label_start(self.at(e + 2)))
                || (self.s[e..].starts_with(b"?->") && is_label_start(self.at(e + 3)))
            {
                self.push_state(St::LookingForProperty);
            } else if self.at(e) == b'[' {
                self.push_state(St::VarOffset);
            }
            self.push(T_VARIABLE, e);
            return true;
        }
        if c == b'$' && self.at(p + 1) == b'{' {
            self.push_state(St::LookingForVarname);
            self.push(T_DOLLAR_OPEN_CURLY_BRACES, p + 2);
            return true;
        }
        if c == b'{' && self.at(p + 1) == b'$' {
            self.push_state(St::Scripting);
            self.push(T_CURLY_OPEN, p + 1);
            return true;
        }
        false
    }

    fn encaps(&mut self, quote: u8) {
        let p = self.p;
        if self.s[p] == quote {
            self.state = St::Scripting;
            return self.push(quote as i64, p + 1);
        }
        if self.encaps_var() {
            return;
        }
        let mut i = p;
        if self.s[i] == b'\\' && i + 1 < self.s.len() {
            i += 1;
        }
        i += 1;
        while i < self.s.len() {
            let c = self.s[i];
            i += 1;
            match c {
                c if c == quote => {
                    i -= 1;
                    break;
                }
                b'$' if is_label_start(self.at(i)) || self.at(i) == b'{' => {
                    i -= 1;
                    break;
                }
                b'{' if self.at(i) == b'$' => {
                    i -= 1;
                    break;
                }
                b'\\' if i < self.s.len() => i += 1,
                _ => {}
            }
        }
        self.push(T_ENCAPSED_AND_WHITESPACE, i.min(self.s.len()))
    }

    /// At `i` (just after a newline): is this the closing label line? Returns
    /// the indentation.
    fn closing_label_at(&self, i: usize) -> Option<usize> {
        let label = &self.labels.last()?.label;
        let mut j = i;
        while matches!(self.at(j), b' ' | b'\t') {
            j += 1;
        }
        if j < self.s.len()
            && is_label_start(self.s[j])
            && label.len() < self.s.len() - j
            && self.s[j..].starts_with(label)
            && !is_label_char(self.at(j + label.len()))
        {
            Some(j - i)
        } else {
            None
        }
    }

    fn heredoc(&mut self) {
        if self.encaps_var() {
            return;
        }
        let p = self.p;
        let mut i = p;
        while i < self.s.len() {
            let c = self.s[i];
            i += 1;
            match c {
                b'\r' | b'\n' => {
                    if c == b'\r' && self.at(i) == b'\n' {
                        i += 1;
                    }
                    if let Some(ind) = self.closing_label_at(i) {
                        if let Some(l) = self.labels.last_mut() {
                            l.indentation = ind;
                        }
                        self.state = St::EndHeredoc;
                        return self.push(T_ENCAPSED_AND_WHITESPACE, i);
                    }
                }
                b'$' if (is_label_start(self.at(i)) || self.at(i) == b'{') && i - 1 > p => {
                    i -= 1;
                    break;
                }
                b'{' if self.at(i) == b'$' && i - 1 > p => {
                    i -= 1;
                    break;
                }
                b'\\' if i < self.s.len() && !matches!(self.s[i], b'\n' | b'\r') => i += 1,
                _ => {}
            }
        }
        self.push(T_ENCAPSED_AND_WHITESPACE, i.min(self.s.len()))
    }

    fn nowdoc(&mut self) {
        let p = self.p;
        let mut i = p;
        while i < self.s.len() {
            let c = self.s[i];
            i += 1;
            if c == b'\r' || c == b'\n' {
                if c == b'\r' && self.at(i) == b'\n' {
                    i += 1;
                }
                if let Some(ind) = self.closing_label_at(i) {
                    if let Some(l) = self.labels.last_mut() {
                        l.indentation = ind;
                    }
                    self.state = St::EndHeredoc;
                    return self.push(T_ENCAPSED_AND_WHITESPACE, i);
                }
            }
        }
        self.push(T_ENCAPSED_AND_WHITESPACE, self.s.len())
    }

    fn looking_for_property(&mut self) {
        let p = self.p;
        let c = self.s[p];
        if is_ws(c) {
            let mut e = p;
            while e < self.s.len() && is_ws(self.s[e]) {
                e += 1;
            }
            return self.push(T_WHITESPACE, e);
        }
        if self.s[p..].starts_with(b"->") {
            return self.push(T_OBJECT_OPERATOR, p + 2);
        }
        if self.s[p..].starts_with(b"?->") {
            return self.push(T_NULLSAFE_OBJECT_OPERATOR, p + 3);
        }
        if is_label_start(c) {
            let e = self.label_end(p);
            self.pop_state();
            return self.push(T_STRING, e);
        }
        if c == b'#' || (c == b'/' && self.at(p + 1) == b'/') {
            return self.line_comment();
        }
        if c == b'/' && self.at(p + 1) == b'*' {
            return self.block_comment();
        }
        self.pop_state();
    }

    fn looking_for_varname(&mut self) {
        let p = self.p;
        if is_label_start(self.s[p]) {
            let e = self.label_end(p);
            if matches!(self.at(e), b'[' | b'}') {
                self.pop_state();
                self.push_state(St::Scripting);
                return self.push(T_STRING_VARNAME, e);
            }
        }
        self.pop_state();
        self.push_state(St::Scripting);
    }

    fn var_offset(&mut self) {
        let p = self.p;
        let c = self.s[p];
        if c.is_ascii_digit() {
            let e = self.num_end(p);
            return self.push(T_NUM_STRING, e);
        }
        if c == b'$' && is_label_start(self.at(p + 1)) {
            let e = self.label_end(p + 1);
            return self.push(T_VARIABLE, e);
        }
        if c == b']' {
            self.pop_state();
            return self.push(b']' as i64, p + 1);
        }
        if b";:,.|^&+-/*=%!~$<>?@[(){}\"`".contains(&c) {
            return self.push(c as i64, p + 1);
        }
        if matches!(c, b' ' | b'\n' | b'\r' | b'\t' | b'\\' | b'\'' | b'#') {
            self.pop_state();
            return self.push(T_ENCAPSED_AND_WHITESPACE, p);
        }
        if is_label_start(c) {
            let e = self.label_end(p);
            return self.push(T_STRING, e);
        }
        self.push(T_BAD_CHARACTER, p + 1)
    }

    /// End of an integer literal ({LNUM}|{HNUM}|{BNUM}|{ONUM}) starting at `p`.
    fn num_end(&self, p: usize) -> usize {
        let s = self.s;
        let digits = |mut i: usize, ok: &dyn Fn(u8) -> bool| -> usize {
            // D+(_D+)*
            if !ok(self.at(i)) {
                return i;
            }
            while i < s.len() {
                if ok(s[i]) || (s[i] == b'_' && ok(self.at(i + 1))) {
                    i += 1;
                } else {
                    break;
                }
            }
            i
        };
        if self.at(p) == b'0' {
            let x = self.at(p + 1) | 0x20;
            let ok: Option<&dyn Fn(u8) -> bool> = match x {
                b'x' => Some(&|c: u8| c.is_ascii_hexdigit()),
                b'b' => Some(&|c: u8| c == b'0' || c == b'1'),
                b'o' => Some(&|c: u8| (b'0'..=b'7').contains(&c)),
                _ => None,
            };
            if let Some(ok) = ok {
                let e = digits(p + 2, ok);
                if e > p + 2 {
                    return e;
                }
            }
        }
        digits(p, &|c: u8| c.is_ascii_digit())
    }

    fn number(&mut self) {
        let s = self.s;
        let p = self.p;
        let isd = |c: u8| c.is_ascii_digit();
        let lnum = |i: usize| -> usize {
            let mut i = i;
            if !isd(self.at(i)) {
                return i;
            }
            while i < s.len() {
                if isd(s[i]) || (s[i] == b'_' && isd(self.at(i + 1))) {
                    i += 1;
                } else {
                    break;
                }
            }
            i
        };
        // hex / binary / octal prefixes
        if self.at(p) == b'0' && matches!(self.at(p + 1) | 0x20, b'x' | b'b' | b'o') {
            let e = self.num_end(p);
            if e > p + 1 && e != lnum(p) {
                let digits: Vec<u8> = s[p + 2..e].iter().copied().filter(|&c| c != b'_').collect();
                let sig: &[u8] = {
                    let z = digits
                        .iter()
                        .position(|&c| c != b'0')
                        .unwrap_or(digits.len());
                    &digits[z..]
                };
                let fits = match self.at(p + 1) | 0x20 {
                    b'x' => sig.len() < 16 || (sig.len() == 16 && sig[0] <= b'7'),
                    b'b' => sig.len() < 64,
                    _ => {
                        // octal: value must fit in i64
                        let mut v: u128 = 0;
                        let mut ok = true;
                        for &c in sig {
                            v = v * 8 + (c - b'0') as u128;
                            if v > i64::MAX as u128 {
                                ok = false;
                                break;
                            }
                        }
                        ok
                    }
                };
                return self.push(if fits { T_LNUMBER } else { T_DNUMBER }, e);
            }
        }
        // decimal: LNUM, DNUM, EXPONENT_DNUM
        let mut e = lnum(p);
        let mut is_float = false;
        if self.at(e) == b'.' {
            let f = lnum(e + 1);
            if f > e + 1 || e > p {
                is_float = true;
                e = f;
            }
        }
        if e > p && matches!(self.at(e), b'e' | b'E') {
            let mut k = e + 1;
            if matches!(self.at(k), b'+' | b'-') {
                k += 1;
            }
            let x = lnum(k);
            if x > k {
                is_float = true;
                e = x;
            }
        }
        if is_float {
            return self.push(T_DNUMBER, e);
        }
        let digits: Vec<u8> = s[p..e].iter().copied().filter(|&c| c != b'_').collect();
        let fits = if digits.len() > 1 && digits[0] == b'0' {
            // octal (strtol base 0 stops at a non-octal digit)
            let mut v: u128 = 0;
            let mut ok = true;
            for &c in &digits[1..] {
                if !(b'0'..=b'7').contains(&c) {
                    break;
                }
                v = v * 8 + (c - b'0') as u128;
                if v > i64::MAX as u128 {
                    ok = false;
                    break;
                }
            }
            ok
        } else {
            let mut v: u128 = 0;
            let mut ok = true;
            for &c in &digits {
                v = v * 10 + (c - b'0') as u128;
                if v > i64::MAX as u128 {
                    ok = false;
                    break;
                }
            }
            ok
        };
        self.push(if fits { T_LNUMBER } else { T_DNUMBER }, e)
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Tokenize `src` exactly as `token_get_all($src)` does (without TOKEN_PARSE).
pub fn tokenize(src: &[u8]) -> Vec<Tok> {
    let mut lx = Lexer {
        s: src,
        p: 0,
        state: St::Initial,
        stack: Vec::new(),
        labels: Vec::new(),
        line: 1,
        out: Vec::new(),
    };
    lx.run_with_halt();
    lx.out
}

impl Lexer<'_> {
    /// `run`, plus ext/tokenizer's `__halt_compiler` handling: after
    /// T_HALT_COMPILER, three more significant tokens, then the rest of the
    /// input as one T_INLINE_HTML.
    fn run_with_halt(&mut self) {
        let mut need: i32 = -1;
        while self.p < self.s.len() {
            let before = self.out.len();
            match self.state {
                St::Initial => self.initial(),
                St::Scripting => self.scripting(),
                St::LookingForProperty => self.looking_for_property(),
                St::DoubleQuotes => self.encaps(b'"'),
                St::Backquote => self.encaps(b'`'),
                St::Heredoc => self.heredoc(),
                St::Nowdoc => self.nowdoc(),
                St::EndHeredoc => self.end_heredoc(),
                St::VarOffset => self.var_offset(),
                St::LookingForVarname => self.looking_for_varname(),
            }
            if self.out.len() == before {
                continue;
            }
            let id = self.out.last().map(|t| t.id).unwrap_or(0);
            if need != -1 {
                if !matches!(id, T_WHITESPACE | T_OPEN_TAG | T_COMMENT | T_DOC_COMMENT) {
                    need -= 1;
                    if need == 0 {
                        if self.p < self.s.len() {
                            // A closing `?>\n` defers its line increment
                            // (CG(increment_lineno)) to the next scan, which
                            // never happens: the rest keeps the tag's line.
                            if id == T_CLOSE_TAG {
                                if let Some(t) = self.out.last() {
                                    self.line = t.line;
                                }
                            }
                            let e = self.s.len();
                            self.push(T_INLINE_HTML, e);
                        }
                        return;
                    }
                }
            } else if id == T_HALT_COMPILER {
                need = 3;
            }
        }
    }
}
