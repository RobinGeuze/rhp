//! php's parse errors, exactly as php reports them: rhp's port of the Zend
//! scanner feeds php's own LALR automaton (Zend/zend_language_parser.y, Bison
//! tables from the release tarball), with the scanner's parser-mode errors
//! (bracket nesting, `(real)`, octal literals, unterminated comments, `\u{}`
//! escapes, heredoc indentation) raised when php's scanner raises them, and
//! the grammar actions that refuse code (modifier lists, a nested
//! `__halt_compiler()`, `<?=` as an identifier) run as php runs them.

use super::bison::{self, Outcome};
use super::language_parser_tables::{rule, TABLES};
use crate::ext::tokenizer::ids::*;
use crate::ext::tokenizer::lexer::{self, Tok};

/// Why php refuses to compile a file: the exception it throws (a
/// `ParseError`, or a `CompileError` from a grammar action), with php's
/// message and line. Line 0: php raises it outside the file (the includer's
/// line for include/eval, no file at all for the main script).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFailure {
    pub class: &'static str,
    pub message: String,
    pub line: u32,
}

thread_local! {
    /// The E_COMPILE_WARNINGs the grammar actions raised (message, line).
    static PARSE_WARNINGS: std::cell::RefCell<Vec<(String, u32)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The E_COMPILE_WARNINGs (message, line) the last [`syntax_error`] run
/// raised: zend_handle_encoding_declaration's, which php raises while
/// parsing, before any compile diagnostic.
pub fn take_parse_warnings() -> Vec<(String, u32)> {
    PARSE_WARNINGS.with(|w| w.take())
}

/// zend_multibyte_fetch_encoding: mbstring's mbfl_name2encoding (none
/// without mbstring).
fn encoding_known(name: &[u8]) -> bool {
    #[cfg(feature = "mbstring-libmbfl")]
    return rhp_mbfl_sys::Encoding::by_name(name).is_some();
    #[cfg(not(feature = "mbstring-libmbfl"))]
    return {
        let _ = name;
        false
    };
}

/// `zval_get_string` of a literal declare value (its first token): a number
/// as php prints it, a quoted string without its quotes.
fn literal_string(src: &[u8], t: Option<&Tok>) -> Vec<u8> {
    let Some(t) = t else {
        return Vec::new();
    };
    let text = t.text(src);
    match t.id {
        T_LNUMBER | T_DNUMBER => {
            let digits: Vec<u8> = text.iter().copied().filter(|&c| c != b'_').collect();
            let d = String::from_utf8_lossy(&digits).to_ascii_lowercase();
            let int = if t.id == T_DNUMBER {
                None
            } else if let Some(h) = d.strip_prefix("0x") {
                i64::from_str_radix(h, 16).ok()
            } else if let Some(b) = d.strip_prefix("0b") {
                i64::from_str_radix(b, 2).ok()
            } else if let Some(o) = d.strip_prefix("0o") {
                i64::from_str_radix(o, 8).ok()
            } else if d.len() > 1 && d.starts_with('0') {
                i64::from_str_radix(&d[1..], 8).ok()
            } else {
                d.parse().ok()
            };
            let v = match int {
                Some(i) => rhp_values::Value::Int(i),
                None => rhp_values::Value::Float(rhp_values::conv::to_float(
                    &rhp_values::Value::str(digits.clone()),
                )),
            };
            rhp_values::conv::to_string_bytes(&v)
        }
        T_CONSTANT_ENCAPSED_STRING if text.len() >= 2 => {
            let inner = &text[1..text.len() - 1];
            if text[0] != b'\'' {
                return inner.to_vec();
            }
            let mut out = Vec::with_capacity(inner.len());
            let mut i = 0;
            while i < inner.len() {
                if inner[i] == b'\\' && matches!(inner.get(i + 1), Some(b'\'' | b'\\')) {
                    i += 1;
                }
                out.push(inner[i]);
                i += 1;
            }
            out
        }
        _ => text.to_vec(),
    }
}

const PARSE_ERROR: &str = "ParseError";
const COMPILE_ERROR: &str = "CompileError";

/// The first error php raises parsing `src`: a syntax error (`syntax error,
/// unexpected …, expecting …`), a scanner error or a grammar action's error.
/// `None` when php's parser accepts the file.
pub fn syntax_error(src: &[u8]) -> Option<ParseFailure> {
    let toks = lexer::tokenize(src);
    PARSE_WARNINGS.with(|w| w.borrow_mut().clear());
    let mut feed = Feed {
        src,
        toks: &toks,
        i: 0,
        nesting: Vec::new(),
        stopped: false,
        last: None,
        strings: Vec::new(),
        modifier_lists: Vec::new(),
        declares: Vec::new(),
        error: None,
        final_line: 1 + count_newlines(src),
    };
    let outcome = TABLES.run(&mut feed);
    if let Some(e) = feed.error.take() {
        return Some(e);
    }
    let failure = |message: String, line| {
        Some(ParseFailure {
            class: PARSE_ERROR,
            message,
            line,
        })
    };
    match outcome {
        // an action that fails records its error
        Outcome::Accept | Outcome::Aborted => None,
        Outcome::MemoryExhausted => failure("memory exhausted".into(), feed.line()),
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
            // yysyntax_error's buffer is read as a C string
            let msg = msg.split(|&c| c == 0).next().unwrap_or_default();
            failure(String::from_utf8_lossy(msg).into_owned(), feed.line())
        }
    }
}

