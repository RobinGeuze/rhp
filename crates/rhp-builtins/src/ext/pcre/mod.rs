//! PCRE2, the system library php links (statically linked; see
//! docs/design/libraries.md), and the per-thread state ext/pcre/php_pcre.c
//! keeps: the compile and match contexts, the JIT stack, the `pcre.*` ini
//! settings and the compiled-pattern cache.
//!
//! Threading: all state is thread-local, so each VM thread has its own. PCRE2
//! needs no process-wide init, and a compiled pattern never leaves its thread.

pub(crate) mod php_pcre;
pub(crate) mod sys;

use std::cell::RefCell;
use std::ptr;
use std::rc::Rc;

use indexmap::IndexMap;
use sys::*;

/// `PCRE2_MAJOR` of the pcre2.h rhp was built against (php's
/// `PCRE_VERSION_MAJOR` is this header macro).
pub const MAJOR: i64 = crate::version::parse_int(env!("RHP_PCRE2_MAJOR"));
/// `PCRE2_MINOR` of the header (`PCRE_VERSION_MINOR`).
pub const MINOR: i64 = crate::version::parse_int(env!("RHP_PCRE2_MINOR"));

/// php's `#ifdef PCRE2_EXTRA_CASELESS_RESTRICT` (the `r` modifier).
pub(crate) const HAVE_CASELESS_RESTRICT: bool = MAJOR > 10 || (MAJOR == 10 && MINOR >= 43);

/// php_pcre.c's `PCRE_JIT_STACK_MIN_SIZE` / `PCRE_JIT_STACK_MAX_SIZE`.
const JIT_STACK_MIN: usize = 32 * 1024;
const JIT_STACK_MAX: usize = 192 * 1024;

/// php_pcre.c's `PCRE_CACHE_SIZE`.
const CACHE_SIZE: usize = 4096;

/// php's ini defaults for `pcre.backtrack_limit`, `pcre.recursion_limit`.
const BACKTRACK_LIMIT: i64 = 1_000_000;
const RECURSION_LIMIT: i64 = 100_000;

fn config_str(what: u32) -> Option<String> {
    // SAFETY: a NULL `where` asks for the length; then the buffer is that long.
    let len = unsafe { pcre2_config_8(what, ptr::null_mut()) };
    if len <= 0 {
        return None;
    }
    let mut buf = vec![0u8; len as usize + 1];
    let n = unsafe { pcre2_config_8(what, buf.as_mut_ptr().cast()) };
    if n <= 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    buf.truncate(end);
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// `pcre2_config(PCRE2_CONFIG_VERSION)` of the linked library, e.g.
/// `"10.49 2026-09-28"` (php's `PCRE_VERSION`).
pub fn version() -> String {
    config_str(PCRE2_CONFIG_VERSION).unwrap_or_default()
}

/// The Unicode version of the linked library (phpinfo).
pub fn unicode_version() -> String {
    config_str(PCRE2_CONFIG_UNICODE_VERSION).unwrap_or_default()
}

/// The JIT's target, when the library has a JIT (phpinfo).
pub fn jit_target() -> Option<String> {
    config_str(PCRE2_CONFIG_JITTARGET)
}

/// Whether the linked library has a JIT. php's configure asks the library
/// the same thing to set `HAVE_PCRE_JIT_SUPPORT` (`PCRE_JIT_SUPPORT`, the
/// `pcre.jit` directive).
pub fn jit_supported() -> bool {
    let mut v: u32 = 0;
    // SAFETY: PCRE2_CONFIG_JIT writes one uint32_t.
    let rc = unsafe { pcre2_config_8(PCRE2_CONFIG_JIT, (&mut v as *mut u32).cast()) };
    rc >= 0 && v != 0
}

/// One thread's contexts, settings and cache (php_pcre.c's TLS globals).
struct State {
    cctx: *mut pcre2_compile_context_8,
    mctx: *mut pcre2_match_context_8,
    /// Null when the library has no JIT.
    jit_stack: *mut pcre2_jit_stack_8,
    /// `PCRE_G(jit)`.
    jit: bool,
    /// Compiled patterns keyed by the whole delimited pattern, oldest first.
    cache: IndexMap<Vec<u8>, Rc<Regex>>,
}

impl State {
    fn new() -> State {
        // SAFETY: plain constructors; NULL general context = malloc/free.
        let (cctx, mctx) = unsafe {
            (
                pcre2_compile_context_create_8(ptr::null_mut()),
                pcre2_match_context_create_8(ptr::null_mut()),
            )
        };
        assert!(
            !cctx.is_null() && !mctx.is_null(),
            "PCRE2 context allocation failed"
        );
        let jit_stack = if jit_supported() {
            unsafe { pcre2_jit_stack_create_8(JIT_STACK_MIN, JIT_STACK_MAX, ptr::null_mut()) }
        } else {
            ptr::null_mut()
        };
        let mut s = State {
            cctx,
            mctx,
            jit_stack,
            jit: false,
            cache: IndexMap::new(),
        };
        s.defaults();
        s
    }

    /// The ini defaults, as MINIT's REGISTER_INI_ENTRIES applies them.
    fn defaults(&mut self) {
        self.set_backtrack_limit(BACKTRACK_LIMIT);
        self.set_recursion_limit(RECURSION_LIMIT);
        self.set_jit(jit_supported());
    }

    fn set_backtrack_limit(&mut self, v: i64) {
        // OnUpdateBacktrackLimit: `(uint32_t)PCRE_G(backtrack_limit)`.
        unsafe { pcre2_set_match_limit_8(self.mctx, v as u32) };
    }

    fn set_recursion_limit(&mut self, v: i64) {
        unsafe { pcre2_set_depth_limit_8(self.mctx, v as u32) };
    }

    /// OnUpdateJit: the JIT stack is attached only while pcre.jit is on.
    fn set_jit(&mut self, on: bool) {
        self.jit = on;
        let data = if on {
            self.jit_stack.cast()
        } else {
            ptr::null_mut()
        };
        unsafe { pcre2_jit_stack_assign_8(self.mctx, None, data) };
    }
}

impl Drop for State {
    fn drop(&mut self) {
        // Patterns first: the JIT stack may only go once none is left.
        self.cache.clear();
        unsafe {
            pcre2_match_context_free_8(self.mctx);
            pcre2_compile_context_free_8(self.cctx);
            if !self.jit_stack.is_null() {
                pcre2_jit_stack_free_8(self.jit_stack);
            }
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::new());
}

/// Back to php's startup state for a new request: ini defaults, empty cache.
pub(crate) fn reset() {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.cache.clear();
        s.defaults();
    });
}

