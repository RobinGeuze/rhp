//! ext/readline/readline_cli.c: php-cli's interactive shell (`php -a`).
//! Lines are read with libedit's `readline()` until they make a complete
//! statement (`cli_is_valid_code`), then evaluated in the global scope as
//! "php shell code" ([`Host::shell_eval`]). `#name=value` sets an ini
//! directive; `exit` / `quit` end the shell; the prompt comes from
//! `cli.prompt`; `cli.pager` pipes each statement's output through a
//! command; tab completes variables, `#` directives, functions,
//! constants and classes.

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_int, CStr, CString};

use rhp_values::{ops::deref, Value};

use super::sys;
use crate::Host;

const DEFAULT_PROMPT: &[u8] = b"\\b \\> ";

thread_local! {
    /// The shell is running: php-cli's writes go through [`shell_output`].
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// `php_last_char`: the last byte written by the last statement.
    static LAST_CHAR: Cell<u8> = const { Cell::new(0) };
    /// `CLIR_G(prompt_str)`: prompt code's output, captured.
    static PROMPT_STR: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    /// `pager_pipe`: the pager of the running statement.
    static PAGER: Cell<*mut libc::FILE> = const { Cell::new(std::ptr::null_mut()) };
    /// The pager command (`cli.pager`) for the running statement.
    static PAGER_CMD: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// `readline_shell_write` + `readline_shell_ub_write`: what php-cli's
/// SAPI calls for each write while the shell runs (the write itself still
/// goes to stdout): the last byte is noted, and the bytes are copied to the
/// prompt being built or to the pager.
pub fn shell_output(data: &[u8]) {
    if !ACTIVE.with(Cell::get) || data.is_empty() {
        return;
    }
    LAST_CHAR.with(|c| c.set(data[data.len() - 1]));
    let captured = PROMPT_STR.with(|p| match p.borrow_mut().as_mut() {
        Some(buf) => {
            buf.extend_from_slice(data);
            true
        }
        None => false,
    });
    if captured {
        return;
    }
    let mut pipe = PAGER.with(Cell::get);
    if pipe.is_null() {
        let cmd = PAGER_CMD.with(|c| c.borrow().clone());
        if cmd.is_empty() {
            return;
        }
        let Ok(c) = CString::new(cmd) else {
            return;
        };
        // SAFETY: popen with valid C strings.
        pipe = unsafe { libc::popen(c.as_ptr(), c"w".as_ptr()) };
        PAGER.with(|p| p.set(pipe));
    }
    if !pipe.is_null() {
        // SAFETY: a live pipe from popen.
        unsafe { libc::fwrite(data.as_ptr().cast(), 1, data.len().min(16384), pipe) };
    }
}

/// `cli_get_prompt`: `cli.prompt` with its escapes expanded, backtick code
/// run, `block` for `\b` and `prompt` for `\>`.
fn get_prompt(host: &mut dyn Host, block: &str, prompt: u8) -> Vec<u8> {
    let spec = host
        .ini_get(b"cli.prompt")
        .unwrap_or_else(|| DEFAULT_PROMPT.to_vec());
    let mut out = Vec::new();
    let mut unicode_warned = false;
    let mut i = 0;
    while i < spec.len() {
        let c = spec[i];
        if c == b'\\' {
            match spec.get(i + 1).copied().unwrap_or(0) {
                b'\\' => {
                    out.push(b'\\');
                    i += 1;
                }
                b'n' => {
                    out.push(b'\n');
                    i += 1;
                }
                b't' => {
                    out.push(b'\t');
                    i += 1;
                }
                b'e' => {
                    out.push(0x1b);
                    i += 1;
                }
                b'v' => {
                    out.extend_from_slice(crate::version::PHP_VERSION.as_bytes());
                    i += 1;
                }
                b'b' => {
                    out.extend_from_slice(block.as_bytes());
                    i += 1;
                }
                b'>' => {
                    out.push(prompt);
                    i += 1;
                }
                b'`' => {
                    out.push(b'`');
                    i += 1;
                }
                _ => out.push(b'\\'),
            }
        } else if c == b'`' {
            if let Some(end) = spec[i + 1..].iter().position(|&b| b == b'`') {
                let code = spec[i + 1..i + 1 + end].to_vec();
                PROMPT_STR.with(|p| *p.borrow_mut() = Some(Vec::new()));
                host.shell_eval(&code, "php prompt code");
                let got = PROMPT_STR
                    .with(|p| p.borrow_mut().take())
                    .unwrap_or_default();
                out.extend_from_slice(&got);
                i += 1 + end;
            }
        } else if c & 0x80 == 0 {
            out.push(c);
        } else {
            if !unicode_warned {
                host.raise_diagnostic(
                    crate::E_WARNING,
                    b"prompt contains unsupported unicode characters",
                );
                unicode_warned = true;
            }
            out.push(b'?');
        }
        i += 1;
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum CodeType {
    Body,
    SString,
    DString,
    SStringEsc,
    DStringEsc,
    CommentLine,
    CommentBlock,
    HeredocStart,
    Heredoc,
    Outside,
}

/// `cli_is_valid_code`: whether `code` ends a statement, and the prompt
/// for the next line.
fn is_valid_code(host: &mut dyn Host, code: &[u8]) -> (bool, Vec<u8>) {
    let at = |i: usize| code.get(i).copied().unwrap_or(0);
    let len = code.len();
    let mut valid_end = true;
    let mut last_valid_end = true;
    let mut brackets = 0usize;
    let mut braces = 0usize;
    let mut ty = CodeType::Body;
    let mut tag: Option<usize> = None;
    let mut tag_len = 0usize;
    let short_tags = crate::short_open_tag();
    let mut i = 0;
    while i < len {
        let c = code[i];
        match ty {
            CodeType::Body => match c {
                b'{' => {
                    brackets += 1;
                    valid_end = false;
                }
                b'}' => {
                    brackets = brackets.saturating_sub(1);
                    valid_end = brackets == 0;
                }
                b'(' => {
                    braces += 1;
                    valid_end = false;
                }
                b')' => {
                    braces = braces.saturating_sub(1);
                    valid_end = false;
                }
                b';' => valid_end = braces == 0 && brackets == 0,
                b' ' | b'\r' | b'\n' | b'\t' => {}
                b'\'' => ty = CodeType::SString,
                b'"' => ty = CodeType::DString,
                b'#' => {
                    if at(i + 1) == b'[' {
                        valid_end = false;
                    } else {
                        ty = CodeType::CommentLine;
                    }
                }
                b'/' => {
                    if at(i + 1) == b'/' {
                        i += 1;
                        ty = CodeType::CommentLine;
                    } else if at(i + 1) == b'*' {
                        last_valid_end = valid_end;
                        valid_end = false;
                        ty = CodeType::CommentBlock;
                        i += 1;
                    } else {
                        valid_end = false;
                    }
                }
                b'?' => {
                    if at(i + 1) == b'>' {
                        i += 1;
                        ty = CodeType::Outside;
                    } else {
                        valid_end = false;
                    }
                }
                b'<' => {
                    valid_end = false;
                    if i + 2 < len && code[i + 1] == b'<' && code[i + 2] == b'<' {
                        i += 2;
                        ty = CodeType::HeredocStart;
                        tag = None;
                        tag_len = 0;
                    }
                }
                _ => valid_end = false,
            },
            CodeType::SString => {
                if c == b'\\' {
                    ty = CodeType::SStringEsc;
                } else if c == b'\'' {
                    ty = CodeType::Body;
                }
            }
            CodeType::SStringEsc => ty = CodeType::SString,
            CodeType::DString => {
                if c == b'\\' {
                    ty = CodeType::DStringEsc;
                } else if c == b'"' {
                    ty = CodeType::Body;
                }
            }
            CodeType::DStringEsc => ty = CodeType::DString,
            CodeType::CommentLine => {
                if c == b'\n' {
                    ty = CodeType::Body;
                }
            }
            CodeType::CommentBlock => {
                if i > 0 && code[i - 1] == b'*' && c == b'/' {
                    ty = CodeType::Body;
                    valid_end = last_valid_end;
                }
            }
            CodeType::HeredocStart => match c {
                b' ' | b'\t' | b'\'' | b'"' => {}
                b'\r' | b'\n' => {
                    ty = if tag.is_some() {
                        CodeType::Heredoc
                    } else {
                        CodeType::Body
                    };
                }
                _ => {
                    if tag.is_none() {
                        tag = Some(i);
                    }
                    tag_len += 1;
                }
            },
            CodeType::Heredoc => {
                let t = tag.unwrap_or(0);
                let start = (i + 1).checked_sub(tag_len);
                if let Some(s) = start.filter(|&s| code[s..].starts_with(&code[t..t + tag_len])) {
                    let n = at(i + 1);
                    if !(n.is_ascii_alphanumeric() || n == b'_' || n >= 0x80) {
                        // The label must start its line, after indentation.
                        let mut p = s as isize - 1;
                        while p >= 0 && matches!(code[p as usize], b' ' | b'\t') {
                            p -= 1;
                        }
                        if p >= 0 && code[p as usize] == b'\n' {
                            ty = CodeType::Body;
                        }
                    }
                }
            }
            CodeType::Outside => {
                if (short_tags && i >= 1 && code[i - 1..].starts_with(b"<?"))
                    || (i > 3 && code[i - 4..].starts_with(b"<?php"))
                {
                    ty = CodeType::Body;
                }
            }
        }
        i += 1;
    }
    let prompt = match ty {
        CodeType::SString | CodeType::SStringEsc => get_prompt(host, "php", b'\''),
        CodeType::DString | CodeType::DStringEsc => get_prompt(host, "php", b'"'),
        CodeType::CommentBlock => get_prompt(host, "/* ", b'>'),
        CodeType::Heredoc => get_prompt(host, "<<<", b'>'),
        CodeType::Outside => get_prompt(host, "   ", b'>'),
        _ if braces > 0 => get_prompt(host, "php", b'('),
        _ if brackets > 0 => get_prompt(host, "php", b'{'),
        _ => get_prompt(host, "php", b'>'),
    };
    (valid_end && brackets == 0, prompt)
}

extern "C" {
    static mut rl_special_prefixes: *const c_char;
    static mut rl_completion_append_character: c_int;
}

/// Completion candidates: each word and the character appended after it.
type Candidates = Vec<(Vec<u8>, u8)>;

thread_local! {
    /// The candidates for the word being completed, and the generator's
    /// position in them.
    static CANDIDATES: RefCell<(Candidates, usize)> = const { RefCell::new((Vec::new(), 0)) };
}

/// The names a builtin returns, as an array's values (or keys).
fn names(host: &mut dyn Host, func: &str, args: &[Value], keys: bool) -> Vec<Vec<u8>> {
    let Ok(v) = host.call_callable_named(&Value::str(func), args, Vec::new()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    fn walk(v: &Value, keys: bool, out: &mut Vec<Vec<u8>>) {
        if let Value::Array(a) = deref(v) {
            for (k, v) in a.iter() {
                if keys {
                    out.push(key_bytes(&k));
                } else if let Value::Array(_) = deref(v) {
                    walk(v, keys, out);
                } else if let Value::Str(s) = deref(v) {
                    out.push(s.as_bytes().to_vec());
                }
            }
        }
    }
    walk(&v, keys, &mut out);
    out
}

/// `cli_completion_generator`'s candidates for `text`, with the character
/// libedit appends after each.
fn candidates(host: &mut dyn Host, text: &[u8]) -> Candidates {
    let prefixed = |list: Vec<Vec<u8>>, sigil: u8, want: &[u8], append: u8| {
        list.into_iter()
            .filter(|n| n.starts_with(want))
            .map(|n| {
                let mut s = vec![sigil];
                s.extend_from_slice(&n);
                (s, append)
            })
            .collect::<Vec<_>>()
    };
    if text.first() == Some(&b'$') {
        let vars = host
            .global_var(b"GLOBALS")
            .map(|g| names_of(&g))
            .unwrap_or_default();
        return prefixed(vars, b'$', &text[1..], 0);
    }
    if text.first() == Some(&b'#') && text.get(1) != Some(&b'[') {
        let ini = names(
            host,
            "ini_get_all",
            &[Value::Null, Value::Bool(false)],
            true,
        );
        return prefixed(ini, b'#', &text[1..], b'=');
    }
    let lc = text.to_ascii_lowercase();
    let mut out: Candidates = names(host, "get_defined_functions", &[], false)
        .into_iter()
        .filter(|n| n.to_ascii_lowercase().starts_with(&lc))
        .map(|n| (n, b'('))
        .collect();
    out.extend(
        names(host, "get_defined_constants", &[], true)
            .into_iter()
            .filter(|n| n.starts_with(text))
            .map(|n| (n, 0)),
    );
    out.extend(
        names(host, "get_declared_classes", &[], false)
            .into_iter()
            .filter(|n| n.to_ascii_lowercase().starts_with(&lc))
            .map(|n| (n, 0)),
    );
    out
}

fn key_bytes(k: &rhp_values::ArrayKey) -> Vec<u8> {
    match k {
        rhp_values::ArrayKey::Int(i) => i.to_string().into_bytes(),
        rhp_values::ArrayKey::Str(s) => s.as_bytes().to_vec(),
    }
}

fn names_of(v: &Value) -> Vec<Vec<u8>> {
    match deref(v) {
        Value::Array(a) => a.iter().map(|(k, _)| key_bytes(&k)).collect(),
        _ => Vec::new(),
    }
}

unsafe extern "C" fn generator(text: *const c_char, state: c_int) -> *mut c_char {
    if state == 0 {
        let t = if text.is_null() {
            Vec::new()
        } else {
            // SAFETY: libedit's word.
            unsafe { CStr::from_ptr(text) }.to_bytes().to_vec()
        };
        let list = match super::HOST.with(|h| h.get()) {
            // SAFETY: published by with_host for the readline call.
            Some(host) => candidates(unsafe { &mut *host }, &t),
            None => Vec::new(),
        };
        CANDIDATES.with(|c| *c.borrow_mut() = (list, 0));
    }
    CANDIDATES.with(|c| {
        let mut c = c.borrow_mut();
        let i = c.1;
        let Some((word, append)) = c.0.get(i).cloned() else {
            return std::ptr::null_mut();
        };
        c.1 += 1;
        // SAFETY: libedit's global; it frees the strdup'd match.
        unsafe {
            rl_completion_append_character = append as c_int;
            match CString::new(word) {
                Ok(s) => libc::strdup(s.as_ptr()),
                Err(_) => std::ptr::null_mut(),
            }
        }
    })
}

/// `cli_code_completion`.
unsafe extern "C" fn code_completion(
    text: *const c_char,
    _start: c_int,
    _end: c_int,
) -> *mut *mut c_char {
    // SAFETY: libedit's word; the generator returns malloc'd strings.
    unsafe { sys::rl_completion_matches(text, generator) }
}

/// `readline_shell_run`: the shell, until end of input, `exit` or `quit`.
/// Returns the exit status (`EG(exit_status)`).
pub fn shell_run(host: &mut dyn Host) -> i32 {
    ACTIVE.with(|a| a.set(true));
    let history_file = match std::env::var_os("PHP_HISTFILE") {
        Some(f) => f.into_encoded_bytes(),
        None => {
            let mut h = std::env::var_os("HOME")
                .map(|h| h.into_encoded_bytes())
                .unwrap_or_else(|| b"(null)".to_vec());
            h.extend_from_slice(b"/.php_history");
            h
        }
    };
    let history_c = CString::new(history_file).unwrap_or_default();
    // SAFETY: libedit's globals, set on this thread before readline runs.
    unsafe {
        // A completion function PHP code installed (auto_prepend_file) wins.
        if super::COMPLETION.with(|c| c.borrow().is_none()) {
            sys::rl_attempted_completion_function = Some(code_completion);
        }
        rl_special_prefixes = c"$".as_ptr();
        sys::read_history(history_c.as_ptr());
    }
    host.set_exit_status(0);
    let mut prompt = get_prompt(host, "php", b'>');
    let mut code: Vec<u8> = Vec::new();
    let mut history_lines = 0;
    loop {
        let p = CString::new(prompt.clone()).unwrap_or_default();
        // SAFETY: a live prompt; the line is malloc'd.
        let line = super::with_host(host, || unsafe { sys::readline(p.as_ptr()) })
            .unwrap_or(std::ptr::null_mut());
        if line.is_null() {
            break;
        }
        // SAFETY: readline's line, freed once copied.
        let l = unsafe { CStr::from_ptr(line) }.to_bytes().to_vec();
        unsafe { libc::free(line.cast()) };
        if l == b"exit" || l == b"quit" {
            break;
        }
        if code.is_empty() && l.is_empty() {
            continue;
        }
        if l.first() == Some(&b'#') && l.get(1) != Some(&b'[') {
            if let Some(eq) = l[1..].iter().position(|&b| b == b'=') {
                let name = l[1..1 + eq].to_vec();
                let value = l[2 + eq..].to_vec();
                // zend_alter_ini_entry_chars_ex at runtime, quietly.
                let args = [Value::str(name), Value::str(value)];
                let _ = host.call_callable_named(&Value::str("ini_set"), &args, Vec::new());
                let c = CString::new(l.clone()).unwrap_or_default();
                // SAFETY: a live C string.
                unsafe { sys::add_history(c.as_ptr()) };
                prompt = get_prompt(host, "php", b'>');
                continue;
            }
        }
        code.extend_from_slice(&l);
        code.push(b'\n');
        if !l.is_empty() {
            let c = CString::new(l.clone()).unwrap_or_default();
            // SAFETY: a live C string.
            unsafe { sys::add_history(c.as_ptr()) };
            history_lines += 1;
        }
        let (valid, next) = is_valid_code(host, &code);
        prompt = next;
        if !valid {
            continue;
        }
        if history_lines > 0 {
            // SAFETY: a live C string.
            unsafe { sys::write_history(history_c.as_ptr()) };
            history_lines = 0;
        }
        PAGER_CMD.with(|c| *c.borrow_mut() = host.ini_get(b"cli.pager").unwrap_or_default());
        let chunk = std::mem::take(&mut code);
        host.shell_eval(&chunk, "php shell code");
        let pager = PAGER.with(Cell::get);
        let last = LAST_CHAR.with(Cell::get);
        if pager.is_null() && last != 0 && last != b'\n' {
            host.write_out(b"\n");
        }
        let pager = PAGER.with(|p| p.replace(std::ptr::null_mut()));
        if !pager.is_null() {
            // SAFETY: the pipe popen opened.
            unsafe { libc::pclose(pager) };
        }
        LAST_CHAR.with(|c| c.set(0));
    }
    ACTIVE.with(|a| a.set(false));
    host.exit_status()
}

#[cfg(test)]
mod tests;
