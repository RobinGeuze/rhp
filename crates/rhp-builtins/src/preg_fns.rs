//! The PCRE `preg_*` builtin family: ext/pcre/php_pcre.c's glue over the
//! real PCRE2 (see [`crate::pcre`] for the library and its per-thread state).
//!
//! Matching is PCRE2's own, so every construct behaves as in php. This file
//! ports php's part: delimiter and modifier parsing, the compiled-pattern
//! cache, `$matches` shaping, php's `$1`/`\1`/`${1}` replacements, the
//! empty-match iteration and preg_last_error().
//!
//! ## Byte semantics
//! php strings are *byte* strings, and so is PCRE2's 8-bit API: patterns and
//! subjects are passed as raw `&[u8]` with an explicit length (no UTF-8
//! decoding, no NUL-termination assumptions). Byte offsets reported for
//! `PREG_OFFSET_CAPTURE` are therefore identical to php's.
//!
//! ## Modifiers → compile options
//! The trailing modifier letters map to PCRE2 compile options exactly as php
//! maps them: `i`=CASELESS, `m`=MULTILINE, `n`=NO_AUTO_CAPTURE, `s`=DOTALL,
//! `x`=EXTENDED, `A`=ANCHORED, `D`=DOLLAR_ENDONLY, `U`=UNGREEDY,
//! `J`=DUPNAMES, `r`=EXTRA_CASELESS_RESTRICT (PCRE2 10.43+),
//! `u`=UTF|UCP|NEVER_BACKSLASH_C. `S` and `X` are accepted as no-ops (as php
//! does); any other modifier letter is rejected → `false`/`null`.
//!
//! ## Empty-match iteration
//! Global operations (`preg_match_all`, `preg_replace*`, `preg_split`) replicate
//! php's exact empty-match handling: after an empty match, the next attempt is
//! retried with `PCRE2_ANCHORED | PCRE2_NOTEMPTY_ATSTART`, and only on failure
//! is the start advanced by one code unit (one byte, or one UTF-8 character
//! under `u`).

use std::collections::HashMap;
use std::rc::Rc;

use rhp_semantics::deref;
use rhp_values::{ArrayKey, PhpArray, PhpStr, Value};

use crate::pcre::{self, sys::*, JitIssue, Ovector, Regex};
use crate::{Builtin, Host, Thrown};

// `preg_last_error()` codes.
const PREG_NO_ERROR: i64 = 0;
const PREG_INTERNAL_ERROR: i64 = 1;
const PREG_BACKTRACK_LIMIT_ERROR: i64 = 2;
const PREG_RECURSION_LIMIT_ERROR: i64 = 3;
const PREG_BAD_UTF8_ERROR: i64 = 4;
const PREG_BAD_UTF8_OFFSET_ERROR: i64 = 5;
const PREG_JIT_STACKLIMIT_ERROR: i64 = 6;

thread_local! {
    /// `PCRE_G(error_code)`: reset at the start of every regex operation.
    static LAST_ERROR: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
}

pub(crate) fn reset_state() {
    set_last_error(0);
    pcre::reset();
}

fn set_last_error(code: i64) {
    LAST_ERROR.with(|c| c.set(code));
}

/// `pcre_handle_exec_error`: map a PCRE2 match error code to php's.
fn set_exec_error(rc: i32) {
    let code = match rc {
        PCRE2_ERROR_MATCHLIMIT => PREG_BACKTRACK_LIMIT_ERROR,
        PCRE2_ERROR_DEPTHLIMIT => PREG_RECURSION_LIMIT_ERROR,
        PCRE2_ERROR_BADUTFOFFSET => PREG_BAD_UTF8_OFFSET_ERROR,
        PCRE2_ERROR_JIT_STACKLIMIT => PREG_JIT_STACKLIMIT_ERROR,
        rc if (PCRE2_ERROR_UTF8_ERR21..=PCRE2_ERROR_UTF8_ERR1).contains(&rc) => PREG_BAD_UTF8_ERROR,
        _ => PREG_INTERNAL_ERROR,
    };
    set_last_error(code);
}

// PCRE flag constants (mirrored into lib.rs `constants()`; see `preg_constants`).
const PREG_PATTERN_ORDER: i64 = 1;
const PREG_SET_ORDER: i64 = 2;
const PREG_OFFSET_CAPTURE: i64 = 256;
const PREG_SPLIT_NO_EMPTY: i64 = 1;
const PREG_SPLIT_DELIM_CAPTURE: i64 = 2;
const PREG_SPLIT_OFFSET_CAPTURE: i64 = 4;
const PREG_UNMATCHED_AS_NULL: i64 = 512;
const PREG_GREP_INVERT: i64 = 1;

/// One match from `start`; a PCRE2 error sets preg_last_error() and is `Err`.
fn find_at(re: &Regex, subject: &[u8], start: usize) -> Result<Option<Ovector>, ()> {
    re.find_at(subject, start).map_err(set_exec_error)
}

// ---------------------------------------------------------------------------
// small shared helpers
// ---------------------------------------------------------------------------

/// `(string)` bytes of a value (after deref).
fn bytes_of(v: &Value) -> Vec<u8> {
    rhp_values::conv::to_string_bytes(&deref(v))
}

/// An array entry or a callback's result as a string (zval_get_string): an
/// array warns, an object goes through `__toString` or throws.
fn entry_bytes(host: &mut dyn Host, v: &Value) -> Result<Vec<u8>, Thrown> {
    crate::argparse::zval_get_string(host, v)
}

/// `(int)` of a value (after deref). Default when the arg is absent.
fn int_arg(args: &[Value], i: usize, default: i64) -> i64 {
    args.get(i)
        .map(|v| rhp_values::conv::to_int(&deref(v)))
        .unwrap_or(default)
}

// ---------------------------------------------------------------------------
// pattern parsing: `/body/flags`, `#body#flags`, `(body)flags`, …
// ---------------------------------------------------------------------------

/// pcre_get_compiled_regex_cache: a cached pattern, or parse and compile it.
/// A failed compile or a JIT problem warns from builtin `fname` right away,
/// as php does; `None` means the pattern did not compile.
pub(crate) fn compile_warn(
    host: &mut dyn Host,
    fname: &str,
    pattern: &[u8],
) -> Result<Option<Rc<Regex>>, Thrown> {
    set_last_error(PREG_NO_ERROR);
    if let Some(re) = pcre::cache_get(pattern) {
        return Ok(Some(re));
    }
    let mut warnings = Vec::new();
    let mut jit_issue = None;
    let re = compile_inner(pattern, &mut jit_issue);
    // A JIT failure also sets PREG_INTERNAL_ERROR in php, but every preg_*
    // caller resets the code before matching.
    warnings.extend(jit_issue.map(jit_warning));
    let re = match re {
        Ok(re) => {
            let re = Rc::new(re);
            pcre::cache_put(pattern.to_vec(), re.clone());
            Some(re)
        }
        Err(m) => {
            warnings.push(m);
            set_last_error(PREG_INTERNAL_ERROR);
            None
        }
    };
    let err = LAST_ERROR.with(|c| c.get());
    for m in warnings {
        crate::warn(host, fname, &m)?;
    }
    // A warning handler may have run preg functions of its own.
    set_last_error(err);
    Ok(re)
}

/// php's warning for a failed pcre2_jit_compile.
fn jit_warning(issue: JitIssue) -> String {
    match issue {
        JitIssue::NoMemory => "Allocation of JIT memory failed, PCRE JIT will be disabled. \
             This is likely caused by security restrictions. \
             Either grant PHP permission to allocate executable memory, or set pcre.jit=0"
            .to_string(),
        JitIssue::Failed(m) => format!("JIT compilation failed: {m}"),
    }
}

/// Match `subject` against a fixed `/body/flags` pattern the way an extension
/// does through pcre_get_compiled_regex + pcre2_match (filter's email check):
/// the shared pattern cache and match limits, and an error counts as no
/// match. `preg_last_error()` is not reset. The one diagnostic is php's JIT
/// warning (from builtin `fname`) when the first compile's JIT step fails.
pub(crate) fn fixed_pattern_matches(
    host: &mut dyn Host,
    fname: &str,
    pattern: &'static [u8],
    subject: &[u8],
) -> Result<bool, Thrown> {
    let re = match pcre::cache_get(pattern) {
        Some(re) => re,
        None => {
            let mut jit_issue = None;
            let re =
                Rc::new(compile_inner(pattern, &mut jit_issue).expect("fixed pattern compiles"));
            pcre::cache_put(pattern.to_vec(), re.clone());
            if let Some(issue) = jit_issue {
                let internal = matches!(issue, JitIssue::Failed(_));
                let warned = crate::warn(host, fname, &jit_warning(issue));
                // php_pcre.c: pcre_handle_exec_error(PCRE2_ERROR_INTERNAL),
                // which nothing on this path resets.
                if internal {
                    set_last_error(PREG_INTERNAL_ERROR);
                }
                warned?;
            }
            re
        }
    };
    Ok(matches!(re.find_at(subject, 0), Ok(Some(_))))
}