/// A `pcre.*` directive changed (ini_set, ini_restore, `-d`): push the new
/// value into the match context the way its OnUpdate handler does.
/// OnUpdateLong's "Invalid quantity" warnings are raised by the ini layer
/// (`on_modify_check` in main/ini.rs).
pub(crate) fn ini_update(name: &[u8], value: &[u8]) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        match name {
            b"pcre.backtrack_limit" => s.set_backtrack_limit(crate::ini_parse_quantity(value)),
            b"pcre.recursion_limit" => s.set_recursion_limit(crate::ini_parse_quantity(value)),
            b"pcre.jit" if jit_supported() => s.set_jit(crate::main::ini::ini_parse_bool(value)),
            _ => {}
        }
    });
}

/// A cached compiled pattern for `key` (php's whole delimited pattern).
pub(crate) fn cache_get(key: &[u8]) -> Option<Rc<Regex>> {
    STATE.with(|s| s.borrow().cache.get(key).cloned())
}

/// Cache a compiled pattern. A full cache first drops its oldest 1/8 that
/// nothing is using (pcre_clean_cache).
pub(crate) fn cache_put(key: Vec<u8>, re: Rc<Regex>) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.cache.len() == CACHE_SIZE {
            let mut n = CACHE_SIZE / 8;
            s.cache.retain(|_, r| {
                if n > 0 && Rc::strong_count(r) == 1 {
                    n -= 1;
                    false
                } else {
                    true
                }
            });
        }
        s.cache.insert(key, re);
    });
}

/// A compile failure: PCRE2's error code, offset and message.
pub(crate) struct CompileError {
    pub code: i32,
    pub offset: usize,
    pub message: String,
}

/// What the JIT step reported (php warns and goes on without the JIT).
pub(crate) enum JitIssue {
    /// pcre2_jit_compile ran out of executable memory; JIT is now off for
    /// this thread.
    NoMemory,
    /// Any other JIT failure, with PCRE2's message.
    Failed(String),
}

/// One match: `(start, end)` per group `0..=ncaptures` (`None` = unset), and
/// the last `(*MARK)` name on the matching path.
#[derive(Clone, Debug, Default)]
pub(crate) struct Ovector {
    pub groups: Vec<Option<(usize, usize)>>,
    pub mark: Option<Vec<u8>>,
}