/// The scanner as the parser pulls tokens from it (`zendlex` in parser
/// mode): whitespace, comments and open tags are skipped, `<?=` is `echo`,
/// `?>` is `;`, and lexing stops once the parser takes `__halt_compiler();`
/// as a top-level statement. The semantic value of a token is its id as
/// scanned (`<?=` stays T_OPEN_TAG_WITH_ECHO).
struct Feed<'a> {
    src: &'a [u8],
    toks: &'a [Tok],
    i: usize,
    /// The scanner's nest_location_stack: (bracket, line).
    nesting: Vec<(u8, u32)>,
    /// zend_stop_lexing() ran.
    stopped: bool,
    /// The token last handed to the parser (None: end of file).
    last: Option<usize>,
    /// The strings being scanned, innermost last (code inside `{$ }` is
    /// `Ctx::Code`).
    strings: Vec<Ctx>,
    /// The values of `non_empty_member_modifiers`: lists of modifier tokens.
    modifier_lists: Vec<Vec<u32>>,
    /// The `(` of each `declare(` whose list the parser has not finished.
    declares: Vec<usize>,
    error: Option<ParseFailure>,
    final_line: u32,
}

impl bison::Parser for Feed<'_> {
    fn lex(&mut self) -> (i32, u32) {
        let id = self.next();
        let raw = match self.last {
            Some(i) if id > 0 && id != SCANNER_ERROR => self.toks[i].id as u32,
            _ => 0,
        };
        (id, raw)
    }

    /// The actions of the rules that can refuse code; the others keep
    /// `$$ = $1`, which carries a modifier or keyword token up to them.
    fn reduce(&mut self, r: usize, rhs: &[u32]) -> Option<u32> {
        let value = match r {
            rule::TOP_HALT_COMPILER => {
                self.stopped = true;
                Ok(0)
            }
            rule::INNER_HALT_COMPILER => {
                Err("__HALT_COMPILER() can only be used from the outermost scope".to_string())
            }
            rule::IDENTIFIER_SEMI_RESERVED
            | rule::CLASS_CONST_DECL_SEMI_RESERVED
            | rule::TRAIT_ALIAS_RESERVED => {
                // zend_lex_tstring
                let t = if r == rule::TRAIT_ALIAS_RESERVED {
                    rhs[2]
                } else {
                    rhs[0]
                };
                if t == T_OPEN_TAG_WITH_ECHO as u32 {
                    let line = self.line();
                    self.error = Some(ParseFailure {
                        class: PARSE_ERROR,
                        message: "Cannot use \"<?=\" as an identifier".into(),
                        line,
                    });
                    return None;
                }
                Ok(rhs[0])
            }
            rule::CLASS_MODIFIERS_FIRST => Ok(class_flag(rhs[0])),
            rule::CLASS_MODIFIERS_ADD => add_class_modifier(rhs[0], class_flag(rhs[1])),
            rule::ANON_CLASS_MODIFIERS_FIRST => add_anonymous_class_modifier(0, class_flag(rhs[0])),
            rule::ANON_CLASS_MODIFIERS_ADD => {
                add_anonymous_class_modifier(rhs[0], class_flag(rhs[1]))
            }
            rule::MEMBER_MODIFIERS_FIRST => {
                self.modifier_lists.push(vec![rhs[0]]);
                Ok(self.modifier_lists.len() as u32 - 1)
            }
            rule::MEMBER_MODIFIERS_ADD => {
                self.modifier_lists[rhs[0] as usize].push(rhs[1]);
                Ok(rhs[0])
            }
            rule::CPP_MODIFIERS => self.modifier_list(Target::Cpp, rhs[0]),
            rule::PROPERTY_MODIFIERS => self.modifier_list(Target::Property, rhs[0]),
            rule::METHOD_MODIFIERS => self.modifier_list(Target::Method, rhs[0]),
            rule::CLASS_CONST_MODIFIERS => self.modifier_list(Target::Constant, rhs[0]),
            rule::PROPERTY_HOOK_MODIFIERS => self.modifier_list(Target::PropertyHook, rhs[0]),
            rule::TRAIT_ALIAS_MODIFIER_IDENTIFIER | rule::TRAIT_ALIAS_MODIFIER => {
                modifier_token_to_flag(Target::Method, rhs[2])
            }
            rule::DECLARE_ENCODING => self.encoding_declaration(),
            _ => return Some(rhs.first().copied().unwrap_or(0)),
        };
        match value {
            Ok(v) => Some(v),
            Err(message) => {
                // a CompileError thrown while compiling: CG(zend_lineno)
                let line = self.line();
                self.error = Some(ParseFailure {
                    class: COMPILE_ERROR,
                    message,
                    line,
                });
                None
            }
        }
    }
}