fn compile_inner(pattern: &[u8], jit_issue: &mut Option<JitIssue>) -> Result<Regex, String> {
    // Leading whitespace is skipped; the next byte is the delimiter.
    let start = pattern
        .iter()
        .position(|b| !b" \t\n\r\x0b\x0c".contains(b))
        .ok_or_else(|| "Empty regular expression".to_string())?;
    let delim = pattern[start];
    if delim.is_ascii_alphanumeric() || delim == b'\\' || delim == 0 {
        return Err("Delimiter must not be alphanumeric, backslash, or NUL byte".into());
    }
    let close = match delim {
        b'(' => b')',
        b'{' => b'}',
        b'[' => b']',
        b'<' => b'>',
        d => d,
    };
    let bracketed = delim != close;
    let body_start = start + 1;
    let mut i = body_start;
    let mut depth = 1usize;
    let close_pos = loop {
        if i >= pattern.len() {
            return Err(if bracketed {
                format!("No ending matching delimiter '{}' found", close as char)
            } else {
                format!("No ending delimiter '{}' found", close as char)
            });
        }
        let b = pattern[i];
        if b == b'\\' && i + 1 < pattern.len() {
            i += 2;
            continue;
        }
        if b == close {
            depth -= 1;
            if !bracketed || depth == 0 {
                break i;
            }
        } else if bracketed && b == delim {
            depth += 1;
        }
        i += 1;
    };

    let body = &pattern[body_start..close_pos];
    let flag_bytes = &pattern[close_pos + 1..];

    // Translate trailing modifier letters into PCRE2 compile options.
    let mut options: u32 = 0;
    let mut extra: u32 = 0;
    for &f in flag_bytes {
        match f {
            b'i' => options |= PCRE2_CASELESS,
            b'm' => options |= PCRE2_MULTILINE,
            b'n' => options |= PCRE2_NO_AUTO_CAPTURE,
            b's' => options |= PCRE2_DOTALL,
            b'x' => options |= PCRE2_EXTENDED,
            b'A' => options |= PCRE2_ANCHORED,
            b'D' => options |= PCRE2_DOLLAR_ENDONLY,
            b'r' if pcre::HAVE_CASELESS_RESTRICT => extra |= PCRE2_EXTRA_CASELESS_RESTRICT,
            b'U' => options |= PCRE2_UNGREEDY,
            b'J' => options |= PCRE2_DUPNAMES,
            // `u`: UTF mode + Unicode properties (UCP is what makes \d\w\s match
            // non-ASCII, exactly as php's `u` modifier does).
            // \C is unsafe in UTF mode: php forbids it (GH-21134).
            b'u' => options |= PCRE2_UTF | PCRE2_UCP | PCRE2_NEVER_BACKSLASH_C,
            // php accepts these for backward compatibility; both are no-ops here.
            b'S' | b'X' | b' ' | b'\n' | b'\r' => {}
            0 => return Err("NUL byte is not a valid modifier".into()),
            c => return Err(format!("Unknown modifier '{}'", c as char)),
        }
    }

    Regex::compile(body, options, extra, jit_issue).map_err(|e| {
        // php 8.5.10+ (GH-21134) words the \C-under-`u` error itself.
        let msg = if e.code == PCRE2_ERROR_BACKSLASH_C_CALLER_DISABLED {
            "using \\C is incompatible with the 'u' modifier".to_string()
        } else {
            e.message
        };
        format!("Compilation failed: {msg} at offset {}", e.offset)
    })
}

// ---------------------------------------------------------------------------
// building the `$matches` array for one match
// ---------------------------------------------------------------------------

/// The highest group index that participated in this match (group 0 always
/// does). php trims *trailing* unmatched groups from `$matches`.
fn last_participating(ov: &Ovector) -> usize {
    let mut last = 0;
    for (i, g) in ov.iter().enumerate() {
        if g.is_some() {
            last = i;
        }
    }
    last
}

/// The bytes of group `i`, or `None` if it did not participate.
fn group_bytes<'a>(ov: &Ovector, subject: &'a [u8], i: usize) -> Option<&'a [u8]> {
    ov.get(i).and_then(|o| *o).map(|(s, e)| &subject[s..e])
}

/// The value for one capture group in the result array. Honours
/// `PREG_OFFSET_CAPTURE` (wrap as `[text, offset]`) and `PREG_UNMATCHED_AS_NULL`.
fn group_value(
    ov: &Ovector,
    subject: &[u8],
    i: usize,
    offset_capture: bool,
    unmatched_null: bool,
) -> Value {
    match ov.get(i).and_then(|o| *o) {
        Some((s, e)) => {
            let val = Value::str(subject[s..e].to_vec());
            if offset_capture {
                pair(val, s as i64)
            } else {
                val
            }
        }
        None => {
            let empty = if unmatched_null {
                Value::Null
            } else {
                Value::str("")
            };
            if offset_capture {
                pair(empty, -1)
            } else {
                empty
            }
        }
    }
}

/// A two-element `[value, offset]` array, as `PREG_OFFSET_CAPTURE` produces.
fn pair(value: Value, offset: i64) -> Value {
    let mut a = PhpArray::new();
    a.push(value);
    a.push(Value::Int(offset));
    Value::Array(a)
}

/// Build a single match's `$matches` array: group 0, then each group (named
/// groups appear *by name first, then by index*, exactly as php orders them).
fn build_match(
    ov: &Ovector,
    subject: &[u8],
    names: &[Option<String>],
    offset_capture: bool,
    unmatched_null: bool,
) -> PhpArray {
    // php trims trailing unmatched groups, unless PREG_UNMATCHED_AS_NULL.
    let last = if unmatched_null {
        ov.len() - 1
    } else {
        last_participating(ov)
    };
    let mut arr = PhpArray::new();
    for i in 0..=last {
        let val = group_value(ov, subject, i, offset_capture, unmatched_null);
        if let Some(name) = names.get(i).and_then(|o| o.as_ref()) {
            // add_named: under (?J) several groups share a name; an unset
            // one never replaces a set one.
            let key = ArrayKey::Str(PhpStr::from(name.as_str()));
            let unset = ov.get(i).and_then(|o| *o).is_none();
            if !unset || arr.get(&key).is_none() {
                arr.insert(key, val.clone());
            }
        }
        arr.insert(ArrayKey::Int(i as i64), val);
    }
    if let Some(m) = &ov.mark {
        arr.insert(ArrayKey::Str(PhpStr::from("MARK")), Value::str(m.clone()));
    }
    arr
}

/// Write an out-value through a by-reference `$matches`/`$count` argument.
/// Mirrors `sort_fns`: the compiler passes arg 2 as a `Value::Ref`; if it is
/// not a ref (the optional out-param was omitted), this is a no-op.
fn write_ref(arg: Option<&Value>, value: Value) {
    if let Some(Value::Ref(rc)) = arg {
        *rc.borrow_mut() = value;
    }
}

/// php_do_pcre_match after the compile, up to the first match attempt: the
/// offset check, `$matches = []`, the flags check, then the start offset (a
/// negative one counts from the end, clamped to 0; one past the subject
/// fails with PREG_INTERNAL_ERROR). `Ok(None)` means return false.
fn match_setup(
    f: &str,
    global: bool,
    subject: &[u8],
    flags: i64,
    offset: i64,
    matches: Option<&Value>,
) -> Result<Option<usize>, Thrown> {
    if offset == i64::MIN {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{f}(): Argument #5 ($offset) must be greater than {}",
                i64::MIN
            ),
        ));
    }
    write_ref(matches, Value::Array(PhpArray::new()));
    let order = flags & 0xff;
    if order != 0 && (!global || !(PREG_PATTERN_ORDER..=PREG_SET_ORDER).contains(&order)) {
        return Err(Thrown::error(
            "ValueError",
            format!("{f}(): Argument #4 ($flags) must be a PREG_* constant"),
        ));
    }
    let len = subject.len() as i64;
    let start = if offset < 0 {
        if offset.unsigned_abs() <= len as u64 {
            len + offset
        } else {
            0
        }
    } else {
        offset
    };
    if start > len {
        set_last_error(PREG_INTERNAL_ERROR);
        return Ok(None);
    }
    Ok(Some(start as usize))
}

// ---------------------------------------------------------------------------
// preg_match
// ---------------------------------------------------------------------------

fn fn_preg_match(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(re) = compile_warn(host, "preg_match", &bytes_of(&args[0]))? else {
        return Ok(Value::Bool(false));
    };
    let subject = bytes_of(&args[1]);
    let flags = int_arg(args, 3, 0);
    let offset = int_arg(args, 4, 0);
    let Some(offset) = match_setup("preg_match", false, &subject, flags, offset, args.get(2))?
    else {
        return Ok(Value::Bool(false));
    };
    let offset_capture = flags & PREG_OFFSET_CAPTURE != 0;
    let unmatched_null = flags & PREG_UNMATCHED_AS_NULL != 0;

    match find_at(&re, &subject, offset) {
        Ok(Some(ov)) => {
            let arr = build_match(&ov, &subject, re.names(), offset_capture, unmatched_null);
            write_ref(args.get(2), Value::Array(arr));
            Ok(Value::Int(1))
        }
        Ok(None) => {
            write_ref(args.get(2), Value::Array(PhpArray::new()));
            Ok(Value::Int(0))
        }
        Err(()) => Ok(Value::Bool(false)),
    }
}

