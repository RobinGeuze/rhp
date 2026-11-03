//! Zend OPcache's compiled-script cache (ext/opcache/ZendAccelerator.c),
//! without the optimizer, the JIT and the file cache.
//!
//! A cached script is a compiled [`Program`]. The main script, `include`
//! and `require` of an unchanged file reuse it instead of lexing, parsing
//! and compiling again. A `Program` is immutable: the VM copies the protos
//! it runs (an include is rebased into the request's tables), so statics,
//! class statics and constants live in the request's `Vm` and start fresh
//! in every request, exactly as without the cache.
//!
//! The cache is per OS thread, like the VM: `Program` holds `Rc`s. Every
//! request served on a thread shares it, as php-fpm's workers share their
//! shared-memory segment. Once compiled bytecode is `Arc`-based, [`Cache`]
//! can move behind a process-wide lock; nothing outside this module
//! touches it directly.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use rhp_builtins::{self as builtins, Host};
use rhp_bytecode::{ClassProto, ConstInitProto, FunctionProto, Program, PropDefaultProto};
use rhp_values::{ArrayKey, PhpArray, Value};

use crate::{IncludeError, Vm};

/// Compiles a file's bytes (the includer, or the SAPI's main-script compile).
pub(crate) type Compile<'a> = dyn FnMut(&mut Vm, &[u8]) -> Result<Program, IncludeError> + 'a;

/// Which compile of a file a [`Program`] is. A request's main script
/// carries the built-in prelude, an included one doesn't, so the two are
/// cached side by side under one script.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Main,
    Include,
}

/// php's `zend_accel_restart_reason`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Restart {
    Oom,
    Hash,
    User,
}

/// One cached file (php's `zend_persistent_script`).
struct Script {
    main: Option<Rc<Program>>,
    include: Option<Rc<Program>>,
    /// The file's mtime when compiled; 0 when not validating timestamps
    /// (and for preloaded scripts, which are never revalidated).
    timestamp: i64,
    /// No stat before this request time (`revalidate_freq`).
    revalidate: i64,
    hits: i64,
    last_used: i64,
    memory: i64,
    /// Discarded (stale or invalidated): never served, but still listed by
    /// `opcache_get_status()` until replaced, as php keeps the hash entry.
    corrupted: bool,
    /// Insertion order, for php's hash-table listing order.
    seq: u64,
}

impl Script {
    fn program(&self, kind: Kind) -> Option<&Rc<Program>> {
        match kind {
            Kind::Main => self.main.as_ref(),
            Kind::Include => self.include.as_ref(),
        }
    }
}

/// Settings fixed when the cache starts (php's PHP_INI_SYSTEM directives).
struct Startup {
    /// `opcache.max_accelerated_files`, rounded up to php's next prime.
    max_entries: i64,
    memory_consumption: i64,
    interned_strings_buffer: i64,
    max_wasted_percentage: f64,
    blacklist: Vec<Vec<u8>>,
}

/// The thread's cache (php's shared memory segment).
#[derive(Default)]
pub(crate) struct Cache {
    startup: Option<Startup>,
    /// Keyed by resolved path.
    scripts: indexmap::IndexMap<Vec<u8>, Script>,
    /// php's indirect keys: a script requested by another name (relative,
    /// or through a symlink) → its resolved path.
    keys: std::collections::HashMap<Vec<u8>, Vec<u8>>,
    seq: u64,
    hits: i64,
    misses: i64,
    blacklist_misses: i64,
    oom_restarts: i64,
    hash_restarts: i64,
    manual_restarts: i64,
    start_time: i64,
    last_restart_time: i64,
    restart_pending: Option<Restart>,
    /// php's `ZCSG(accelerator_enabled)`: off from a scheduled restart until
    /// it happens at the start of the next request.
    disabled: bool,
    memory_exhausted: bool,
    used: i64,
    wasted: i64,
    /// `zend_accel_error` lines raised inside the cache, written by the
    /// engine once it has the cache back.
    log: Vec<(i64, String)>,
    /// `opcache.preload`'s result, once run on this thread (it survives
    /// restarts, as php re-adds the preloaded scripts).
    preload: Option<Rc<Preload>>,
    /// Whether `opcache.preload` succeeded, once it ran on this thread.
    preload_result: Option<bool>,
}

mod preload;

use preload::{cache_preloaded, preload_report};
pub(crate) use preload::{Preload, PreloadRec};

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

fn with_cache<R>(f: impl FnOnce(&mut Cache) -> R) -> R {
    CACHE.with(|c| f(&mut c.borrow_mut()))
}

/// A request's view of the cache (php's per-request `ZCG` fields).
pub(crate) struct Request {
    /// `accel_startup_ok`: opcache.enable (and opcache.enable_cli under
    /// the CLI) at startup.
    started: bool,
    /// `ZCG(accelerator_enabled)` as the request began.
    enabled: bool,
    /// `ZCG(request_time)`: the request's start, in whole seconds.
    request_time: i64,
}

/// php's `zend_accel_hash_init` sizes: the first prime ≥ the setting.
const PRIMES: &[i64] = &[
    5, 11, 19, 53, 107, 223, 463, 983, 1979, 3907, 7963, 16229, 32531, 65407, 130987, 262237,
    524521, 1048793,
];

/// What a `zend_accel_hash_entry` and its bucket pointer take.
const HASH_ENTRY_SIZE: i64 = 48;
/// The cache's own bookkeeping outside the hash and the strings buffer.
const BASE_SIZE: i64 = 512 * 1024;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A path from raw bytes (paths are bytes on unix).
fn os_path(b: &[u8]) -> &Path {
    use std::os::unix::ffi::OsStrExt;
    Path::new(std::ffi::OsStr::from_bytes(b))
}