impl std::ops::Deref for Ovector {
    type Target = Vec<Option<(usize, usize)>>;
    fn deref(&self) -> &Self::Target {
        &self.groups
    }
}

/// A compiled pattern; frees its code on drop.
pub struct Regex {
    code: *mut pcre2_code_8,
    ncaptures: usize,
    /// Group index → name, `ncaptures + 1` long.
    names: Vec<Option<String>>,
    /// Compiled with PCRE2_UTF (the `u` modifier).
    utf: bool,
    /// JIT-compiled (php's `PREG_JIT`).
    jit: bool,
    /// Matches through the thread's php match context (else PCRE2's
    /// defaults, for [`Pattern`]).
    php_ctx: bool,
}

impl Drop for Regex {
    fn drop(&mut self) {
        unsafe { pcre2_code_free_8(self.code) };
    }
}

/// The address PCRE2 gets for a pattern or subject. php's are zend_strings,
/// so an empty one is still a readable NUL; an empty slice's pointer is
/// dangling, and PCRE2's JIT on aarch64 Linux reads at it.
fn buf_ptr(s: &[u8]) -> *const u8 {
    static NUL: u8 = 0;
    if s.is_empty() {
        &NUL
    } else {
        s.as_ptr()
    }
}

/// Match data sized for one pattern; freed on drop.
struct MatchData(*mut pcre2_match_data_8);

impl MatchData {
    fn new(re: &Regex) -> MatchData {
        let p = unsafe { pcre2_match_data_create_from_pattern_8(re.code, ptr::null_mut()) };
        assert!(!p.is_null(), "PCRE2 match data allocation failed");
        MatchData(p)
    }
}

impl Drop for MatchData {
    fn drop(&mut self) {
        unsafe { pcre2_match_data_free_8(self.0) };
    }
}

fn error_message(code: i32) -> String {
    let mut buf = [0u8; 256];
    let n = unsafe { pcre2_get_error_message_8(code, buf.as_mut_ptr(), buf.len()) };
    String::from_utf8_lossy(&buf[..n.max(0) as usize]).into_owned()
}

impl Regex {
    /// pcre_get_compiled_regex_cache's compile step: pcre2_compile through
    /// the thread's compile context, then pcre2_jit_compile when pcre.jit is
    /// on. A JIT problem is reported in `jit_issue`; the pattern still works.
    pub(crate) fn compile(
        body: &[u8],
        options: u32,
        extra_options: u32,
        jit_issue: &mut Option<JitIssue>,
    ) -> Result<Regex, CompileError> {
        let (cctx, jit) = STATE.with(|s| {
            let s = s.borrow();
            (s.cctx, s.jit)
        });
        unsafe { pcre2_set_compile_extra_options_8(cctx, extra_options) };
        let mut re = Self::compile_with(body, options, cctx)?;
        re.php_ctx = true;
        if jit {
            let rc = unsafe { pcre2_jit_compile_8(re.code, PCRE2_JIT_COMPLETE) };
            if rc >= 0 {
                let mut size: usize = 0;
                let info = unsafe {
                    pcre2_pattern_info_8(
                        re.code,
                        PCRE2_INFO_JITSIZE,
                        (&mut size as *mut usize).cast(),
                    )
                };
                re.jit = info == 0 && size > 0;
            } else if rc == PCRE2_ERROR_NOMEMORY {
                STATE.with(|s| s.borrow_mut().jit = false);
                *jit_issue = Some(JitIssue::NoMemory);
            } else {
                *jit_issue = Some(JitIssue::Failed(error_message(rc)));
            }
        }
        Ok(re)
    }

    fn compile_with(
        body: &[u8],
        options: u32,
        cctx: *mut pcre2_compile_context_8,
    ) -> Result<Regex, CompileError> {
        let mut errcode: i32 = 0;
        let mut erroffset: usize = 0;
        let code = unsafe {
            pcre2_compile_8(
                buf_ptr(body),
                body.len(),
                options,
                &mut errcode,
                &mut erroffset,
                cctx,
            )
        };
        if code.is_null() {
            return Err(CompileError {
                code: errcode,
                offset: erroffset,
                message: error_message(errcode),
            });
        }
        let ncaptures = pattern_info_u32(code, PCRE2_INFO_CAPTURECOUNT) as usize;
        let names = read_name_table(code, ncaptures);
        Ok(Regex {
            code,
            ncaptures,
            names,
            utf: options & PCRE2_UTF != 0,
            jit: false,
            php_ctx: false,
        })
    }

