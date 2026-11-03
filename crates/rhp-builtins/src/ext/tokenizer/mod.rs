//! The `tokenizer` extension: `token_get_all`, `token_name`, the `T_*`
//! constants and `PhpToken`, on a port of the Zend scanner (`lexer`).
//!
//! `TOKEN_PARSE` runs php's parser (`crate::zend`): invalid code throws its
//! ParseError. Semi-reserved keywords the parser would accept as identifiers
//! (after `::`, method/const/enum-case names, named arguments, trait aliases)
//! are re-tagged as `T_STRING` by a heuristic, not by the parser's feedback.

pub(crate) mod ids;
pub(crate) mod lexer;

use std::collections::HashMap;

use rhp_values::{conv, PhpArray, Value};

use crate::{Builtin, Host, Thrown};
use ids::*;
use lexer::Tok;

pub const TOKEN_PARSE: i64 = 1;

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

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

fn lex(code: &[u8], flags: i64) -> Result<Vec<Tok>, Thrown> {
    if flags & TOKEN_PARSE != 0 {
        if let Some((message, _line)) = crate::zend::syntax_error(code) {
            return Err(Thrown::error("ParseError", message));
        }
    }
    let mut toks = lexer::tokenize(code);
    if flags & TOKEN_PARSE != 0 {
        apply_parse_mode(&mut toks);
    }
    Ok(toks)
}

fn fn_token_get_all(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = conv::to_string_bytes(&deref(&args[0]));
    let flags = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    let mut out = PhpArray::new();
    for t in lex(&code, flags)? {
        if t.id < 256 {
            out.push(Value::str(t.text(&code).to_vec()));
        } else {
            let mut a = PhpArray::new();
            a.push(Value::Int(t.id));
            a.push(Value::str(t.text(&code).to_vec()));
            a.push(Value::Int(t.line));
            out.push(Value::Array(a));
        }
    }
    Ok(Value::Array(out))
}

/// `[id, text, line, pos]` for every token (single chars get their byte as id);
/// backs `PhpToken::tokenize`.
fn fn_token_list(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = conv::to_string_bytes(&deref(&args[0]));
    let flags = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    let mut out = PhpArray::new();
    for t in lex(&code, flags)? {
        let mut a = PhpArray::new();
        a.push(Value::Int(t.id));
        a.push(Value::str(t.text(&code).to_vec()));
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

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    let table: &[(&'static str, crate::NativeFn, usize, Option<usize>)] = &[
        ("token_get_all", fn_token_get_all, 1, Some(2)),
        ("token_name", fn_token_name, 1, Some(1)),
        ("__token_list", fn_token_list, 1, Some(2)),
    ];
    for &(name, func, min, max) in table {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    }
}

pub fn constants() -> Vec<(&'static str, Value)> {
    let mut v: Vec<(&'static str, Value)> =
        ALL.iter().map(|(n, id)| (*n, Value::Int(*id))).collect();
    v.push(("TOKEN_PARSE", Value::Int(TOKEN_PARSE)));
    v
}

pub const PRELUDE_PHP: &str = r#"
class PhpToken implements Stringable {
    public int $id;
    public string $text;
    public int $line;
    public int $pos;

    final public function __construct(int $id, string $text, int $line = -1, int $pos = -1) {
        $this->id = $id;
        $this->text = $text;
        $this->line = $line;
        $this->pos = $pos;
    }
    public static function tokenize(string $code, int $flags = 0): array {
        $out = [];
        foreach (__token_list($code, $flags) as $t) {
            $out[] = new static($t[0], $t[1], $t[2], $t[3]);
        }
        return $out;
    }
    public function getTokenName(): ?string {
        if ($this->id < 256) { return chr($this->id); }
        $n = token_name($this->id);
        return $n === 'UNKNOWN' ? null : $n;
    }
    public function is($kind): bool {
        if (is_int($kind)) { return $this->id === $kind; }
        if (is_string($kind)) { return $this->text === $kind; }
        if (is_array($kind)) {
            foreach ($kind as $k) {
                if (is_int($k)) { if ($this->id === $k) { return true; } }
                elseif (is_string($k)) { if ($this->text === $k) { return true; } }
                else { throw new TypeError('PhpToken::is(): Argument #1 ($kind) must only have elements of type string|int, ' . get_debug_type($k) . ' given'); }
            }
            return false;
        }
        throw new TypeError('PhpToken::is(): Argument #1 ($kind) must be of type string|int|array, ' . get_debug_type($kind) . ' given');
    }
    public function isIgnorable(): bool {
        return $this->id === T_WHITESPACE || $this->id === T_COMMENT || $this->id === T_DOC_COMMENT || $this->id === T_OPEN_TAG;
    }
    public function __toString(): string { return $this->text; }
}
"#;