fn path_bytes(p: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    p.as_os_str().as_bytes().to_vec()
}

fn mtime_and_size(path: &[u8]) -> Option<(i64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(os_path(path)).ok()?;
    Some((m.mtime(), m.size()))
}

fn ini_flag(v: &[u8]) -> bool {
    crate::ini_bool(v)
}

/// php's `zend_inline_hash_func` (DJBX33A) with the high bit set, as the
/// accelerator hash stores it.
fn zend_hash(key: &[u8]) -> u64 {
    key.iter()
        .fold(5381u64, |h, &b| h.wrapping_mul(33).wrapping_add(b as u64))
        | (1 << 63)
}

/// The bytes a compiled program occupies, as this cache accounts them
/// (php's `memory_consumption` per script).
fn program_size(p: &Program) -> i64 {
    fn proto(f: &FunctionProto) -> usize {
        let mut n = std::mem::size_of::<FunctionProto>()
            + f.code.len() * std::mem::size_of::<rhp_bytecode::Instr>()
            + f.consts.len() * std::mem::size_of::<Value>()
            + f.params.len() * std::mem::size_of::<rhp_bytecode::ParamInfo>()
            + f.debug.lines.len() * 8
            + f.exception_table.len() * std::mem::size_of::<rhp_bytecode::ExRegion>();
        n += f
            .names
            .iter()
            .map(|s| s.as_bytes().len() + 24)
            .sum::<usize>();
        n += f.consts.iter().map(value_size).sum::<usize>();
        n
    }
    fn value_size(v: &Value) -> usize {
        match v {
            Value::Str(s) => s.as_bytes().len() + 24,
            Value::Array(a) => a.iter().map(|(_, v)| 32 + value_size(v)).sum(),
            _ => 0,
        }
    }
    fn class(c: &ClassProto) -> usize {
        let mut n = std::mem::size_of::<ClassProto>();
        for m in &c.methods {
            n += proto(&m.proto) + m.name.as_bytes().len() + 24;
        }
        for p in &c.props {
            n += 64 + p.name.as_bytes().len();
            if let PropDefaultProto::Init(t) = &p.default {
                n += proto(t);
            }
        }
        for s in &c.static_props {
            n += 64 + s.name.as_bytes().len();
            if let ConstInitProto::Thunk(t) = &s.init {
                n += proto(t);
            }
        }
        for k in &c.constants {
            n += 64 + k.name.as_bytes().len();
            if let ConstInitProto::Thunk(t) = &k.value {
                n += proto(t);
            }
        }
        n
    }
    let user_fns = p.functions.iter().skip(p.num_prelude_functions);
    let user_classes = p.classes.iter().skip(p.num_prelude_classes);
    let n = std::mem::size_of::<Program>()
        + proto(&p.main)
        + user_fns.map(|f| proto(f)).sum::<usize>()
        + p.closures.iter().map(|f| proto(f)).sum::<usize>()
        + user_classes
            .chain(&p.cond_classes)
            .map(class)
            .sum::<usize>();
    // ZEND_ALIGNED_SIZE
    ((n + 7) & !7) as i64
}

/// php's blacklist entry matching (zend_accelerator_blacklist.c): each
/// entry is a path prefix in which `?` is one character but `/`, `*` any
/// run within a path component and `**` any run at all.
fn blacklisted(entries: &[Vec<u8>], path: &[u8]) -> bool {
    fn prefix(pat: &[u8], s: &[u8]) -> bool {
        match pat.first() {
            None => true,
            Some(b'*') if pat.get(1) == Some(&b'*') => {
                (0..=s.len()).any(|i| prefix(&pat[2..], &s[i..]))
            }
            Some(b'*') => {
                let run = s.iter().take_while(|&&c| c != b'/').count();
                (0..=run).any(|i| prefix(&pat[1..], &s[i..]))
            }
            Some(b'?') => s.first().is_some_and(|&c| c != b'/') && prefix(&pat[1..], &s[1..]),
            Some(&c) => s.first() == Some(&c) && prefix(&pat[1..], &s[1..]),
        }
    }
    entries.iter().any(|e| prefix(e, path))
}

/// `expand_filepath`: an absolute path with `.` and `..` resolved
/// lexically, a relative one taken from `base`.
fn expand_path(path: &[u8], base: &[u8]) -> Vec<u8> {
    let mut full = Vec::new();
    if !path.starts_with(b"/") {
        full.extend_from_slice(base);
        full.push(b'/');
    }
    full.extend_from_slice(path);
    let mut parts: Vec<&[u8]> = Vec::new();
    for seg in full.split(|&c| c == b'/') {
        match seg {
            b"" | b"." => {}
            b".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let mut out = Vec::new();
    for p in parts {
        out.push(b'/');
        out.extend_from_slice(p);
    }
    if out.is_empty() {
        out.push(b'/');
    }
    out
}

/// The files `opcache.blacklist_filename` names (a glob in its last path
/// component), sorted as php_glob returns them.
fn blacklist_files(pattern: &[u8]) -> Vec<Vec<u8>> {
    let (dir, name) = match pattern.iter().rposition(|&c| c == b'/') {
        Some(i) => (&pattern[..i.max(1)], &pattern[i + 1..]),
        None => (&b"."[..], pattern),
    };
    if !name.iter().any(|c| matches!(c, b'*' | b'?' | b'[')) {
        return vec![pattern.to_vec()];
    }
    let Ok(rd) = std::fs::read_dir(os_path(dir)) else {
        return Vec::new();
    };
    let mut out: Vec<Vec<u8>> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let n = path_bytes(Path::new(&e.file_name()));
            (!n.starts_with(b".") && glob_full(name, &n)).then(|| {
                let mut p = dir.to_vec();
                p.push(b'/');
                p.extend_from_slice(&n);
                p
            })
        })
        .collect();
    out.sort();
    out
}