    pub(crate) fn ncaptures(&self) -> usize {
        self.ncaptures
    }

    pub(crate) fn names(&self) -> &[Option<String>] {
        &self.names
    }

    /// The options of a match's first attempt: php validates the subject's
    /// UTF-8 once, on the first attempt only.
    fn first_options(&self) -> u32 {
        if self.utf {
            0
        } else {
            PCRE2_NO_UTF_CHECK
        }
    }

    /// One attempt, calling what php calls: pcre2_jit_match for a
    /// JIT-compiled pattern when no UTF check or other option is wanted,
    /// pcre2_match otherwise. Returns PCRE2's result code.
    fn exec(&self, subject: &[u8], start: usize, options: u32, md: &MatchData) -> i32 {
        let mctx = if self.php_ctx {
            STATE.with(|s| s.borrow().mctx)
        } else {
            ptr::null_mut()
        };
        unsafe {
            if self.jit && options == PCRE2_NO_UTF_CHECK {
                pcre2_jit_match_8(
                    self.code,
                    buf_ptr(subject),
                    subject.len(),
                    start,
                    options,
                    md.0,
                    mctx,
                )
            } else {
                pcre2_match_8(
                    self.code,
                    buf_ptr(subject),
                    subject.len(),
                    start,
                    options,
                    md.0,
                    mctx,
                )
            }
        }
    }

    fn read(&self, md: &MatchData) -> Ovector {
        let ov = unsafe { pcre2_get_ovector_pointer_8(md.0) };
        let groups = (0..=self.ncaptures)
            .map(|i| {
                let (s, e) = unsafe { (*ov.add(2 * i), *ov.add(2 * i + 1)) };
                (s != PCRE2_UNSET && e != PCRE2_UNSET).then_some((s, e))
            })
            .collect();
        // The mark is NUL-terminated and preceded by its length.
        let mp = unsafe { pcre2_get_mark_8(md.0) };
        let mark = (!mp.is_null()).then(|| {
            let len = unsafe { *mp.sub(1) } as usize;
            unsafe { std::slice::from_raw_parts(mp, len) }.to_vec()
        });
        Ovector { groups, mark }
    }

    /// One match from `start` (preg_match, preg_grep). `Err` is PCRE2's
    /// error code.
    pub(crate) fn find_at(&self, subject: &[u8], start: usize) -> Result<Option<Ovector>, i32> {
        let md = MatchData::new(self);
        match self.exec(subject, start, self.first_options(), &md) {
            rc if rc >= 0 => Ok(Some(self.read(&md))),
            PCRE2_ERROR_NOMATCH => Ok(None),
            rc => Err(rc),
        }
    }

    /// Successive matches from `start`, as php's global loops find them:
    /// after an empty match, retry at the same place anchored and not-empty,
    /// and on failure advance one code unit. Each attempt runs only when the
    /// next item is asked for, so a caller with a limit stops matching there
    /// (php ignores what happens past its limits). An error ends it.
    pub(crate) fn matches<'a>(&'a self, subject: &'a [u8], start: usize) -> Matches<'a> {
        Matches {
            re: self,
            subject,
            md: MatchData::new(self),
            pos: start,
            first: true,
            retry: false,
            done: false,
        }
    }

    /// Up to `max` of [`Regex::matches`] into `out`. On an error, `out`
    /// keeps the matches found before it.
    pub(crate) fn find_all(
        &self,
        subject: &[u8],
        start: usize,
        max: usize,
        out: &mut Vec<Ovector>,
    ) -> Result<(), i32> {
        let mut it = self.matches(subject, start);
        while out.len() < max {
            match it.next() {
                Some(m) => out.push(m?),
                None => break,
            }
        }
        Ok(())
    }

    /// calculate_unit_length: one UTF-8 character under `u`, else one byte.
    fn unit_len(&self, subject: &[u8], pos: usize) -> usize {
        if !self.utf {
            return 1;
        }
        let mut end = pos + 1;
        while end < subject.len() && (subject[end] & 0xC0) == 0x80 {
            end += 1;
        }
        end - pos
    }
}

/// See [`Regex::matches`].
pub(crate) struct Matches<'a> {
    re: &'a Regex,
    subject: &'a [u8],
    md: MatchData,
    pos: usize,
    first: bool,
    /// The last match was empty: the next attempt is the anchored retry.
    retry: bool,
    done: bool,
}

