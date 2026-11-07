//! The `tokenizer` extension: `token_get_all`, `token_name`, the `T_*`
//! constants and `PhpToken`, on a port of the Zend scanner (`lexer`).
//!
//! `TOKEN_PARSE` runs php's parser (`crate::zend`): invalid code throws its
//! ParseError. Semi-reserved keywords the parser would accept as identifiers
//! (after `::`, method/const/enum-case names, named arguments, trait aliases)
//! are re-tagged as `T_STRING` by a heuristic, not by the parser's feedback.

pub(crate) mod ids;
pub(crate) mod lexer;

use rhp_values::ops::deref;
use rhp_values::{conv, PhpArray, PhpStr, Value};

use crate::{Host, Thrown};
use ids::*;
use lexer::Tok;

pub const TOKEN_PARSE: i64 = 1;

fn is_trivia(id: i64) -> bool {
    matches!(id, T_WHITESPACE | T_COMMENT | T_DOC_COMMENT | T_OPEN_TAG)
}

fn is_modifier(id: i64) -> bool {
    matches!(
        id,
        T_PUBLIC | T_PROTECTED | T_PRIVATE | T_STATIC | T_ABSTRACT | T_FINAL | T_READONLY
    )
}

/// TOKEN_PARSE's identifier feedback (see module docs).
fn apply_parse_mode(toks: &mut [Tok]) {
    let sig: Vec<usize> = (0..toks.len())
        .filter(|&i| !is_trivia(toks[i].id))
        .collect();
    let id_at = |toks: &[Tok], k: Option<usize>| k.map(|k| toks[sig[k]].id).unwrap_or(0);
    let mut changes = Vec::new();
    for (k, &i) in sig.iter().enumerate() {
        let id = toks[i].id;
        if !lexer::is_keyword_id(id) {
            continue;
        }
        let prev = id_at(toks, k.checked_sub(1));
        let prev2 = id_at(toks, k.checked_sub(2));
        let next = id_at(toks, Some(k + 1).filter(|&n| n < sig.len()));
        let amp = matches!(
            prev,
            T_AMPERSAND_FOLLOWED_BY_VAR_OR_VARARG | T_AMPERSAND_NOT_FOLLOWED_BY_VAR_OR_VARARG
        );
        let ident = prev == T_PAAMAYIM_NEKUDOTAYIM
            || ((prev == T_FUNCTION || (amp && prev2 == T_FUNCTION)) && next == b'(' as i64)
            || (prev == T_CONST && next == b'=' as i64)
            || (prev == T_CASE && (next == b'=' as i64 || next == b';' as i64))
            || (next == b':' as i64 && (prev == b'(' as i64 || prev == b',' as i64))
            || ((next == T_AS || next == T_INSTEADOF)
                && (prev == b'{' as i64 || prev == b';' as i64))
            || (prev == T_AS && next == b';' as i64 && !is_modifier(id))
            || (is_modifier(prev) && prev2 == T_AS && next == b';' as i64);
        if ident {
            changes.push(i);
        }
    }
    for i in changes {
        toks[i].id = T_STRING;
    }
}

fn lex(host: &mut dyn Host, code: &[u8], flags: i64) -> Result<Vec<Tok>, Thrown> {
    if flags & TOKEN_PARSE != 0 {
        if let Some(f) = crate::zend::syntax_error(code) {
            return Err(parse_error(host, f));
        }
    }
    let mut toks = lexer::tokenize(code);
    if flags & TOKEN_PARSE != 0 {
        apply_parse_mode(&mut toks);
    }
    Ok(toks)
}