/// A whole-name glob match (`*` and `?`).
fn glob_full(pat: &[u8], s: &[u8]) -> bool {
    match pat.first() {
        None => s.is_empty(),
        Some(b'*') => (0..=s.len()).any(|i| glob_full(&pat[1..], &s[i..])),
        Some(b'?') => !s.is_empty() && glob_full(&pat[1..], &s[1..]),
        Some(&c) => s.first() == Some(&c) && glob_full(&pat[1..], &s[1..]),
    }
}

/// `zend_accel_blacklist_loadone`: one path per line; blank lines and `;`
/// comments skipped, surrounding quotes stripped, relative entries taken
/// from the blacklist file's directory.
fn load_blacklist(pattern: &[u8]) -> Vec<Vec<u8>> {
    let mut entries = Vec::new();
    for file in blacklist_files(pattern) {
        let p = os_path(&file);
        let Ok(text) = std::fs::read(p) else {
            continue;
        };
        let base = std::fs::canonicalize(p)
            .ok()
            .and_then(|c| c.parent().map(path_bytes));
        for line in text.split(|&c| c == b'\n') {
            let mut l = line.strip_suffix(b"\r").unwrap_or(line);
            while let Some(rest) = l.strip_prefix(b"\r") {
                l = rest;
            }
            if l.len() >= 2 && l[0] == b'"' && l[l.len() - 1] == b'"' {
                l = &l[1..l.len() - 1];
            }
            if l.is_empty() || l[0] == b';' {
                continue;
            }
            let base = base.clone().unwrap_or_else(|| {
                std::env::current_dir()
                    .map(|d| path_bytes(&d))
                    .unwrap_or_default()
            });
            entries.push(expand_path(l, &base));
        }
    }
    entries
}

/// php's `asctime(localtime(t))` without the newline.
fn asctime(t: i64) -> String {
    builtins::asctime_local(t)
}

/// `zend_accel_error` levels.
const LOG_INFO: i64 = 3;
const LOG_DEBUG: i64 = 4;

impl Vm {
    /// Write the cache's pending `zend_accel_error` lines to opcache's log.
    fn flush_accel_log(&mut self) {
        let msgs = with_cache(|c| std::mem::take(&mut c.log));
        for (level, msg) in msgs {
            if let Some(line) = builtins::accel_log_line(level, &msg) {
                self.write_accel_log(&line, &builtins::accel_log_dest());
            }
        }
    }

    fn ini(&self, name: &[u8]) -> Vec<u8> {
        self.ini_get(name).unwrap_or_default()
    }

    fn ini_long(&self, name: &[u8]) -> i64 {
        builtins::c_atol(&self.ini(name))
    }

    /// The request's opcache state, set up on first use (php's
    /// `accel_startup` for the thread's first request, then `RINIT`).
    pub(crate) fn opcache_request(&mut self) -> &Request {
        if self.opcache.is_none() {
            let startup = |n: &[u8]| builtins::ini_startup_value(n).unwrap_or_default();
            let cli = builtins::sapi::sapi_name() == "cli";
            let started = ini_flag(&startup(b"opcache.enable"))
                && (!cli || ini_flag(&startup(b"opcache.enable_cli")));
            let request_time = self
                .sapi
                .request
                .as_ref()
                .map(|r| r.request_time as i64)
                .filter(|&t| t > 0)
                .unwrap_or_else(now);
            let enabled = started && self.opcache_activate(request_time);
            self.opcache = Some(Request {
                started,
                enabled,
                request_time,
            });
            self.flush_accel_log();
        }
        self.opcache.as_ref().unwrap()
    }

    /// Start the thread's cache if this is its first enabled request, run a
    /// scheduled restart, and return whether the cache serves this request.
    fn opcache_activate(&mut self, request_time: i64) -> bool {
        let startup = |n: &[u8]| builtins::ini_startup_value(n).unwrap_or_default();
        let max_files = builtins::c_atol(&startup(b"opcache.max_accelerated_files"));
        let max_entries = PRIMES
            .iter()
            .copied()
            .find(|&p| max_files <= p)
            .unwrap_or(max_files);
        let mb = builtins::c_atol(&startup(b"opcache.memory_consumption")).max(8);
        let strings = builtins::ini_parse_quantity(&startup(b"opcache.interned_strings_buffer"));
        let wasted = builtins::c_atol(&startup(b"opcache.max_wasted_percentage"));
        let blacklist_file = startup(b"opcache.blacklist_filename");
        with_cache(|c| {
            if c.startup.is_none() {
                let blacklist = if blacklist_file.is_empty() {
                    Vec::new()
                } else {
                    load_blacklist(&blacklist_file)
                };
                c.startup = Some(Startup {
                    max_entries,
                    memory_consumption: mb.saturating_mul(1024 * 1024),
                    interned_strings_buffer: strings.max(0) * 1024 * 1024,
                    max_wasted_percentage: if (1..=50).contains(&wasted) {
                        wasted as f64 / 100.0
                    } else {
                        0.05
                    },
                    blacklist,
                });
                c.start_time = request_time;
            }
            if let Some(reason) = c.restart_pending.take() {
                match reason {
                    Restart::Oom => c.oom_restarts += 1,
                    Restart::Hash => c.hash_restarts += 1,
                    Restart::User => c.manual_restarts += 1,
                }
                c.log.push((LOG_DEBUG, "Restarting!".into()));
                // zend_reset_cache_vars, then preload_restart.
                c.scripts.clear();
                c.keys.clear();
                c.used = 0;
                c.wasted = 0;
                c.hits = 0;
                c.misses = 0;
                c.blacklist_misses = 0;
                c.memory_exhausted = false;
                c.disabled = false;
                if let Some(pre) = c.preload.clone() {
                    cache_preloaded(c, &pre, None, request_time);
                }
                c.last_restart_time = if c.last_restart_time < request_time {
                    request_time
                } else {
                    c.last_restart_time + 1
                };
            }
            !c.disabled
        })
    }