impl Feed<'_> {
    /// CG(zend_lineno) once the last token was scanned.
    fn line(&self) -> u32 {
        let Some(i) = self.last else {
            return self.final_line;
        };
        let t = &self.toks[i];
        if self.unclosed_single_quote(t) {
            // the scanner returns it without HANDLE_NEWLINES
            return t.line as u32;
        }
        let mut line = t.line as u32 + count_newlines(t.text(self.src));
        if defers_newline(self.src, self.toks, i) {
            line -= 1;
        }
        line
    }

    /// The T_ENCAPSED_AND_WHITESPACE the scanner returns for a `'` (or `b'`)
    /// in code that is never closed.
    fn unclosed_single_quote(&self, t: &Tok) -> bool {
        let text = t.text(self.src);
        t.id == T_ENCAPSED_AND_WHITESPACE
            && matches!(self.strings.last(), None | Some(Ctx::Code))
            && (text.starts_with(b"'") || text.starts_with(b"b'") || text.starts_with(b"B'"))
    }

    fn fail(&mut self, message: String, line: u32) -> i32 {
        self.error = Some(ParseFailure {
            class: PARSE_ERROR,
            message,
            line,
        });
        SCANNER_ERROR
    }

    fn modifier_list(&self, target: Target, list: u32) -> Result<u32, String> {
        modifier_list_to_flags(target, &self.modifier_lists[list as usize])
    }

    /// zend_handle_encoding_declaration's check on the `declare(…)` list
    /// just parsed: an `encoding` value must be a literal.
    fn encoding_declaration(&mut self) -> Result<u32, String> {
        let Some(open) = self.declares.pop() else {
            return Ok(0);
        };
        // the list's tokens up to its `)`, split at its commas
        let mut entries: Vec<Vec<usize>> = vec![Vec::new()];
        let mut depth = 0;
        for (k, t) in self.toks.iter().enumerate().skip(open + 1) {
            if matches!(t.id, T_WHITESPACE | T_COMMENT | T_DOC_COMMENT) {
                continue;
            }
            match t.id {
                0x28 | 0x5B | 0x7B | T_CURLY_OPEN | T_DOLLAR_OPEN_CURLY_BRACES | T_ATTRIBUTE => {
                    depth += 1
                }
                0x29 if depth == 0 => break,
                0x29 | 0x5D | 0x7D => depth -= 1,
                0x2C if depth == 0 => {
                    entries.push(Vec::new());
                    continue;
                }
                _ => {}
            }
            entries.last_mut().unwrap().push(k);
        }
        for e in &entries {
            // const_decl: T_STRING '=' expr
            let [name, _, value @ ..] = &e[..] else {
                continue;
            };
            let ids: Vec<i64> = value.iter().map(|&k| self.toks[k].id).collect();
            let named = self.toks[*name]
                .text(self.src)
                .eq_ignore_ascii_case(b"encoding");
            if named && !is_literal(&ids) {
                return Err("Encoding must be a literal".into());
            }
            if named {
                let line = self.last.map_or(0, |l| self.toks[l].line as u32);
                let msg = if super::multibyte::enabled() {
                    let name = literal_string(self.src, value.first().map(|&k| &self.toks[k]));
                    if encoding_known(&name) {
                        continue;
                    }
                    format!("Unsupported encoding [{}]", String::from_utf8_lossy(&name))
                } else {
                    "declare(encoding=...) ignored because Zend multibyte feature is turned off by settings".to_string()
                };
                PARSE_WARNINGS.with(|w| w.borrow_mut().push((msg, line)));
            }
        }
        Ok(0)
    }

    fn next(&mut self) -> i32 {
        if self.error.is_some() {
            return SCANNER_ERROR;
        }
        if self.stopped {
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
            if t.id == 0x28 && self.last.is_some_and(|p| self.toks[p].id == T_DECLARE) {
                self.declares.push(i);
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
        if !self.stopped {
            self.last = None;
        }
        if let Some(&(o, ol)) = self.nesting.last() {
            let line = self.line();
            return self.fail(bad_nesting(o, ol, line, None), line);
        }
        0
    }
}

/// Whether an expression's tokens make a literal (ZEND_AST_ZVAL) in php's
/// parser: numbers, strings and heredocs without variables, in parentheses
/// or joined by `.` (zend_ast_create_concat_op folds those).
fn is_literal(t: &[i64]) -> bool {
    let mut i = 0;
    loop {
        let n = literal_atom(&t[i..]);
        if n == 0 {
            return false;
        }
        i += n;
        match t.get(i) {
            None => return true,
            Some(0x2E) => i += 1,
            Some(_) => return false,
        }
    }
}

/// The length of the literal operand at the start of `t` (0: none).
fn literal_atom(t: &[i64]) -> usize {
    match t {
        [T_LNUMBER | T_DNUMBER | T_CONSTANT_ENCAPSED_STRING, ..] => 1,
        [T_START_HEREDOC, T_END_HEREDOC, ..] => 2,
        [T_START_HEREDOC, T_ENCAPSED_AND_WHITESPACE, T_END_HEREDOC, ..] => 3,
        [0x28, ..] => {
            let mut depth = 0;
            for (k, &id) in t.iter().enumerate() {
                match id {
                    0x28 => depth += 1,
                    0x29 => {
                        depth -= 1;
                        if depth == 0 {
                            return if is_literal(&t[1..k]) { k + 1 } else { 0 };
                        }
                    }
                    _ => {}
                }
            }
            0
        }
        _ => 0,
    }
}

/// zend_modifier_target.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Property,
    Method,
    Constant,
    Cpp,
    PropertyHook,
}