/// The ParseError (or CompileError) of a TOKEN_PARSE run: php locates it at
/// the error's line in a nameless file.
fn parse_error(host: &mut dyn Host, f: crate::zend::ParseFailure) -> Thrown {
    let crate::zend::ParseFailure {
        class,
        message,
        line,
    } = f;
    let fallback = Thrown::error(class, message.clone());
    let Ok(e @ Value::Object(_)) =
        host.reflect_new_instance(class.as_bytes(), vec![Value::str(message)], true)
    else {
        return fallback;
    };
    let located = host
        .reflect_set_prop(&e, b"Error", b"file", Value::str(""))
        .and_then(|_| host.reflect_set_prop(&e, b"Error", b"line", Value::Int(line as i64)));
    match (located, e) {
        (Ok(()), Value::Object(o)) => Thrown::Object(o),
        _ => fallback,
    }
}

fn fn_token_get_all(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = conv::to_string_bytes(&deref(&args[0]));
    let flags = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    let mut toks = lex(host, &code, flags)?;
    // the doubling slack would stay allocated while the array is built
    toks.shrink_to_fit();
    let mut texts = TextCache::default();
    // Equal short tokens on one line (the spaces between tokens) share one
    // array: copy-on-write keeps them apart, only refcounts differ.
    let mut same_line: rustc_hash::FxHashMap<(i64, &[u8]), PhpArray> = Default::default();
    let mut line = 0;
    let mut out = PhpArray::with_capacity(toks.len());
    for t in &toks {
        let src = t.text(&code);
        if t.id < 256 {
            out.push(Value::Str(texts.get(src)));
            continue;
        }
        if t.line != line {
            same_line.clear();
            line = t.line;
        }
        let shared = src.len() <= SHORT_TEXT;
        if shared {
            if let Some(a) = same_line.get(&(t.id, src)) {
                out.push(Value::Array(a.clone()));
                continue;
            }
        }
        let mut a = PhpArray::with_capacity(3);
        a.push(Value::Int(t.id));
        a.push(Value::Str(texts.get(src)));
        a.push(Value::Int(t.line));
        if shared {
            same_line.insert((t.id, src), a.clone());
        }
        out.push(Value::Array(a));
    }
    Ok(Value::Array(out))
}

/// Token texts up to this length are shared.
const SHORT_TEXT: usize = 32;

/// One string per distinct short token text (whitespace, names, operators),
/// so a big file's tokens share them; php interns one-character strings.
#[derive(Default)]
struct TextCache<'a>(rustc_hash::FxHashMap<&'a [u8], PhpStr>);

impl<'a> TextCache<'a> {
    fn get(&mut self, text: &'a [u8]) -> PhpStr {
        if text.len() > SHORT_TEXT {
            return PhpStr::from_bytes(text);
        }
        self.0
            .entry(text)
            .or_insert_with(|| PhpStr::from_bytes(text))
            .clone()
    }
}

/// `[id, text, line, pos]` for every token (single chars get their byte as id);
/// backs `PhpToken::tokenize`.
fn fn_token_list(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = conv::to_string_bytes(&deref(&args[0]));
    let flags = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    let mut out = PhpArray::new();
    for t in lex(host, &code, flags)? {
        let mut a = PhpArray::new();
        a.push(Value::Int(t.id));
        a.push(Value::str(t.text(&code)));
        a.push(Value::Int(t.line));
        a.push(Value::Int(t.pos as i64));
        out.push(Value::Array(a));
    }
    Ok(Value::Array(out))
}

fn fn_token_name(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let id = conv::to_int(&deref(&args[0]));
    Ok(Value::str(token_name(id)))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("token_get_all", fn_token_get_all),
            ("token_name", fn_token_name),
        ],
    );
    crate::register_helpers(m, &[("__token_list", fn_token_list, 1, Some(2))]);
}

pub fn constants() -> Vec<(&'static str, Value)> {
    let mut v: Vec<(&'static str, Value)> =
        ALL.iter().map(|(n, id)| (*n, Value::Int(*id))).collect();
    v.push(("TOKEN_PARSE", Value::Int(TOKEN_PARSE)));
    v
}

pub const PRELUDE_PHP: &str = include_str!("prelude.php");