    /// Whether compiles go through the cache now: started, enabled when
    /// the request began, and not switched off by `ini_set('opcache.enable', 0)`.
    fn opcache_on(&mut self) -> bool {
        self.opcache_request().enabled && ini_flag(&self.ini(b"opcache.enable"))
    }

    /// php's `persistent_compile_file`: the cached program for the file at
    /// `path` (a resolved path), or a fresh compile, cached when php would
    /// cache it. `source` reads the file; `compile` compiles its bytes.
    pub(crate) fn opcache_compile(
        &mut self,
        path: &[u8],
        given: &[u8],
        kind: Kind,
        validated: bool,
        source: &mut dyn FnMut() -> Vec<u8>,
        compile: &mut Compile,
    ) -> Result<Rc<Program>, IncludeError> {
        let r = self.persistent_compile(path, given, kind, validated, source, compile);
        self.flush_accel_log();
        r
    }

    fn persistent_compile(
        &mut self,
        path: &[u8],
        given: &[u8],
        kind: Kind,
        validated: bool,
        source: &mut dyn FnMut() -> Vec<u8>,
        compile: &mut Compile,
    ) -> Result<Rc<Program>, IncludeError> {
        if !self.opcache_on() {
            let src = source();
            return compile(self, &src).map(Rc::new);
        }
        let request_time = self.opcache_request().request_time;
        let validate = ini_flag(&self.ini(b"opcache.validate_timestamps"));
        let freq = self.ini_long(b"opcache.revalidate_freq");
        // A cached script, still valid (validate_timestamp_and_record).
        let found = with_cache(|c| {
            let script = c.scripts.get_mut(path).filter(|s| !s.corrupted)?;
            if !validated
                && validate
                && script.timestamp != 0
                && !(freq != 0 && script.revalidate >= request_time)
            {
                let fresh = mtime_and_size(path).is_some_and(|(m, _)| m == script.timestamp);
                if !fresh {
                    script.corrupted = true;
                    script.main = None;
                    script.include = None;
                    c.wasted += script.memory;
                    return None;
                }
                script.revalidate = request_time + freq;
            }
            c.hits += 1;
            script.hits += 1;
            script.last_used = request_time;
            Some(script.program(kind).cloned())
        });
        let key = self.persistent_key(given, path);
        let add_key = |c: &mut Cache| {
            if let Some(k) = &key {
                let room = ((c.scripts.len() + c.keys.len()) as i64)
                    < c.startup.as_ref().map_or(0, |s| s.max_entries);
                if room && c.scripts.contains_key(path) && !c.keys.contains_key(k) {
                    c.keys.insert(k.clone(), path.to_vec());
                    let shown = String::from_utf8_lossy(k).into_owned();
                    c.log.push((LOG_INFO, format!("Added key '{shown}'")));
                }
            }
        };
        match found {
            Some(Some(p)) => {
                with_cache(add_key);
                return Ok(p);
            }
            // The other compile of a cached file: compile this one too and
            // keep it with the script.
            Some(None) => {
                let src = source();
                let p = Rc::new(compile(self, &src)?);
                with_cache(|c| {
                    if let Some(s) = c.scripts.get_mut(path) {
                        let extra = program_size(&p);
                        s.memory += extra;
                        c.used += extra;
                        match kind {
                            Kind::Main => s.main = Some(p.clone()),
                            Kind::Include => s.include = Some(p.clone()),
                        }
                    }
                });
                with_cache(add_key);
                return Ok(p);
            }
            None => {}
        }
        // A miss (opcache_compile_file + cache_script_in_shared_memory).
        let full = with_cache(|c| {
            c.misses += 1;
            c.memory_exhausted || c.restart_pending.is_some()
        });
        let src = source();
        let cacheable = if full {
            None
        } else {
            self.opcache_cacheable(path, request_time)
        };
        let program = Rc::new(compile(self, &src)?);
        if let Some(timestamp) = cacheable {
            self.opcache_store(path, kind, &program, if validate { timestamp } else { 0 });
            with_cache(|c| {
                if let Some(s) = c.scripts.get_mut(path) {
                    s.revalidate = request_time + freq;
                    s.last_used = request_time;
                }
                add_key(c);
            });
        }
        Ok(program)
    }

    /// `accel_make_persistent_key`: the name a script was requested by, when
    /// it isn't its resolved path. A relative name is qualified by what it
    /// was resolved under: the working directory, the include_path and the
    /// executing script's directory.
    fn persistent_key(&self, given: &[u8], path: &[u8]) -> Option<Vec<u8>> {
        if given == path || given.is_empty() {
            return None;
        }
        if given.starts_with(b"/") {
            return Some(given.to_vec());
        }
        let mut k = given.to_vec();
        k.push(b':');
        if let Ok(cwd) = std::env::current_dir() {
            k.extend_from_slice(&path_bytes(&cwd));
        }
        let include_path = self.ini(b"include_path");
        if !include_path.is_empty() {
            k.push(b':');
            k.extend_from_slice(&include_path);
        }
        // zend_get_executed_filename_ex: the innermost user frame's file.
        let user = (0..self.stack.len())
            .rev()
            .find(|&i| !self.is_internal_frame(i));
        if let Some(i) = user {
            let file = self.current_file(i);
            let dir = file.iter().rposition(|&c| c == b'/').unwrap_or(0);
            k.push(b':');
            k.extend_from_slice(&file[..dir]);
        }
        Some(k)
    }