// ---------------------------------------------------------------------------
// preg_match_all
// ---------------------------------------------------------------------------

fn fn_preg_match_all(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(re) = compile_warn(host, "preg_match_all", &bytes_of(&args[0]))? else {
        return Ok(Value::Bool(false));
    };
    let subject = bytes_of(&args[1]);
    let flags = int_arg(args, 3, 0);
    let offset = int_arg(args, 4, 0);
    let Some(offset) = match_setup("preg_match_all", true, &subject, flags, offset, args.get(2))?
    else {
        return Ok(Value::Bool(false));
    };
    // The ordering flag lives in the low bits; default is PREG_PATTERN_ORDER.
    let set_order = flags & 0xff == PREG_SET_ORDER;
    let offset_capture = flags & PREG_OFFSET_CAPTURE != 0;
    let unmatched_null = flags & PREG_UNMATCHED_AS_NULL != 0;
    let names = re.names().to_vec();
    let ngroups = re.ncaptures() + 1;

    // On an error php still fills `$matches` with what it found before it,
    // then returns false.
    let mut all = Vec::new();
    let ok = re
        .find_all(&subject, offset, usize::MAX, &mut all)
        .map_err(set_exec_error)
        .is_ok();

    let count = all.len();
    let result = if set_order {
        // SET_ORDER: an array of per-match arrays (each like preg_match's).
        let mut outer = PhpArray::new();
        for ov in &all {
            outer.push(Value::Array(build_match(
                ov,
                &subject,
                &names,
                offset_capture,
                unmatched_null,
            )));
        }
        outer
    } else {
        // PATTERN_ORDER: one array per group (named group by name then index).
        // All groups are included (no trailing trim), even with zero matches.
        let mut outer = PhpArray::new();
        for i in 0..ngroups {
            let mut col = PhpArray::new();
            for ov in &all {
                col.push(group_value(ov, &subject, i, offset_capture, unmatched_null));
            }
            if let Some(name) = names.get(i).and_then(|o| o.as_ref()) {
                outer.insert(
                    ArrayKey::Str(PhpStr::from(name.as_str())),
                    Value::Array(col.clone()),
                );
            }
            outer.insert(ArrayKey::Int(i as i64), Value::Array(col));
        }
        // `$matches['MARK'][$n]` for each match that passed a mark.
        let mut marks = PhpArray::new();
        for (n, ov) in all.iter().enumerate() {
            if let Some(m) = &ov.mark {
                marks.insert(ArrayKey::Int(n as i64), Value::str(m.clone()));
            }
        }
        if !marks.is_empty() {
            outer.insert(ArrayKey::Str(PhpStr::from("MARK")), Value::Array(marks));
        }
        outer
    };

    write_ref(args.get(2), Value::Array(result));
    Ok(if ok {
        Value::Int(count as i64)
    } else {
        Value::Bool(false)
    })
}

// ---------------------------------------------------------------------------
// replacement template expansion (`$1`, `\1`, `${1}`)
// ---------------------------------------------------------------------------