// The ZEND_ACC_* flags the modifier checks use.
const ACC_PUBLIC: u32 = 1 << 0;
const ACC_PROTECTED: u32 = 1 << 1;
const ACC_PRIVATE: u32 = 1 << 2;
const ACC_PPP_MASK: u32 = ACC_PUBLIC | ACC_PROTECTED | ACC_PRIVATE;
const ACC_STATIC: u32 = 1 << 4;
const ACC_FINAL: u32 = 1 << 5;
const ACC_ABSTRACT: u32 = 1 << 6;
const ACC_EXPLICIT_ABSTRACT_CLASS: u32 = 1 << 6;
const ACC_READONLY: u32 = 1 << 7;
const ACC_PUBLIC_SET: u32 = 1 << 10;
const ACC_PROTECTED_SET: u32 = 1 << 11;
const ACC_PRIVATE_SET: u32 = 1 << 12;
const ACC_PPP_SET_MASK: u32 = ACC_PUBLIC_SET | ACC_PROTECTED_SET | ACC_PRIVATE_SET;
const ACC_READONLY_CLASS: u32 = 1 << 16;
const ACC_NO_DYNAMIC_PROPERTIES: u32 = 1 << 13;

/// class_modifier's action.
fn class_flag(token: u32) -> u32 {
    match token as i64 {
        T_ABSTRACT => ACC_EXPLICIT_ABSTRACT_CLASS,
        T_FINAL => ACC_FINAL,
        _ => ACC_READONLY_CLASS | ACC_NO_DYNAMIC_PROPERTIES,
    }
}

/// zend_add_class_modifier.
fn add_class_modifier(flags: u32, new_flag: u32) -> Result<u32, String> {
    let new_flags = flags | new_flag;
    let both = |f: u32| flags & f != 0 && new_flag & f != 0;
    if both(ACC_EXPLICIT_ABSTRACT_CLASS) {
        return Err("Multiple abstract modifiers are not allowed".into());
    }
    if both(ACC_FINAL) {
        return Err("Multiple final modifiers are not allowed".into());
    }
    if both(ACC_READONLY_CLASS) {
        return Err("Multiple readonly modifiers are not allowed".into());
    }
    if new_flags & ACC_EXPLICIT_ABSTRACT_CLASS != 0 && new_flags & ACC_FINAL != 0 {
        return Err("Cannot use the final modifier on an abstract class".into());
    }
    Ok(new_flags)
}