    /// Whether a just-compiled file may be cached, and its mtime: not
    /// blacklisted, not too new (file_update_protection), not too big.
    fn opcache_cacheable(&mut self, path: &[u8], request_time: i64) -> Option<i64> {
        let black = with_cache(|c| {
            let hit = c
                .startup
                .as_ref()
                .is_some_and(|s| blacklisted(&s.blacklist, path));
            if hit {
                c.blacklist_misses += 1;
            }
            hit
        });
        if black {
            return None;
        }
        let validate = ini_flag(&self.ini(b"opcache.validate_timestamps"));
        let protect = self.ini_long(b"opcache.file_update_protection");
        let max_size = self.ini_long(b"opcache.max_file_size");
        if !(validate || protect != 0 || max_size > 0) {
            return Some(0);
        }
        let (mtime, size) = mtime_and_size(path)?;
        if mtime == 0 || (protect != 0 && request_time - protect < mtime) {
            return None;
        }
        if max_size > 0 && size > max_size as u64 {
            with_cache(|c| c.blacklist_misses += 1);
            return None;
        }
        Some(mtime)
    }

    fn opcache_store(&mut self, path: &[u8], kind: Kind, program: &Rc<Program>, timestamp: i64) {
        let size = program_size(program);
        with_cache(|c| {
            let Some(st) = c.startup.as_ref() else {
                return;
            };
            let (max_entries, budget, wasted_max) = (
                st.max_entries,
                st.memory_consumption,
                st.max_wasted_percentage,
            );
            let base = BASE_SIZE + st.interned_strings_buffer + max_entries * HASH_ENTRY_SIZE;
            let existing = c.scripts.contains_key(path);
            if !existing && (c.scripts.len() + c.keys.len()) as i64 >= max_entries {
                c.log
                    .push((LOG_DEBUG, "No more entries in hash table!".into()));
                c.memory_exhausted = true;
                c.schedule_restart_if_necessary(Restart::Hash, budget, wasted_max);
                return;
            }
            if base + c.used + c.wasted + size > budget {
                c.memory_exhausted = true;
                c.schedule_restart_if_necessary(Restart::Oom, budget, wasted_max);
                return;
            }
            c.used += size;
            c.seq += 1;
            let seq = c.seq;
            let script = Script {
                main: (kind == Kind::Main).then(|| program.clone()),
                include: (kind == Kind::Include).then(|| program.clone()),
                timestamp,
                revalidate: 0,
                hits: 0,
                last_used: 0,
                memory: size,
                corrupted: false,
                seq,
            };
            let shown = String::from_utf8_lossy(path).into_owned();
            c.log.push((LOG_INFO, format!("Cached script '{shown}'")));
            match c.scripts.get_mut(path) {
                // php updates a discarded entry in place.
                Some(old) => {
                    let seq = old.seq;
                    *old = Script { seq, ..script };
                }
                None => {
                    c.scripts.insert(path.to_vec(), script);
                }
            }
        });
    }

    /// The `opcache_*` functions, once the accelerator is started: `None`
    /// when it isn't (the builtins then answer as php does without it).
    pub(crate) fn opcache_op(
        &mut self,
        op: &str,
        args: &[Value],
    ) -> Option<Result<Value, rhp_values::Thrown>> {
        if !self.opcache_request().started {
            return None;
        }
        let path_arg = || match args.first() {
            Some(Value::Str(s)) => s.as_bytes().to_vec(),
            _ => Vec::new(),
        };
        let bool_arg = |i: usize| matches!(args.get(i), Some(Value::Bool(true)));
        Some(Ok(match op {
            "get_status" => self.opcache_status(bool_arg(0)),
            "is_script_cached" => Value::Bool(self.opcache_is_cached(&path_arg())),
            "invalidate" => Value::Bool(self.opcache_invalidate(&path_arg(), bool_arg(1))),
            "reset" => Value::Bool(self.opcache_reset()),
            "compile_file" => return Some(self.opcache_compile_file(&path_arg())),
            "blacklist" => Value::Array(with_cache(|c| {
                let mut a = PhpArray::new();
                for e in c.startup.iter().flat_map(|s| &s.blacklist) {
                    a.push(Value::str(e.clone()));
                }
                a
            })),
            _ => return None,
        }))
    }

    /// `zend_resolve_path`: the real path include would open, or None.
    fn opcache_resolve(&self, path: &[u8]) -> Option<Vec<u8>> {
        let top = self.stack.len().checked_sub(1)?;
        let p = String::from_utf8_lossy(path).into_owned();
        self.resolve_include_file(top, &p).map(|(rp, _)| rp)
    }

    /// An include of an absolute path that is a cached script, still valid:
    /// php finds it by that name and needs neither realpath nor open (only
    /// the timestamp check's stat).
    pub(crate) fn opcache_fast_hit(&mut self, given: &[u8]) -> bool {
        if !given.starts_with(b"/") || !self.opcache_on() {
            return false;
        }
        let validate = ini_flag(&self.ini(b"opcache.validate_timestamps"));
        let freq = self.ini_long(b"opcache.revalidate_freq");
        let request_time = self.opcache_request().request_time;
        with_cache(|c| {
            let Some(s) = c
                .scripts
                .get_mut(given)
                .filter(|s| !s.corrupted && s.include.is_some())
            else {
                return false;
            };
            if validate && s.timestamp != 0 && !(freq != 0 && s.revalidate >= request_time) {
                // A changed file goes the long way: discarded, recompiled.
                if mtime_and_size(given).is_none_or(|(m, _)| m != s.timestamp) {
                    return false;
                }
                s.revalidate = request_time + freq;
            }
            true
        })
    }