/// Expand one replacement template against a match, appending bytes to `out`.
///
/// php backreference rules (verified against php 8.5.8):
/// - `$1`, `\1` → group 1; `$12`/`\12` read *up to two* digits (so `$12` is
///   group 12, not group 1 then `'2'`); an out-of-range group yields "".
/// - `${12}` reads all digits inside the braces.
/// - `\$` → literal `$`; `\\` → literal `\`; `\` + other → kept verbatim
///   (`\n` stays `\n`). `$` not followed by a digit or `{` is a literal `$`.
fn expand<'a>(template: &[u8], group: impl Fn(usize) -> &'a [u8], out: &mut Vec<u8>) {
    let b = template;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'$' => {
                if i + 1 < b.len() && b[i + 1] == b'{' {
                    // ${ digits }
                    let mut j = i + 2;
                    let ds = j;
                    while j < b.len() && b[j].is_ascii_digit() {
                        j += 1;
                    }
                    if j > ds && j < b.len() && b[j] == b'}' {
                        let n: usize = std::str::from_utf8(&b[ds..j])
                            .unwrap()
                            .parse()
                            .unwrap_or(usize::MAX);
                        out.extend_from_slice(group(n));
                        i = j + 1;
                    } else {
                        out.push(b'$');
                        i += 1;
                    }
                } else if let Some(n) = read_num(b, i + 1) {
                    out.extend_from_slice(group(n.0));
                    i = n.1;
                } else {
                    out.push(b'$');
                    i += 1;
                }
            }
            b'\\' => {
                if i + 1 >= b.len() {
                    out.push(b'\\');
                    i += 1;
                } else if let Some(n) = read_num(b, i + 1) {
                    out.extend_from_slice(group(n.0));
                    i = n.1;
                } else if b[i + 1] == b'\\' {
                    out.push(b'\\');
                    i += 2;
                } else if b[i + 1] == b'$' {
                    out.push(b'$');
                    i += 2;
                } else {
                    out.push(b'\\');
                    out.push(b[i + 1]);
                    i += 2;
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
}

/// Read up to two decimal digits starting at `start`; returns (group, next_idx).
fn read_num(b: &[u8], start: usize) -> Option<(usize, usize)> {
    if start >= b.len() || !b[start].is_ascii_digit() {
        return None;
    }
    let mut j = start + 1;
    if j < b.len() && b[j].is_ascii_digit() {
        j += 1; // at most two digits
    }
    let n: usize = std::str::from_utf8(&b[start..j]).unwrap().parse().ok()?;
    Some((n, j))
}

// ---------------------------------------------------------------------------
// the core replace driver (shared by preg_replace / preg_replace_callback)
// ---------------------------------------------------------------------------

/// Replace matches in `subject`, producing bytes. `make_rep` appends the
/// replacement for each match as it is found (template expansion or a
/// callback) and returns false to stop. Returns `(output, replacement_count)`.
/// `limit < 0` means unlimited. `Err(())` on a PCRE2 error, or when
/// `make_rep` stopped (PREG_INTERNAL_ERROR, as php's error path sets for a
/// match count); either sets preg_last_error().
fn replace_driver<F>(
    re: &Regex,
    subject: &[u8],
    limit: i64,
    mut make_rep: F,
) -> Result<(Vec<u8>, usize), ()>
where
    F: FnMut(&Ovector, &mut Vec<u8>) -> bool,
{
    let max = usize::try_from(limit).unwrap_or(usize::MAX);
    let mut out: Vec<u8> = Vec::with_capacity(subject.len());
    let mut last = 0usize;
    let mut n = 0usize;
    if max > 0 {
        for m in re.matches(subject, 0) {
            let ov = m.map_err(set_exec_error)?;
            let (s, e) = ov[0].unwrap();
            out.extend_from_slice(&subject[last..s]);
            if !make_rep(&ov, &mut out) {
                set_last_error(PREG_INTERNAL_ERROR);
                return Err(());
            }
            last = e;
            n += 1;
            if n == max {
                break;
            }
        }
    }
    out.extend_from_slice(&subject[last.min(subject.len())..]);
    Ok((out, n))
}

// ---------------------------------------------------------------------------
// preg_replace
// ---------------------------------------------------------------------------

/// Run every (pattern, replacement) pair over a single subject in sequence.
/// `pats`/`reps` are parallel lists; a missing replacement means "".
/// `Ok(None)` is a compile or match failure.
fn replace_subject(
    host: &mut dyn Host,
    fname: &str,
    pats: &[Vec<u8>],
    reps: &[Vec<u8>],
    subject: &[u8],
    limit: i64,
    count: &mut usize,
) -> Result<Option<Vec<u8>>, Thrown> {
    let mut cur = subject.to_vec();
    for (i, pat) in pats.iter().enumerate() {
        let rep = reps.get(i).map(|v| v.as_slice()).unwrap_or(b"");
        let Some(re) = compile_warn(host, fname, pat)? else {
            return Ok(None);
        };
        let res = replace_driver(&re, &cur, limit, |ov, out| {
            expand(rep, |g| group_bytes(ov, &cur, g).unwrap_or(b""), out);
            true
        });
        let Ok((next, n)) = res else {
            return Ok(None);
        };
        *count += n;
        cur = next;
    }
    Ok(Some(cur))
}

/// Collect a pattern/replacement argument into a list of byte vectors. A scalar
/// becomes a one-element list; an array keeps its values in order.
fn as_list(v: &Value) -> (Vec<Vec<u8>>, bool) {
    match deref(v) {
        Value::Array(a) => (a.iter().map(|(_, x)| bytes_of(x)).collect(), true),
        other => (vec![bytes_of(&other)], false),
    }
}

/// _preg_replace_common: preg_replace, or preg_filter (`filter`), which keeps
/// only the subjects something was replaced in. A subject whose replacement
/// fails is left out of an array result, and makes a string result null.
fn preg_replace_common(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    filter: bool,
) -> Result<Value, Thrown> {
    let (pats, pat_is_arr) = as_list(&args[0]);
    let (reps, rep_is_arr) = as_list(&args[1]);
    if rep_is_arr && !pat_is_arr {
        return Err(Thrown::type_error(format!(
            "{fname}(): Argument #1 ($pattern) must be of type array when argument #2 ($replacement) is an array, string given"
        )));
    }
    let limit = int_arg(args, 3, -1);
    let mut count = 0usize;

    // A scalar replacement applies to every pattern.
    let reps: Vec<Vec<u8>> = if rep_is_arr {
        reps
    } else {
        vec![reps.into_iter().next().unwrap_or_default(); pats.len()]
    };

    let result = match deref(&args[2]) {
        Value::Array(subjects) => {
            // subject is an array: return an array with the same keys.
            let mut out = PhpArray::new();
            for (k, sv) in subjects.iter() {
                let before = count;
                let sb = entry_bytes(host, sv)?;
                if let Some(r) = replace_subject(host, fname, &pats, &reps, &sb, limit, &mut count)?
                {
                    if !filter || count > before {
                        out.insert(k.clone(), Value::str(r));
                    }
                }
            }
            Value::Array(out)
        }
        other => {
            let sb = bytes_of(&other);
            match replace_subject(host, fname, &pats, &reps, &sb, limit, &mut count)? {
                Some(r) if !filter || count > 0 => Value::str(r),
                _ => Value::Null,
            }
        }
    };

    write_ref(args.get(4), Value::Int(count as i64));
    Ok(result)
}

fn fn_preg_replace(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    preg_replace_common(host, args, "preg_replace", false)
}

fn fn_preg_filter(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    preg_replace_common(host, args, "preg_filter", true)
}

// ---------------------------------------------------------------------------
// preg_replace_callback
// ---------------------------------------------------------------------------

/// Apply `(pattern, callback)` pairs in order to one string subject
/// (`php_replace_in_subject_func`). `Ok(None)` = a compile/match failure.
fn cb_string(
    host: &mut dyn Host,
    fname: &str,
    pairs: &[(Vec<u8>, Value)],
    subject: &[u8],
    limit: i64,
    flags: i64,
    count: &mut usize,
) -> Result<Option<Vec<u8>>, Thrown> {
    let offset_capture = flags & PREG_OFFSET_CAPTURE != 0;
    let unmatched_null = flags & PREG_UNMATCHED_AS_NULL != 0;
    let mut cur = subject.to_vec();
    for (pat, cb) in pairs {
        let Some(re) = compile_warn(host, fname, pat)? else {
            return Ok(None);
        };
        let names = re.names().to_vec();
        // An exception from the callback, or from converting its result
        // (preg_do_repl_func), stops matching and propagates.
        let mut pending: Result<(), Thrown> = Ok(());
        let src = cur.clone();
        let res = replace_driver(&re, &src, limit, |ov, out| {
            let m = build_match(ov, &src, &names, offset_capture, unmatched_null);
            let rep = host
                .call_callable(cb, &[Value::Array(m)])
                .and_then(|v| entry_bytes(host, &v));
            match rep {
                Ok(b) => {
                    out.extend_from_slice(&b);
                    true
                }
                Err(e) => {
                    pending = Err(e);
                    false
                }
            }
        });
        pending?;
        let Ok((bytes, n)) = res else {
            return Ok(None);
        };
        *count += n;
        cur = bytes;
    }
    Ok(Some(cur))
}

/// Apply `pairs` to a string-or-array subject. A failing array entry is left
/// out of the result (php); a failing string subject yields `None`.
fn cb_subject(
    host: &mut dyn Host,
    fname: &str,
    pairs: &[(Vec<u8>, Value)],
    subject: Value,
    limit: i64,
    flags: i64,
    count: &mut usize,
) -> Result<Option<Value>, Thrown> {
    match subject {
        Value::Array(subjects) => {
            let mut out = PhpArray::new();
            for (k, sv) in subjects.iter() {
                let sb = entry_bytes(host, sv)?;
                if let Some(r) = cb_string(host, fname, pairs, &sb, limit, flags, count)? {
                    out.insert(k.clone(), Value::str(r));
                }
            }
            Ok(Some(Value::Array(out)))
        }
        other => {
            let sb = bytes_of(&other);
            Ok(cb_string(host, fname, pairs, &sb, limit, flags, count)?.map(Value::str))
        }
    }
}

fn fn_preg_replace_callback(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (pats, _) = as_list(&args[0]);
    let cb = deref(&args[1]);
    let limit = int_arg(args, 3, -1);
    let flags = int_arg(args, 5, 0);
    let pairs: Vec<(Vec<u8>, Value)> = pats.into_iter().map(|p| (p, cb.clone())).collect();
    let mut count = 0usize;
    let result = cb_subject(
        host,
        "preg_replace_callback",
        &pairs,
        deref(&args[2]),
        limit,
        flags,
        &mut count,
    )?;
    write_ref(args.get(4), Value::Int(count as i64));
    Ok(result.unwrap_or(Value::Null))
}

/// `preg_replace_callback_array($pattern, $subject, $limit, &$count, $flags)`:
/// each `pattern => callback` pair is applied in turn to the whole subject.
fn fn_preg_replace_callback_array(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let map = match deref(&args[0]) {
        Value::Array(a) => a,
        other => {
            return Err(Thrown::type_error(format!(
            "preg_replace_callback_array(): Argument #1 ($pattern) must be of type array, {} given",
            crate::argparse::value_name(host, &other)
        )))
        }
    };
    let limit = int_arg(args, 2, -1);
    let flags = int_arg(args, 4, 0);
    let mut count = 0usize;
    let mut cur = match deref(&args[1]) {
        Value::Array(a) => Value::Array(a),
        other => Value::str(bytes_of(&other)),
    };
    for (k, cb) in map.iter() {
        let ArrayKey::Str(pat) = k else {
            return Err(Thrown::type_error(
                "preg_replace_callback_array(): Argument #1 ($pattern) must contain only string patterns as keys",
            ));
        };
        let cb = deref(cb);
        if !host.is_callable_value(&cb) {
            return Err(Thrown::type_error("preg_replace_callback_array(): Argument #1 ($pattern) must contain only valid callbacks"));
        }
        let pairs = [(pat.as_bytes().to_vec(), cb)];
        let fname = "preg_replace_callback_array";
        match cb_subject(host, fname, &pairs, cur, limit, flags, &mut count)? {
            Some(v) => cur = v,
            None => return Ok(Value::Null),
        }
    }
    write_ref(args.get(3), Value::Int(count as i64));
    Ok(cur)
}

fn fn_preg_last_error(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(LAST_ERROR.with(|c| c.get())))
}

fn fn_preg_last_error_msg(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let msg = match LAST_ERROR.with(|c| c.get()) {
        PREG_NO_ERROR => "No error",
        PREG_INTERNAL_ERROR => "Internal error",
        PREG_BAD_UTF8_ERROR => "Malformed UTF-8 characters, possibly incorrectly encoded",
        PREG_BAD_UTF8_OFFSET_ERROR => {
            "The offset did not correspond to the beginning of a valid UTF-8 code point"
        }
        PREG_BACKTRACK_LIMIT_ERROR => "Backtrack limit exhausted",
        PREG_RECURSION_LIMIT_ERROR => "Recursion limit exhausted",
        PREG_JIT_STACKLIMIT_ERROR => "JIT stack limit exhausted",
        _ => "Unknown error",
    };
    Ok(Value::str(msg))
}

// ---------------------------------------------------------------------------
// preg_split
// ---------------------------------------------------------------------------

fn fn_preg_split(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(re) = compile_warn(host, "preg_split", &bytes_of(&args[0]))? else {
        return Ok(Value::Bool(false));
    };
    let subject = bytes_of(&args[1]);
    let flags = int_arg(args, 3, 0);
    let no_empty = flags & PREG_SPLIT_NO_EMPTY != 0;
    let delim_capture = flags & PREG_SPLIT_DELIM_CAPTURE != 0;
    let offset_capture = flags & PREG_SPLIT_OFFSET_CAPTURE != 0;
    // php_pcre_split_impl: -1 and 0 mean no limit; any other limit below 2
    // returns the whole subject. Pieces left out by NO_EMPTY don't count.
    let mut limit = match int_arg(args, 2, -1) {
        0 => -1,
        l => l,
    };

    let mut out = PhpArray::new();
    // One piece; `None` offsets are an unset group's (offset -1).
    let push = |out: &mut PhpArray, piece: &[u8], off: Option<usize>| {
        let v = Value::str(piece.to_vec());
        out.push(if offset_capture {
            pair(v, off.map_or(-1, |o| o as i64))
        } else {
            v
        });
    };

    let mut last = 0usize; // end of the last match: where the next piece starts
    if limit == -1 || limit > 1 {
        for m in re.matches(&subject, 0) {
            let Ok(ov) = m.map_err(set_exec_error) else {
                return Ok(Value::Bool(false));
            };
            let (s, e) = ov[0].unwrap();
            if e < s {
                // \K in a lookahead made the match end before it starts.
                set_last_error(PREG_INTERNAL_ERROR);
                return Ok(Value::Bool(false));
            }
            if !no_empty || s != last {
                push(&mut out, &subject[last..s], Some(last));
                if limit != -1 {
                    limit -= 1;
                }
            }
            if delim_capture {
                for g in ov.iter().take(last_participating(&ov) + 1).skip(1) {
                    match *g {
                        Some((gs, ge)) if !no_empty || gs != ge => {
                            push(&mut out, &subject[gs..ge], Some(gs))
                        }
                        None if !no_empty => push(&mut out, b"", None),
                        _ => {}
                    }
                }
            }
            last = e;
            if limit != -1 && limit <= 1 {
                break;
            }
        }
    }
    if !no_empty || last < subject.len() {
        push(&mut out, &subject[last..], Some(last));
    }

    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// preg_quote
// ---------------------------------------------------------------------------

/// Escape the PCRE metacharacters (and an optional delimiter). NUL becomes the
/// literal escape `\000`; every metacharacter is prefixed with a backslash.
fn fn_preg_quote(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    // delimiter: first byte of the second argument, if any.
    let delim = args
        .get(1)
        .map(bytes_of)
        .filter(|d| !d.is_empty())
        .map(|d| d[0]);
    let mut out = Vec::with_capacity(s.len() + s.len() / 2);
    for &b in &s {
        match b {
            0 => out.extend_from_slice(b"\\000"),
            b'.' | b'\\' | b'+' | b'*' | b'?' | b'[' | b'^' | b']' | b'$' | b'(' | b')' | b'{'
            | b'}' | b'=' | b'!' | b'<' | b'>' | b'|' | b':' | b'-' | b'#' => {
                out.push(b'\\');
                out.push(b);
            }
            other => {
                if Some(other) == delim {
                    out.push(b'\\');
                }
                out.push(other);
            }
        }
    }
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// preg_grep
// ---------------------------------------------------------------------------

fn fn_preg_grep(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(re) = compile_warn(host, "preg_grep", &bytes_of(&args[0]))? else {
        return Ok(Value::Bool(false));
    };
    let Value::Array(input) = deref(&args[1]) else {
        return Err(Thrown::type_error(
            "preg_grep(): Argument #2 ($array) must be of type array",
        ));
    };
    let invert = int_arg(args, 2, 0) & PREG_GREP_INVERT != 0;
    let mut out = PhpArray::new();
    for (k, v) in input.iter() {
        let b = entry_bytes(host, v)?;
        // A match error stops the scan; the entries kept so far are the
        // result (php_pcre_grep_impl).
        let Ok(found) = find_at(&re, &b, 0) else {
            break;
        };
        let matched = found.is_some();
        if matched != invert {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// registration + constants
// ---------------------------------------------------------------------------

/// The pcre extension's constants, for lib.rs `constants()`. The `PCRE_*`
/// ones describe the linked PCRE2 as php_pcre.c does: `PCRE_VERSION` from
/// pcre2_config at run time, MAJOR/MINOR from the header.
pub fn preg_constants() -> Vec<(&'static str, Value)> {
    vec![
        ("PREG_PATTERN_ORDER", Value::Int(PREG_PATTERN_ORDER)),
        ("PREG_SET_ORDER", Value::Int(PREG_SET_ORDER)),
        ("PREG_OFFSET_CAPTURE", Value::Int(PREG_OFFSET_CAPTURE)),
        ("PREG_SPLIT_NO_EMPTY", Value::Int(PREG_SPLIT_NO_EMPTY)),
        (
            "PREG_SPLIT_DELIM_CAPTURE",
            Value::Int(PREG_SPLIT_DELIM_CAPTURE),
        ),
        (
            "PREG_SPLIT_OFFSET_CAPTURE",
            Value::Int(PREG_SPLIT_OFFSET_CAPTURE),
        ),
        ("PREG_UNMATCHED_AS_NULL", Value::Int(PREG_UNMATCHED_AS_NULL)),
        ("PREG_GREP_INVERT", Value::Int(PREG_GREP_INVERT)),
        ("PREG_NO_ERROR", Value::Int(PREG_NO_ERROR)),
        ("PREG_INTERNAL_ERROR", Value::Int(PREG_INTERNAL_ERROR)),
        (
            "PREG_BACKTRACK_LIMIT_ERROR",
            Value::Int(PREG_BACKTRACK_LIMIT_ERROR),
        ),
        (
            "PREG_RECURSION_LIMIT_ERROR",
            Value::Int(PREG_RECURSION_LIMIT_ERROR),
        ),
        ("PREG_BAD_UTF8_ERROR", Value::Int(PREG_BAD_UTF8_ERROR)),
        (
            "PREG_BAD_UTF8_OFFSET_ERROR",
            Value::Int(PREG_BAD_UTF8_OFFSET_ERROR),
        ),
        (
            "PREG_JIT_STACKLIMIT_ERROR",
            Value::Int(PREG_JIT_STACKLIMIT_ERROR),
        ),
        ("PCRE_VERSION", Value::str(pcre::version())),
        ("PCRE_VERSION_MAJOR", Value::Int(pcre::MAJOR)),
        ("PCRE_VERSION_MINOR", Value::Int(pcre::MINOR)),
        ("PCRE_JIT_SUPPORT", Value::Bool(pcre::jit_supported())),
    ]
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
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
    // `&$matches` / `&$count` are by-reference OUT params (arg 2 for the match
    // functions, the final arg for the replace functions).
    put(m, "preg_match", fn_preg_match, 2, Some(5));
    put(m, "preg_match_all", fn_preg_match_all, 2, Some(5));
    put(m, "preg_replace", fn_preg_replace, 3, Some(5));
    put(m, "preg_filter", fn_preg_filter, 3, Some(5));
    put(
        m,
        "preg_replace_callback",
        fn_preg_replace_callback,
        3,
        Some(6),
    );
    put(
        m,
        "preg_replace_callback_array",
        fn_preg_replace_callback_array,
        2,
        Some(5),
    );
    put(m, "preg_last_error", fn_preg_last_error, 0, Some(0));
    put(m, "preg_last_error_msg", fn_preg_last_error_msg, 0, Some(0));
    put(m, "preg_split", fn_preg_split, 2, Some(4));
    put(m, "preg_quote", fn_preg_quote, 1, Some(2));
    put(m, "preg_grep", fn_preg_grep, 2, Some(3));
}

// ---------------------------------------------------------------------------
// tests — expected values captured from php 8.5.8
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct NullHost;
    impl Host for NullHost {
        fn write_out(&mut self, _: &[u8]) {}
    }

    /// A host that dispatches `call_callable` to a Rust closure, for the
    /// preg_replace_callback test (mirrors how other modules stub a callback).
    struct CbHost<'a> {
        #[allow(clippy::type_complexity)]
        f: Box<dyn FnMut(&[Value]) -> Value + 'a>,
    }
    impl Host for CbHost<'_> {
        fn write_out(&mut self, _: &[u8]) {}
        fn call_callable(&mut self, _c: &Value, args: &[Value]) -> Result<Value, Thrown> {
            Ok((self.f)(args))
        }
    }

    fn s(b: &str) -> Value {
        Value::str(b)
    }
    fn re(args: &[Value]) -> Value {
        fn_preg_match(&mut NullHost, args).unwrap()
    }

    /// A fresh by-ref slot to pass as `&$matches`.
    fn slot() -> Value {
        Value::Ref(Rc::new(RefCell::new(Value::Null)))
    }
    fn slot_val(v: &Value) -> Value {
        match v {
            Value::Ref(rc) => rc.borrow().clone(),
            o => o.clone(),
        }
    }

    // ---- small array readers for assertions -------------------------------

    fn arr_str(v: &Value, key: ArrayKey) -> String {
        let Value::Array(a) = v else {
            panic!("not array")
        };
        match a.get(&key) {
            Some(Value::Str(p)) => String::from_utf8_lossy(p.as_bytes()).into_owned(),
            other => panic!(
                "expected string at {key:?}, got {:?}",
                other.map(|v| v.type_name())
            ),
        }
    }
    fn as_str(v: &Value) -> String {
        match v {
            Value::Str(p) => String::from_utf8_lossy(p.as_bytes()).into_owned(),
            o => panic!("expected string, got {}", o.type_name()),
        }
    }
    fn as_bytes(v: &Value) -> Vec<u8> {
        match v {
            Value::Str(p) => p.as_bytes().to_vec(),
            o => panic!("expected string, got {}", o.type_name()),
        }
    }

    #[test]
    fn match_simple() {
        let m = slot();
        let r = re(&[s("/\\d+/"), s("abc123"), m.clone()]);
        assert!(matches!(r, Value::Int(1)));
        assert_eq!(arr_str(&slot_val(&m), ArrayKey::Int(0)), "123");
    }

    #[test]
    fn match_no_match() {
        let m = slot();
        let r = re(&[s("/xyz/"), s("abc"), m.clone()]);
        assert!(matches!(r, Value::Int(0)));
        // $matches set to an empty array
        let Value::Array(a) = slot_val(&m) else {
            panic!()
        };
        assert_eq!(a.len(), 0);
    }

    #[test]
    fn match_groups() {
        // php: array(0=>'12-34',1=>'12',2=>'34')
        let m = slot();
        re(&[s("/(\\d+)-(\\d+)/"), s("12-34"), m.clone()]);
        let v = slot_val(&m);
        assert_eq!(arr_str(&v, ArrayKey::Int(0)), "12-34");
        assert_eq!(arr_str(&v, ArrayKey::Int(1)), "12");
        assert_eq!(arr_str(&v, ArrayKey::Int(2)), "34");
    }

    #[test]
    fn match_named_groups() {
        // php: 0=>'2024-01','year'=>'2024',1=>'2024',2=>'01' (name before index)
        let m = slot();
        re(&[s("/(?<year>\\d{4})-(\\d{2})/"), s("2024-01"), m.clone()]);
        let v = slot_val(&m);
        let Value::Array(a) = &v else { panic!() };
        let keys: Vec<ArrayKey> = a.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(
            keys,
            vec![
                ArrayKey::Int(0),
                ArrayKey::Str(PhpStr::from("year")),
                ArrayKey::Int(1),
                ArrayKey::Int(2),
            ]
        );
        assert_eq!(arr_str(&v, ArrayKey::Str(PhpStr::from("year"))), "2024");
        assert_eq!(arr_str(&v, ArrayKey::Int(2)), "01");
    }

    #[test]
    fn match_named_groups_all_spellings() {
        // php accepts (?P<n>), (?<n>) and (?'n'); fancy-regex rejected the last.
        for pat in ["/(?P<y>\\d+)/", "/(?<y>\\d+)/", "/(?'y'\\d+)/"] {
            let m = slot();
            let r = re(&[s(pat), s("42"), m.clone()]);
            assert!(matches!(r, Value::Int(1)), "pattern {pat} failed to match");
            assert_eq!(
                arr_str(&slot_val(&m), ArrayKey::Str(PhpStr::from("y"))),
                "42"
            );
        }
    }

    #[test]
    fn match_offset_capture() {
        // php: 0 => array('123', 3)
        let m = slot();
        re(&[
            s("/\\d+/"),
            s("abc123def"),
            m.clone(),
            Value::Int(PREG_OFFSET_CAPTURE),
        ]);
        let v = slot_val(&m);
        let Value::Array(a) = &v else { panic!() };
        let Some(Value::Array(inner)) = a.get(&ArrayKey::Int(0)) else {
            panic!()
        };
        assert_eq!(as_str(inner.get(&ArrayKey::Int(0)).unwrap()), "123");
        assert!(matches!(inner.get(&ArrayKey::Int(1)), Some(Value::Int(3))));
    }

    #[test]
    fn match_trailing_unmatched_trimmed() {
        // php: (a)(b)?(c) on "ac" => 0=>'ac',1=>'a',2=>'',3=>'c'
        let m = slot();
        re(&[s("/(a)(b)?(c)/"), s("ac"), m.clone()]);
        let v = slot_val(&m);
        assert_eq!(arr_str(&v, ArrayKey::Int(2)), "");
        assert_eq!(arr_str(&v, ArrayKey::Int(3)), "c");
        // (a)(b)? on "a" trims trailing unmatched group 2 entirely
        let m2 = slot();
        re(&[s("/(a)(b)?/"), s("a"), m2.clone()]);
        let Value::Array(a2) = slot_val(&m2) else {
            panic!()
        };
        assert_eq!(a2.len(), 2); // only 0 and 1
    }

    #[test]
    fn match_unmatched_as_null() {
        // php: (a)(b)?(c) on "ac" with UNMATCHED_AS_NULL => group 2 is null
        let m = slot();
        re(&[
            s("/(a)(b)?(c)/"),
            s("ac"),
            m.clone(),
            Value::Int(PREG_UNMATCHED_AS_NULL),
        ]);
        let Value::Array(a) = slot_val(&m) else {
            panic!()
        };
        assert!(matches!(a.get(&ArrayKey::Int(2)), Some(Value::Null)));
        assert_eq!(arr_str(&slot_val(&m), ArrayKey::Int(3)), "c");
    }

    #[test]
    fn match_offset_arg() {
        // php: preg_match('/\d+/','ab12cd34',$m,0,4) => '34'
        let m = slot();
        re(&[
            s("/\\d+/"),
            s("ab12cd34"),
            m.clone(),
            Value::Int(0),
            Value::Int(4),
        ]);
        assert_eq!(arr_str(&slot_val(&m), ArrayKey::Int(0)), "34");
    }

    // ---- NEW: PCRE features fancy-regex could not handle -------------------

    #[test]
    fn feature_possessive_quantifier() {
        // php: /^a++a$/ on "aaa" => 0 (a++ eats all 'a', nothing left for the
        // trailing 'a'); /a*+b/ on "aaab" => 1. fancy-regex failed to compile.
        let m = slot();
        assert!(matches!(re(&[s("/^a++a$/"), s("aaa"), m]), Value::Int(0)));
        let m2 = slot();
        assert!(matches!(re(&[s("/a*+b/"), s("aaab"), m2]), Value::Int(1)));
    }

    #[test]
    fn feature_lookbehind() {
        // php: /(?<=foo)bar/ on "foobar" => 1, match "bar"
        let m = slot();
        assert!(matches!(
            re(&[s("/(?<=foo)bar/"), s("foobar"), m.clone()]),
            Value::Int(1)
        ));
        assert_eq!(arr_str(&slot_val(&m), ArrayKey::Int(0)), "bar");
    }

    #[test]
    fn feature_g_anchor() {
        // php: preg_match_all('/\G\d/','123 45') => ['1','2','3'] (stops at space)
        let (r, v) = match_all(&[s("/\\G\\d/"), s("123 45")]);
        assert!(matches!(r, Value::Int(3)));
        let Value::Array(a) = &v else { panic!() };
        let Some(Value::Array(g0)) = a.get(&ArrayKey::Int(0)) else {
            panic!()
        };
        let got: Vec<String> = g0.iter().map(|(_, x)| as_str(x)).collect();
        assert_eq!(got, vec!["1", "2", "3"]);
    }

    #[test]
    fn feature_recursion_and_subroutines() {
        // php: balanced parens via (?1); fancy-regex could not compile these.
        let m = slot();
        assert!(matches!(
            re(&[s("/^(\\((?:[^()]++|(?1))*\\))$/"), s("(a(b)c)"), m]),
            Value::Int(1)
        ));
        // (?R) whole-pattern recursion
        let m2 = slot();
        assert!(matches!(
            re(&[s("/\\((?:[^()]++|(?R))*\\)/"), s("(a(b)c)"), m2]),
            Value::Int(1)
        ));
        // \g<1> subroutine call
        let m3 = slot();
        assert!(matches!(
            re(&[s("/(\\d+)-\\g<1>/"), s("12-34"), m3.clone()]),
            Value::Int(1)
        ));
        assert_eq!(arr_str(&slot_val(&m3), ArrayKey::Int(0)), "12-34");
    }

    #[test]
    fn feature_ascii_vs_unicode_classes() {
        // php: \d is ASCII-only without /u, Unicode-aware with /u.
        // '٥' is U+0665 ARABIC-INDIC DIGIT FIVE.
        let m = slot();
        assert!(
            matches!(re(&[s("/^\\d+$/"), s("٥"), m]), Value::Int(0)),
            "without /u, \\d must not match an Arabic-Indic digit"
        );
        let m2 = slot();
        assert!(
            matches!(re(&[s("/^\\d+$/u"), s("٥"), m2]), Value::Int(1)),
            "with /u, \\d must match an Arabic-Indic digit"
        );
        // \w likewise: 'ñ' (U+00F1) only matches under /u.
        let m3 = slot();
        assert!(matches!(re(&[s("/^\\w+$/"), s("ñ"), m3]), Value::Int(0)));
        let m4 = slot();
        assert!(matches!(re(&[s("/^\\w+$/u"), s("ñ"), m4]), Value::Int(1)));
    }

    #[test]
    fn feature_invalid_utf8_subject_under_u() {
        // php: an invalid UTF-8 subject with /u returns false (PREG_BAD_UTF8).
        let m = slot();
        let bad = Value::str(vec![0xff, 0xfe]);
        assert!(matches!(re(&[s("/./u"), bad, m]), Value::Bool(false)));
        // without /u, raw bytes match fine.
        let m2 = slot();
        let bad2 = Value::str(vec![0xff, 0xfe]);
        assert!(matches!(re(&[s("/./"), bad2, m2]), Value::Int(1)));
    }

    #[test]
    fn feature_extra_modifiers() {
        // U (ungreedy): a.+ matches as little as possible.
        let m = slot();
        re(&[s("/a.+/U"), s("axxbxx"), m.clone()]);
        assert_eq!(arr_str(&slot_val(&m), ArrayKey::Int(0)), "ax");
        // A (anchored): only matches at the start.
        let m2 = slot();
        assert!(matches!(re(&[s("/bar/A"), s("foobar"), m2]), Value::Int(0)));
        let m3 = slot();
        assert!(matches!(re(&[s("/foo/A"), s("foobar"), m3]), Value::Int(1)));
        // D (dollar end only): $ does not match before a trailing newline.
        let m4 = slot();
        assert!(matches!(re(&[s("/foo$/D"), s("foo\n"), m4]), Value::Int(0)));
        let m5 = slot();
        assert!(matches!(re(&[s("/foo$/"), s("foo\n"), m5]), Value::Int(1)));
        // S / X are accepted as no-ops.
        let m6 = slot();
        assert!(matches!(re(&[s("/foo/SX"), s("foo"), m6]), Value::Int(1)));
        // an unknown modifier is rejected => false.
        let m7 = slot();
        assert!(matches!(
            re(&[s("/foo/z"), s("foo"), m7]),
            Value::Bool(false)
        ));
    }

    #[test]
    fn match_trailing_unmatched_trimmed_middle_kept() {
        // middle unmatched group kept, trailing trimmed: nothing new, sanity.
        let m = slot();
        re(&[s("/(a)(b)?(c)/"), s("ac"), m.clone()]);
        let Value::Array(a) = slot_val(&m) else {
            panic!()
        };
        assert_eq!(a.len(), 4);
    }

    fn match_all(args: &[Value]) -> (Value, Value) {
        let m = slot();
        let mut full = args.to_vec();
        full.insert(2, m.clone());
        let r = fn_preg_match_all(&mut NullHost, &full).unwrap();
        (r, slot_val(&m))
    }

    #[test]
    fn match_all_pattern_order() {
        // php PATTERN_ORDER: 0=>['1a','2b','3c'],1=>['1','2','3'],2=>['a','b','c']
        let (r, v) = match_all(&[s("/(\\d)(\\w)/"), s("1a2b3c")]);
        assert!(matches!(r, Value::Int(3)));
        let Value::Array(a) = &v else { panic!() };
        let Some(Value::Array(g0)) = a.get(&ArrayKey::Int(0)) else {
            panic!()
        };
        assert_eq!(as_str(g0.get(&ArrayKey::Int(1)).unwrap()), "2b");
        let Some(Value::Array(g2)) = a.get(&ArrayKey::Int(2)) else {
            panic!()
        };
        assert_eq!(as_str(g2.get(&ArrayKey::Int(2)).unwrap()), "c");
    }

    #[test]
    fn match_all_set_order() {
        // php SET_ORDER: [[ '1a','1','a'], ['2b','2','b'], ['3c','3','c']]
        let (r, v) = match_all(&[s("/(\\d)(\\w)/"), s("1a2b3c"), Value::Int(PREG_SET_ORDER)]);
        assert!(matches!(r, Value::Int(3)));
        let Value::Array(a) = &v else { panic!() };
        let Some(Value::Array(set1)) = a.get(&ArrayKey::Int(1)) else {
            panic!()
        };
        assert_eq!(as_str(set1.get(&ArrayKey::Int(0)).unwrap()), "2b");
        assert_eq!(as_str(set1.get(&ArrayKey::Int(2)).unwrap()), "b");
    }

    #[test]
    fn match_all_zero() {
        // php: returns 0, $matches == [0 => []]
        let (r, v) = match_all(&[s("/z/"), s("abc")]);
        assert!(matches!(r, Value::Int(0)));
        let Value::Array(a) = &v else { panic!() };
        assert_eq!(a.len(), 1);
        let Some(Value::Array(g0)) = a.get(&ArrayKey::Int(0)) else {
            panic!()
        };
        assert_eq!(g0.len(), 0);
    }

    #[test]
    fn match_all_empty_lazy() {
        // php: preg_match_all('/a*?/','aaa') => ['','a','','a','','a',''] (7).
        // This is the key case the old naive loop got wrong (would give 4).
        let (r, v) = match_all(&[s("/a*?/"), s("aaa")]);
        assert!(matches!(r, Value::Int(7)), "expected 7 matches");
        let Value::Array(a) = &v else { panic!() };
        let Some(Value::Array(g0)) = a.get(&ArrayKey::Int(0)) else {
            panic!()
        };
        let got: Vec<String> = g0.iter().map(|(_, x)| as_str(x)).collect();
        assert_eq!(got, vec!["", "a", "", "a", "", "a", ""]);
    }

    #[test]
    fn match_all_empty_star() {
        // php: preg_match_all('/x*/','axb') => ['','x','',''] (4)
        let (r, v) = match_all(&[s("/x*/"), s("axb")]);
        assert!(matches!(r, Value::Int(4)));
        let Value::Array(a) = &v else { panic!() };
        let Some(Value::Array(g0)) = a.get(&ArrayKey::Int(0)) else {
            panic!()
        };
        let got: Vec<String> = g0.iter().map(|(_, x)| as_str(x)).collect();
        assert_eq!(got, vec!["", "x", "", ""]);
    }

    fn replace(args: &[Value]) -> Value {
        fn_preg_replace(&mut NullHost, args).unwrap()
    }

    #[test]
    fn replace_backrefs() {
        // swap words: $2 $1
        assert_eq!(
            as_str(&replace(&[
                s("/(\\w+)\\s(\\w+)/"),
                s("$2 $1"),
                s("hello world")
            ])),
            "world hello"
        );
        // \1 backslash form
        assert_eq!(
            as_str(&replace(&[s("/(\\d+)/"), s("[\\1]"), s("a1b22c")])),
            "a[1]b[22]c"
        );
        // ${1} brace form, and disambiguation ${1}2 => group1 + '2'
        assert_eq!(
            as_str(&replace(&[s("/(\\d)(\\d)/"), s("${1}2"), s("ab45cd")])),
            "ab42cd"
        );
        // $12 reads two digits => group 12 (nonexistent) => ''
        assert_eq!(
            as_str(&replace(&[s("/(\\d)(\\d)/"), s("$12"), s("ab45cd")])),
            "abcd"
        );
        // \$ is a literal dollar, \n stays verbatim
        assert_eq!(as_str(&replace(&[s("/a/"), s("x\\$1y"), s("a")])), "x$1y");
        assert_eq!(as_str(&replace(&[s("/a/"), s("x\\nz"), s("a")])), "x\\nz");
        // no match returns subject unchanged
        assert_eq!(as_str(&replace(&[s("/z/"), s("#"), s("abc")])), "abc");
    }

    #[test]
    fn replace_limit_and_count() {
        // limit 2
        assert_eq!(
            as_str(&replace(&[s("/\\d/"), s("#"), s("1234"), Value::Int(2)])),
            "##34"
        );
        // count out-param
        let c = slot();
        fn_preg_replace(
            &mut NullHost,
            &[s("/\\d/"), s("#"), s("1a2b3"), Value::Int(-1), c.clone()],
        )
        .unwrap();
        assert!(matches!(slot_val(&c), Value::Int(3)));
    }

    #[test]
    fn replace_array_subject_and_pattern() {
        // subject array: keys preserved
        let mut subj = PhpArray::new();
        subj.push(s("a1"));
        subj.push(s("b2c3"));
        let r = replace(&[s("/\\d/"), s("#"), Value::Array(subj)]);
        let Value::Array(a) = &r else { panic!() };
        assert_eq!(as_str(a.get(&ArrayKey::Int(0)).unwrap()), "a#");
        assert_eq!(as_str(a.get(&ArrayKey::Int(1)).unwrap()), "b#c#");
        // pattern array + replacement array, applied in sequence: XYc
        let mut pats = PhpArray::new();
        pats.push(s("/a/"));
        pats.push(s("/b/"));
        let mut reps = PhpArray::new();
        reps.push(s("X"));
        reps.push(s("Y"));
        let r2 = replace(&[Value::Array(pats), Value::Array(reps), s("abc")]);
        assert_eq!(as_str(&r2), "XYc");
    }

    #[test]
    fn replace_empty_matches() {
        // php: preg_replace('/x*/','-','axb') => '-a--b-'
        assert_eq!(as_str(&replace(&[s("/x*/"), s("-"), s("axb")])), "-a--b-");
        // php: preg_replace('//','-','ab') => '-a-b-'
        assert_eq!(as_str(&replace(&[s("//"), s("-"), s("ab")])), "-a-b-");
    }

    #[test]
    fn replace_callback() {
        // double each number: 'a3b4' => 'a6b8'
        let mut host = CbHost {
            f: Box::new(|args: &[Value]| {
                let Value::Array(m) = &args[0] else { panic!() };
                let whole = m.get(&ArrayKey::Int(0)).unwrap();
                let n = rhp_values::conv::to_int(whole);
                Value::str((n * 2).to_string())
            }),
        };
        let r =
            fn_preg_replace_callback(&mut host, &[s("/\\d+/"), Value::Null, s("a3b4")]).unwrap();
        assert_eq!(as_str(&r), "a6b8");

        // use capture groups: swap the two digits of each pair
        let mut host2 = CbHost {
            f: Box::new(|args: &[Value]| {
                let Value::Array(m) = &args[0] else { panic!() };
                let g1 = as_str(m.get(&ArrayKey::Int(1)).unwrap());
                let g2 = as_str(m.get(&ArrayKey::Int(2)).unwrap());
                Value::str(format!("{g2}{g1}"))
            }),
        };
        let r2 =
            fn_preg_replace_callback(&mut host2, &[s("/(\\d)(\\d)/"), Value::Null, s("x12y34")])
                .unwrap();
        assert_eq!(as_str(&r2), "x21y43");
    }

    fn split(args: &[Value]) -> Vec<String> {
        let Value::Array(a) = fn_preg_split(&mut NullHost, args).unwrap() else {
            panic!()
        };
        a.iter().map(|(_, v)| as_str(v)).collect()
    }

    #[test]
    fn split_basic_and_limit() {
        assert_eq!(split(&[s("/,/"), s("a,b,c")]), vec!["a", "b", "c"]);
        assert_eq!(
            split(&[s("/,/"), s("a,b,c,d"), Value::Int(2)]),
            vec!["a", "b,c,d"]
        );
        assert_eq!(
            split(&[s("/\\s+/"), s("hello   world  foo")]),
            vec!["hello", "world", "foo"]
        );
    }

    #[test]
    fn split_flags() {
        // NO_EMPTY
        assert_eq!(
            split(&[
                s("/,/"),
                s("a,,b,"),
                Value::Int(-1),
                Value::Int(PREG_SPLIT_NO_EMPTY)
            ]),
            vec!["a", "b"]
        );
        // DELIM_CAPTURE
        assert_eq!(
            split(&[
                s("/([,;])/"),
                s("a,b;c"),
                Value::Int(-1),
                Value::Int(PREG_SPLIT_DELIM_CAPTURE)
            ]),
            vec!["a", ",", "b", ";", "c"]
        );
        // DELIM_CAPTURE | NO_EMPTY
        assert_eq!(
            split(&[
                s("/(\\d)/"),
                s("a1b2"),
                Value::Int(-1),
                Value::Int(PREG_SPLIT_DELIM_CAPTURE | PREG_SPLIT_NO_EMPTY)
            ]),
            vec!["a", "1", "b", "2"]
        );
    }

    #[test]
    fn split_offset_capture() {
        // php: preg_split('/,/','a,bb',-1,PREG_SPLIT_OFFSET_CAPTURE)
        //   => [['a',0],['bb',2]]
        let Value::Array(a) = fn_preg_split(
            &mut NullHost,
            &[
                s("/,/"),
                s("a,bb"),
                Value::Int(-1),
                Value::Int(PREG_SPLIT_OFFSET_CAPTURE),
            ],
        )
        .unwrap() else {
            panic!()
        };
        let Some(Value::Array(p0)) = a.get(&ArrayKey::Int(0)) else {
            panic!()
        };
        assert_eq!(as_str(p0.get(&ArrayKey::Int(0)).unwrap()), "a");
        assert!(matches!(p0.get(&ArrayKey::Int(1)), Some(Value::Int(0))));
        let Some(Value::Array(p1)) = a.get(&ArrayKey::Int(1)) else {
            panic!()
        };
        assert_eq!(as_str(p1.get(&ArrayKey::Int(0)).unwrap()), "bb");
        assert!(matches!(p1.get(&ArrayKey::Int(1)), Some(Value::Int(2))));
    }

    #[test]
    fn split_empty_and_edges() {
        // empty pattern: '',a,b,c,''
        assert_eq!(split(&[s("//"), s("abc")]), vec!["", "a", "b", "c", ""]);
        // /a*/ on 'aba' => '','','b','',''
        assert_eq!(split(&[s("/a*/"), s("aba")]), vec!["", "", "b", "", ""]);
        // leading / trailing delimiters
        assert_eq!(split(&[s("/,/"), s(",a,b")]), vec!["", "a", "b"]);
        assert_eq!(split(&[s("/,/"), s("a,b,")]), vec!["a", "b", ""]);
    }

    #[test]
    fn split_delim_capture_optional() {
        // (,)|(;) on 'a,b;c': a, ',', b, '', ';', c
        assert_eq!(
            split(&[
                s("/(,)|(;)/"),
                s("a,b;c"),
                Value::Int(-1),
                Value::Int(PREG_SPLIT_DELIM_CAPTURE)
            ]),
            vec!["a", ",", "b", "", ";", "c"]
        );
    }

    fn quote(args: &[Value]) -> String {
        as_str(&fn_preg_quote(&mut NullHost, args).unwrap())
    }

    #[test]
    fn quote_cases() {
        assert_eq!(quote(&[s("a.b*c+d?e(f)g")]), "a\\.b\\*c\\+d\\?e\\(f\\)g");
        assert_eq!(quote(&[s("a/b/c"), s("/")]), "a\\/b\\/c");
        // full metachar sweep (delimiterless: '/' not escaped)
        assert_eq!(
            quote(&[s(". \\ + * ? [ ^ ] $ ( ) { } = ! < > | : - # /")]),
            "\\. \\\\ \\+ \\* \\? \\[ \\^ \\] \\$ \\( \\) \\{ \\} \\= \\! \\< \\> \\| \\: \\- \\# /"
        );
        // NUL byte -> \000
        let nul = fn_preg_quote(&mut NullHost, &[Value::str(vec![b'a', 0, b'b'])]).unwrap();
        assert_eq!(as_bytes(&nul), b"a\\000b");
    }

    fn grep(args: &[Value]) -> Vec<(ArrayKey, String)> {
        let Value::Array(a) = fn_preg_grep(&mut NullHost, args).unwrap() else {
            panic!()
        };
        a.iter().map(|(k, v)| (k.clone(), as_str(v))).collect()
    }

    #[test]
    fn grep_cases() {
        let mut input = PhpArray::new();
        input.push(s("a1"));
        input.push(s("bb"));
        input.push(s("c3"));
        input.push(s("dd"));
        // php preg_grep('/\d/', ...) => [0=>'a1',2=>'c3'] (keys preserved)
        let r = grep(&[s("/\\d/"), Value::Array(input.clone())]);
        assert_eq!(
            r,
            vec![
                (ArrayKey::Int(0), "a1".into()),
                (ArrayKey::Int(2), "c3".into())
            ]
        );
        // INVERT => [1=>'bb',3=>'dd']
        let r2 = grep(&[
            s("/\\d/"),
            Value::Array(input),
            Value::Int(PREG_GREP_INVERT),
        ]);
        assert_eq!(
            r2,
            vec![
                (ArrayKey::Int(1), "bb".into()),
                (ArrayKey::Int(3), "dd".into())
            ]
        );
    }

    #[test]
    fn invalid_pattern_returns_false() {
        let m = slot();
        assert!(matches!(re(&[s("/(/"), s("abc"), m]), Value::Bool(false)));
        assert!(matches!(
            fn_preg_split(&mut NullHost, &[s("bad"), s("x")]).unwrap(),
            Value::Bool(false)
        ));
        // preg_replace returns null on a bad pattern
        assert!(matches!(
            replace(&[s("/(/"), s("x"), s("abc")]),
            Value::Null
        ));
    }

    #[test]
    fn flag_modifiers() {
        // case-insensitive
        let m = slot();
        assert!(matches!(re(&[s("/ABC/i"), s("xabcy"), m]), Value::Int(1)));
        // dotall: a.b matches across newline
        let m2 = slot();
        assert!(matches!(re(&[s("/a.b/s"), s("a\nb"), m2]), Value::Int(1)));
        // multiline: ^ matches after a newline
        let m2b = slot();
        assert!(matches!(re(&[s("/^b/m"), s("a\nb"), m2b]), Value::Int(1)));
        // extended: whitespace in the pattern is ignored
        let m2c = slot();
        assert!(matches!(re(&[s("/a b c/x"), s("abc"), m2c]), Value::Int(1)));
        // alternate delimiters
        let m3 = slot();
        assert!(matches!(
            re(&[s("#/path/#"), s("/path/"), m3]),
            Value::Int(1)
        ));
        let m4 = slot();
        assert!(matches!(re(&[s("{\\d+}"), s("a12b"), m4]), Value::Int(1)));
    }
}