impl Iterator for Matches<'_> {
    type Item = Result<Ovector, i32>;

    fn next(&mut self) -> Option<Self::Item> {
        let (re, subject) = (self.re, self.subject);
        while !self.done {
            let rc = if self.first {
                self.first = false;
                re.exec(subject, self.pos, re.first_options(), &self.md)
            } else if self.retry {
                let opts = PCRE2_NO_UTF_CHECK | PCRE2_NOTEMPTY_ATSTART | PCRE2_ANCHORED;
                re.exec(subject, self.pos, opts, &self.md)
            } else if re.jit && self.pos > subject.len() {
                PCRE2_ERROR_BADOFFSET
            } else {
                re.exec(subject, self.pos, PCRE2_NO_UTF_CHECK, &self.md)
            };
            if rc >= 0 {
                let ov = re.read(&self.md);
                let (s, e) = ov[0].unwrap_or((self.pos, self.pos));
                self.pos = e;
                self.retry = s == e;
                return Some(Ok(ov));
            }
            if rc == PCRE2_ERROR_NOMATCH && self.retry && self.pos < subject.len() {
                self.retry = false;
                self.pos += re.unit_len(subject, self.pos);
                continue;
            }
            self.done = true;
            if rc != PCRE2_ERROR_NOMATCH {
                return Some(Err(rc));
            }
        }
        None
    }
}

fn pattern_info_u32(code: *const pcre2_code_8, what: u32) -> u32 {
    let mut v: u32 = 0;
    unsafe { pcre2_pattern_info_8(code, what, (&mut v as *mut u32).cast()) };
    v
}

/// The name table as an index → name map, `ncaptures + 1` long. Each entry
/// is `[group_hi, group_lo, name..., 0]`.
fn read_name_table(code: *const pcre2_code_8, ncaptures: usize) -> Vec<Option<String>> {
    let mut names = vec![None; ncaptures + 1];
    let count = pattern_info_u32(code, PCRE2_INFO_NAMECOUNT) as usize;
    if count == 0 {
        return names;
    }
    let entry_size = pattern_info_u32(code, PCRE2_INFO_NAMEENTRYSIZE) as usize;
    let mut table: *const u8 = ptr::null();
    unsafe {
        pcre2_pattern_info_8(
            code,
            PCRE2_INFO_NAMETABLE,
            (&mut table as *mut *const u8).cast(),
        )
    };
    for k in 0..count {
        let entry = unsafe { std::slice::from_raw_parts(table.add(k * entry_size), entry_size) };
        let group = ((entry[0] as usize) << 8) | entry[1] as usize;
        let name = &entry[2..];
        let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
        if group < names.len() {
            names[group] = Some(String::from_utf8_lossy(name).into_owned());
        }
    }
    names
}

/// `PCRE2_DOTALL`, for [`Pattern::new`].
pub const DOTALL: u32 = PCRE2_DOTALL;

/// A pattern compiled outside php's settings: no JIT, PCRE2's default
/// limits. For tools that need PCRE semantics (rhp-difftest's EXPECTF),
/// so a binary links one PCRE2.
pub struct Pattern(Regex);

impl Pattern {
    /// Compile `pattern` (no delimiters) with PCRE2 compile `options`.
    pub fn new(pattern: &str, options: u32) -> Result<Pattern, String> {
        Regex::compile_with(pattern.as_bytes(), options, ptr::null_mut())
            .map(Pattern)
            .map_err(|e| format!("{} at offset {}", e.message, e.offset))
    }

    /// Whether `subject` matches. `Err` is PCRE2's error code.
    pub fn is_match(&self, subject: &[u8]) -> Result<bool, i32> {
        self.0.find_at(subject, 0).map(|m| m.is_some())
    }

    /// The groups of every non-overlapping match, `(start, end)` per group
    /// (`None` = unset). After an empty match the search resumes one byte
    /// later, and an empty match right after a match is skipped. Stops at
    /// the first error.
    pub fn captures_all(&self, subject: &[u8]) -> Vec<Vec<Option<(usize, usize)>>> {
        let md = MatchData::new(&self.0);
        let mut out = Vec::new();
        let mut at = 0;
        let mut last_match = None;
        while at <= subject.len() {
            if self.0.exec(subject, at, 0, &md) < 0 {
                break;
            }
            let ov = self.0.read(&md);
            let Some((s, e)) = ov[0] else { break };
            if s == e {
                at = e + 1;
                if last_match == Some(e) {
                    continue;
                }
            } else {
                at = e;
            }
            last_match = Some(e);
            out.push(ov.groups);
        }
        out
    }
}

#[cfg(test)]
mod tests;