    /// Whether the cache has a script under the name `given` (resolving to
    /// `path`): php then doesn't open the file to look it up.
    pub(crate) fn opcache_has_name(&mut self, given: &[u8], path: &[u8]) -> bool {
        if !self.opcache_on() {
            return false;
        }
        let key = self.persistent_key(given, path);
        with_cache(|c| match &key {
            None => c.scripts.get(path).is_some_and(|s| !s.corrupted),
            Some(k) => c.keys.get(k).is_some_and(|p| p == path),
        })
    }

    /// The resolved path of a cached script requested by `given`, when it
    /// is served without a stat (timestamps not validated, within
    /// `revalidate_freq`, or preloaded).
    pub(crate) fn opcache_cached_name(&mut self, given: &[u8]) -> Option<Vec<u8>> {
        if given.is_empty() || !self.opcache_on() {
            return None;
        }
        let validate = ini_flag(&self.ini(b"opcache.validate_timestamps"));
        let freq = self.ini_long(b"opcache.revalidate_freq");
        let request_time = self.opcache_request().request_time;
        let key = self.persistent_key(given, b"").unwrap_or_default();
        with_cache(|c| {
            let real = if c.scripts.contains_key(given) {
                given.to_vec()
            } else {
                c.keys.get(&key)?.clone()
            };
            let s = c.scripts.get(&real)?;
            let fresh =
                !validate || s.timestamp == 0 || (freq != 0 && s.revalidate >= request_time);
            (!s.corrupted && s.include.is_some() && fresh).then_some(real)
        })
    }

    /// `filename_is_in_cache`: found by the name as given (an absolute
    /// path, or a relative one some include used), never by resolving it.
    fn opcache_is_cached(&mut self, path: &[u8]) -> bool {
        if !self.opcache_on() || path.is_empty() {
            return false;
        }
        let key = self.persistent_key(path, b"").unwrap_or_default();
        let Some(real) = with_cache(|c| {
            if c.scripts.contains_key(path) {
                Some(path.to_vec())
            } else {
                c.keys.get(&key).cloned()
            }
        }) else {
            return false;
        };
        let validate = ini_flag(&self.ini(b"opcache.validate_timestamps"));
        let freq = self.ini_long(b"opcache.revalidate_freq");
        let request_time = self.opcache_request().request_time;
        with_cache(|c| {
            let Some(s) = c.scripts.get_mut(&real).filter(|s| !s.corrupted) else {
                return false;
            };
            if !validate || s.timestamp == 0 || (freq != 0 && s.revalidate >= request_time) {
                return true;
            }
            if mtime_and_size(&real).is_some_and(|(m, _)| m == s.timestamp) {
                s.revalidate = request_time + freq;
                true
            } else {
                false
            }
        })
    }

    /// `zend_accel_invalidate`: discard a script when forced, when not
    /// validating timestamps, or when its file changed. True when the file
    /// was found in the cache (or resolves at all).
    fn opcache_invalidate(&mut self, path: &[u8], force: bool) -> bool {
        if !self.opcache_on() {
            return false;
        }
        let (real, mut found) = match self.opcache_resolve(path) {
            Some(r) => (r, true),
            None => (path.to_vec(), false),
        };
        let validate = ini_flag(&self.ini(b"opcache.validate_timestamps"));
        with_cache(|c| {
            if let Some(s) = c.scripts.get_mut(&real).filter(|s| !s.corrupted) {
                let stale = || mtime_and_size(&real).is_none_or(|(m, _)| m != s.timestamp);
                if force || !validate || s.timestamp == 0 || stale() {
                    s.corrupted = true;
                    s.main = None;
                    s.include = None;
                    c.wasted += s.memory;
                }
                found = true;
            }
        });
        found
    }

    /// `opcache_reset`: schedule a restart; it happens when the next
    /// request starts (until then cached scripts are still served, and
    /// nothing new is cached).
    fn opcache_reset(&mut self) -> bool {
        // php: ZCG(enabled), and the shared cache not already restarting.
        if !self.opcache_request().started
            || !ini_flag(&self.ini(b"opcache.enable"))
            || with_cache(|c| c.disabled)
        {
            return false;
        }
        with_cache(|c| c.schedule_restart(Restart::User));
        self.flush_accel_log();
        true
    }

    /// `opcache_compile_file`: compile and cache a file without running
    /// it. Like php, its functions and classes are declared in the
    /// current request.
    fn opcache_compile_file(&mut self, path: &[u8]) -> Result<Value, rhp_values::Thrown> {
        let top = self.stack.len().saturating_sub(1);
        let p = String::from_utf8_lossy(path).into_owned();
        let Some((real, file)) = self.resolve_include_file(top, &p) else {
            let ip = String::from_utf8_lossy(&self.ini(b"include_path")).into_owned();
            self.diag(
                2,
                &format!(
                    "opcache_compile_file({p}): Failed to open stream: No such file or directory"
                ),
            )?;
            self.diag(
                2,
                &format!("opcache_compile_file(): Failed opening '{p}' for inclusion (include_path='{ip}')"),
            )?;
            return Ok(Value::Bool(false));
        };
        let prog = self.compile_included_file(&real, path, file)?;
        self.install_file(&real, &prog)?;
        Ok(Value::Bool(true))
    }