/// zend_add_anonymous_class_modifier.
fn add_anonymous_class_modifier(flags: u32, new_flag: u32) -> Result<u32, String> {
    if new_flag & ACC_EXPLICIT_ABSTRACT_CLASS != 0 {
        return Err("Cannot use the abstract modifier on an anonymous class".into());
    }
    if new_flag & ACC_FINAL != 0 {
        return Err("Cannot use the final modifier on an anonymous class".into());
    }
    if flags & ACC_READONLY_CLASS != 0 && new_flag & ACC_READONLY_CLASS != 0 {
        return Err("Multiple readonly modifiers are not allowed".into());
    }
    Ok(flags | new_flag)
}

/// zend_modifier_token_to_string.
fn modifier_name(token: u32) -> &'static str {
    match token as i64 {
        T_PUBLIC => "public",
        T_PROTECTED => "protected",
        T_PRIVATE => "private",
        T_STATIC => "static",
        T_FINAL => "final",
        T_READONLY => "readonly",
        T_ABSTRACT => "abstract",
        T_PUBLIC_SET => "public(set)",
        T_PROTECTED_SET => "protected(set)",
        _ => "private(set)",
    }
}

/// zend_modifier_token_to_flag.
fn modifier_token_to_flag(target: Target, token: u32) -> Result<u32, String> {
    use Target::*;
    let flag = match token as i64 {
        T_PUBLIC if target != PropertyHook => Some(ACC_PUBLIC),
        T_PROTECTED if target != PropertyHook => Some(ACC_PROTECTED),
        T_PRIVATE if target != PropertyHook => Some(ACC_PRIVATE),
        T_READONLY if matches!(target, Property | Cpp) => Some(ACC_READONLY),
        T_ABSTRACT if matches!(target, Method | Property) => Some(ACC_ABSTRACT),
        T_FINAL => Some(ACC_FINAL),
        T_STATIC if matches!(target, Property | Method) => Some(ACC_STATIC),
        T_PUBLIC_SET if matches!(target, Property | Cpp) => Some(ACC_PUBLIC_SET),
        T_PROTECTED_SET if matches!(target, Property | Cpp) => Some(ACC_PROTECTED_SET),
        T_PRIVATE_SET if matches!(target, Property | Cpp) => Some(ACC_PRIVATE_SET),
        _ => None,
    };
    flag.ok_or_else(|| {
        let member = match target {
            Property => "property",
            Method => "method",
            Constant => "class constant",
            Cpp => "parameter",
            PropertyHook => "property hook",
        };
        format!(
            "Cannot use the {} modifier on a {member}",
            modifier_name(token)
        )
    })
}

/// zend_modifier_list_to_flags.
fn modifier_list_to_flags(target: Target, tokens: &[u32]) -> Result<u32, String> {
    let mut flags = 0;
    for &token in tokens {
        let new_flag = modifier_token_to_flag(target, token)?;
        // zend_add_member_modifier's errors come before a duplicate's
        let duplicate = flags & new_flag != 0;
        flags = add_member_modifier(flags, new_flag, target)?;
        if duplicate {
            return Err(format!(
                "Multiple {} modifiers are not allowed",
                modifier_name(token)
            ));
        }
    }
    Ok(flags)
}

/// zend_add_member_modifier.
fn add_member_modifier(flags: u32, new_flag: u32, target: Target) -> Result<u32, String> {
    let new_flags = flags | new_flag;
    if flags & ACC_PPP_MASK != 0 && new_flag & ACC_PPP_MASK != 0 {
        return Err("Multiple access type modifiers are not allowed".into());
    }
    if new_flags & ACC_ABSTRACT != 0 && new_flags & ACC_FINAL != 0 {
        match target {
            Target::Method => {
                return Err("Cannot use the final modifier on an abstract method".into())
            }
            Target::Property => {
                return Err("Cannot use the final modifier on an abstract property".into())
            }
            _ => {}
        }
    }
    if matches!(target, Target::Property | Target::Cpp)
        && flags & ACC_PPP_SET_MASK != 0
        && new_flag & ACC_PPP_SET_MASK != 0
    {
        return Err("Multiple access type modifiers are not allowed".into());
    }
    Ok(new_flags)
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
    let truncated = content.len() > 33;
    if truncated {
        content = &content[..30];
    }
    // snprintf's `%.*s` stops at a NUL, but yysyntax_error advances by the
    // full length, past the string's end: the message ends after this name
    let nul = content.iter().position(|&c| c == 0);
    m.extend_from_slice(&content[..nul.unwrap_or(content.len())]);
    if truncated {
        m.extend_from_slice(b"...");
    }
    m.push(b'"');
    if nul.is_some() {
        m.push(0);
    }
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