    /// `opcache_get_status()`.
    fn opcache_status(&mut self, scripts: bool) -> Value {
        let enabled = self.opcache_on();
        let validate = ini_flag(&self.ini(b"opcache.validate_timestamps"));
        let jit = self.jit_status();
        let s = |b: &[u8]| ArrayKey::from_bytes(b);
        with_cache(|c| {
            let st = c.startup.as_ref();
            let budget = st.map_or(0, |s| s.memory_consumption);
            let strings = st.map_or(0, |s| s.interned_strings_buffer);
            let max_entries = st.map_or(0, |s| s.max_entries);
            let base = BASE_SIZE + strings + max_entries * HASH_ENTRY_SIZE;
            let used = (base + c.used).min(budget);
            let free = (budget - used - c.wasted).max(0);
            let mut out = PhpArray::new();
            out.insert(s(b"opcache_enabled"), Value::Bool(enabled));
            out.insert(s(b"cache_full"), Value::Bool(c.memory_exhausted));
            out.insert(
                s(b"restart_pending"),
                Value::Bool(c.restart_pending.is_some()),
            );
            out.insert(s(b"restart_in_progress"), Value::Bool(false));
            let mut mem = PhpArray::new();
            mem.insert(s(b"used_memory"), Value::Int(used));
            mem.insert(s(b"free_memory"), Value::Int(free));
            mem.insert(s(b"wasted_memory"), Value::Int(c.wasted));
            mem.insert(
                s(b"current_wasted_percentage"),
                Value::Float(if budget > 0 {
                    c.wasted as f64 / budget as f64 * 100.0
                } else {
                    0.0
                }),
            );
            out.insert(s(b"memory_usage"), Value::Array(mem));
            if strings > 0 {
                let (count, bytes) = c.interned_strings();
                let used = bytes.min(strings);
                let mut is = PhpArray::new();
                is.insert(s(b"buffer_size"), Value::Int(strings));
                is.insert(s(b"used_memory"), Value::Int(used));
                is.insert(s(b"free_memory"), Value::Int(strings - used));
                is.insert(s(b"number_of_strings"), Value::Int(count));
                out.insert(s(b"interned_strings_usage"), Value::Array(is));
            }
            let reqs = c.hits + c.misses;
            let ratio = |n: i64| {
                if reqs != 0 {
                    n as f64 / reqs as f64 * 100.0
                } else {
                    0.0
                }
            };
            let mut stats = PhpArray::new();
            let n = c.scripts.len() as i64;
            stats.insert(s(b"num_cached_scripts"), Value::Int(n));
            stats.insert(s(b"num_cached_keys"), Value::Int(n + c.keys.len() as i64));
            stats.insert(s(b"max_cached_keys"), Value::Int(max_entries));
            stats.insert(s(b"hits"), Value::Int(c.hits));
            stats.insert(s(b"start_time"), Value::Int(c.start_time));
            stats.insert(s(b"last_restart_time"), Value::Int(c.last_restart_time));
            stats.insert(s(b"oom_restarts"), Value::Int(c.oom_restarts));
            stats.insert(s(b"hash_restarts"), Value::Int(c.hash_restarts));
            stats.insert(s(b"manual_restarts"), Value::Int(c.manual_restarts));
            let misses = if c.memory_exhausted {
                c.misses
            } else {
                c.misses - c.blacklist_misses
            };
            stats.insert(s(b"misses"), Value::Int(misses));
            stats.insert(s(b"blacklist_misses"), Value::Int(c.blacklist_misses));
            stats.insert(
                s(b"blacklist_miss_ratio"),
                Value::Float(ratio(c.blacklist_misses)),
            );
            stats.insert(s(b"opcache_hit_rate"), Value::Float(ratio(c.hits)));
            out.insert(s(b"opcache_statistics"), Value::Array(stats));
            if let Some(pre) = &c.preload {
                out.insert(s(b"preload_statistics"), Value::Array(preload_report(pre)));
            }
            if scripts && enabled {
                out.insert(
                    s(b"scripts"),
                    Value::Array(c.scripts_report(validate, max_entries)),
                );
            }
            out.insert(s(b"jit"), Value::Array(jit));
            Value::Array(out)
        })
    }

    /// `zend_jit_status` for the configured JIT. rhp has no JIT; this
    /// reports what php's settings select (`opcache.jit`,
    /// `opcache.jit_buffer_size`), as php's JIT globals hold them.
    fn jit_status(&self) -> PhpArray {
        let startup = |n: &[u8]| builtins::ini_startup_value(n).unwrap_or_default();
        let mode = String::from_utf8_lossy(&self.ini(b"opcache.jit")).to_ascii_lowercase();
        let buffer = builtins::ini_parse_quantity(&startup(b"opcache.jit_buffer_size"));
        // (enabled, on, trigger, opt_level, opt_flags): zend_jit_config.
        let (mut enabled, mut on, kind, level, flags) = match mode.as_str() {
            "disable" => (false, false, 0, 0, 0),
            "" | "0" | "off" | "no" | "false" => (true, false, 0, 0, 0),
            "1" | "on" | "yes" | "true" | "tracing" => (true, true, 5, 4, 6),
            "function" => (true, true, 0, 5, 6),
            num => match parse_jit_num(num) {
                Some((on, kind, level, flags)) => (true, on, kind, level, flags),
                None => (false, false, 0, 0, 0),
            },
        };
        if buffer <= 0 {
            enabled = false;
            on = false;
        }
        // php's buffer is the setting less a 16-byte header; its stubs
        // take the first 3640 bytes.
        let size = if enabled { (buffer - 16).max(0) } else { 0 };
        let free = if enabled { (size - 3640).max(0) } else { 0 };
        let mut a = PhpArray::new();
        let s = |b: &[u8]| ArrayKey::from_bytes(b);
        a.insert(s(b"enabled"), Value::Bool(enabled));
        a.insert(s(b"on"), Value::Bool(on));
        a.insert(s(b"kind"), Value::Int(kind));
        a.insert(s(b"opt_level"), Value::Int(level));
        a.insert(s(b"opt_flags"), Value::Int(flags));
        a.insert(s(b"buffer_size"), Value::Int(size));
        a.insert(s(b"buffer_free"), Value::Int(free));
        a
    }
}

/// `zend_jit_parse_config_num`: a 4-digit `opcache.jit` ("CRTO").
fn parse_jit_num(s: &str) -> Option<(bool, i64, i64, i64)> {
    let mut jit: i64 = s.parse().ok()?;
    if jit == 0 {
        return Some((false, 0, 0, 0));
    }
    if jit < 0 || jit % 10 == 0 || jit % 10 > 5 {
        return None;
    }
    let level = jit % 10;
    jit /= 10;
    if jit % 10 > 5 || jit % 10 == 4 {
        return None;
    }
    let kind = jit % 10;
    jit /= 10;
    if jit % 10 > 2 {
        return None;
    }
    let mut flags = jit % 10;
    jit /= 10;
    if jit % 10 > 1 {
        return None;
    }
    if jit % 10 == 1 {
        flags |= 4;
    }
    if jit / 10 != 0 {
        return None;
    }
    Some((true, kind, level, flags))
}

impl Cache {
    fn schedule_restart(&mut self, reason: Restart) {
        if self.restart_pending.is_none() {
            self.restart_pending = Some(reason);
            self.disabled = true;
            let why = match reason {
                Restart::Oom => "out of memory",
                Restart::Hash => "hash overflow",
                Restart::User => "user",
            };
            self.log
                .push((LOG_DEBUG, format!("Restart Scheduled! Reason: {why}")));
        }
    }

    fn schedule_restart_if_necessary(&mut self, reason: Restart, budget: i64, max: f64) {
        if budget > 0 && self.wasted as f64 / budget as f64 >= max {
            self.schedule_restart(reason);
        }
    }

    /// Distinct names in the cached programs, and the bytes they'd take
    /// as interned strings (a `zend_string` header plus the bytes).
    fn interned_strings(&self) -> (i64, i64) {
        let mut seen = std::collections::HashSet::new();
        let mut add = |b: &[u8]| {
            seen.insert(b.to_vec());
        };
        fn walk(f: &FunctionProto, add: &mut dyn FnMut(&[u8])) {
            add(f.name.as_bytes());
            for n in &f.names {
                add(n.as_bytes());
            }
        }
        for s in self.scripts.values() {
            for p in [&s.main, &s.include].into_iter().flatten() {
                walk(&p.main, &mut add);
                for f in p.functions.iter().skip(p.num_prelude_functions) {
                    walk(f, &mut add);
                }
                for c in p.classes.iter().skip(p.num_prelude_classes) {
                    add(c.name.as_bytes());
                    for m in &c.methods {
                        walk(&m.proto, &mut add);
                    }
                }
            }
        }
        let bytes = seen.iter().map(|s| ((s.len() + 25 + 7) & !7) as i64).sum();
        (seen.len() as i64, bytes)
    }

    /// `accelerator_get_scripts`: in php's hash-table order (bucket index,
    /// newest first within a bucket).
    fn scripts_report(&self, validate: bool, max_entries: i64) -> PhpArray {
        let mut order: Vec<(&Vec<u8>, &Script)> = self.scripts.iter().collect();
        let m = max_entries.max(1) as u64;
        order.sort_by(|a, b| {
            (zend_hash(a.0) % m)
                .cmp(&(zend_hash(b.0) % m))
                .then(b.1.seq.cmp(&a.1.seq))
        });
        let mut out = PhpArray::new();
        for (path, s) in order {
            let k = |b: &[u8]| ArrayKey::from_bytes(b);
            let mut r = PhpArray::new();
            r.insert(k(b"full_path"), Value::str(path.clone()));
            r.insert(k(b"hits"), Value::Int(s.hits));
            r.insert(k(b"memory_consumption"), Value::Int(s.memory));
            r.insert(k(b"last_used"), Value::str(asctime(s.last_used)));
            r.insert(k(b"last_used_timestamp"), Value::Int(s.last_used));
            if validate {
                r.insert(k(b"timestamp"), Value::Int(s.timestamp));
            }
            r.insert(k(b"revalidate"), Value::Int(s.revalidate));
            out.insert(ArrayKey::from_bytes(path), Value::Array(r));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blacklist_entries_are_prefixes_with_globs() {
        let e = vec![
            b"/a/b".to_vec(),
            b"/t/?x.inc".to_vec(),
            b"/w/*/d".to_vec(),
            b"/z/**.php".to_vec(),
        ];
        assert!(blacklisted(&e, b"/a/b/c.php"));
        assert!(blacklisted(&e, b"/a/bc.php"));
        assert!(blacklisted(&e, b"/t/1x.inc"));
        assert!(!blacklisted(&e, b"/t/12x.inc"));
        assert!(blacklisted(&e, b"/w/q/d/e.php"));
        assert!(!blacklisted(&e, b"/w/q/r/d"));
        assert!(blacklisted(&e, b"/z/q/r.php"));
        assert!(!blacklisted(&e, b"/y"));
    }

    #[test]
    fn expands_like_expand_filepath() {
        assert_eq!(expand_path(b"./current.php", b"/d"), b"/d/current.php");
        assert_eq!(expand_path(b"a/../b", b"/d"), b"/d/b");
        assert_eq!(expand_path(b"/tmp/path/*/x", b"/d"), b"/tmp/path/*/x");
    }

    #[test]
    fn jit_numbers() {
        assert_eq!(parse_jit_num("1205"), Some((true, 0, 5, 6)));
        assert_eq!(parse_jit_num("1255"), Some((true, 5, 5, 6)));
        assert_eq!(parse_jit_num("0"), Some((false, 0, 0, 0)));
        assert_eq!(parse_jit_num("1240"), None);
    }

    #[test]
    fn php_hash() {
        // zend_inline_hash_func("a") = 5381 * 33 + 97.
        assert_eq!(zend_hash(b"a"), (5381 * 33 + 97) | (1 << 63));
    }
}
