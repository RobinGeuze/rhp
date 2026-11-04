//! Standard-library builtins that tooling (phpunit, psalm, php-cs-fixer,
//! rector) needs: the seeded Mt19937 engine (`mt_srand`/`mt_rand`/`rand`,
//! `shuffle`, `array_rand`, `str_shuffle`), `array_change_key_case`,
//! `array_multisort`, `settype`, `fnmatch`, process introspection
//! (`getrusage`, `gc_status`, `sys_getloadavg`, `memory_reset_peak_usage`,
//! `gc_mem_caches`), `getopt` and `readline`.

use std::cell::RefCell;

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::zend::zpp::{int_or_null_param, int_param, str_param};
use crate::{Host, Thrown};

// --- Mt19937 (ext/random/engine_mt19937.c) ----------------------------------

const MT_N: usize = 624;
const MT_M: usize = 397;
const MT_RAND_MT19937: i64 = 0;
const MT_RAND_PHP: i64 = 1;

struct Mt {
    state: [u32; MT_N],
    count: usize,
    mode: i64,
}

#[inline]
fn twist(m: u32, u: u32, v: u32, php: bool) -> u32 {
    let mix = (u & 0x8000_0000) | (v & 0x7fff_ffff);
    let lo = if php { u & 1 } else { v & 1 };
    m ^ (mix >> 1) ^ ((lo as i32).wrapping_neg() as u32 & 0x9908_b0df)
}

impl Mt {
    fn seeded(seed: u32, mode: i64) -> Mt {
        let mut mt = Mt {
            state: [0; MT_N],
            count: 0,
            mode,
        };
        mt.seed(seed);
        mt
    }

    fn seed(&mut self, seed: u32) {
        self.state[0] = seed;
        for i in 1..MT_N {
            let prev = self.state[i - 1];
            self.state[i] = 1_812_433_253u32
                .wrapping_mul(prev ^ (prev >> 30))
                .wrapping_add(i as u32);
        }
        self.reload();
    }

    fn reload(&mut self) {
        let php = self.mode == MT_RAND_PHP;
        let s = &mut self.state;
        for i in 0..MT_N - MT_M {
            s[i] = twist(s[i + MT_M], s[i], s[i + 1], php);
        }
        for i in MT_N - MT_M..MT_N - 1 {
            s[i] = twist(s[i + MT_M - MT_N], s[i], s[i + 1], php);
        }
        s[MT_N - 1] = twist(s[MT_M - 1], s[MT_N - 1], s[0], php);
        self.count = 0;
    }

    fn next32(&mut self) -> u32 {
        if self.count >= MT_N {
            self.reload();
        }
        let mut s1 = self.state[self.count];
        self.count += 1;
        s1 ^= s1 >> 11;
        s1 ^= (s1 << 7) & 0x9d2c_5680;
        s1 ^= (s1 << 15) & 0xefc6_0000;
        s1 ^ (s1 >> 18)
    }

    /// `php_random_range32`.
    fn range32(&mut self, umax: u32) -> u32 {
        let mut result = self.next32();
        if umax == u32::MAX {
            return result;
        }
        let umax = umax + 1;
        if umax & (umax - 1) == 0 {
            return result & (umax - 1);
        }
        let limit = u32::MAX - (u32::MAX % umax) - 1;
        while result > limit {
            result = self.next32();
        }
        result % umax
    }

    /// `php_random_range64` (two 32-bit draws per 64-bit result).
    fn range64(&mut self, umax: u64) -> u64 {
        let draw = |mt: &mut Mt| (mt.next32() as u64) | ((mt.next32() as u64) << 32);
        let mut result = draw(self);
        if umax == u64::MAX {
            return result;
        }
        let umax = umax + 1;
        if umax & (umax - 1) == 0 {
            return result & (umax - 1);
        }
        let limit = u64::MAX - (u64::MAX % umax) - 1;
        while result > limit {
            result = draw(self);
        }
        result % umax
    }

    /// `php_random_range`: uniform in `[min, max]`.
    fn range(&mut self, min: i64, max: i64) -> i64 {
        let umax = (max as u64).wrapping_sub(min as u64);
        if umax > u32::MAX as u64 {
            return (self.range64(umax).wrapping_add(min as u64)) as i64;
        }
        (self.range32(umax as u32) as u64).wrapping_add(min as u64) as i64
    }
}

thread_local! {
    static MT: RefCell<Option<Mt>> = const { RefCell::new(None) };
}

pub(crate) fn reset_state() {
    MT.with(|m| *m.borrow_mut() = None);
}

fn random_seed() -> u32 {
    let mut b = [0u8; 4];
    let _ = getrandom::getrandom(&mut b);
    u32::from_le_bytes(b)
}

/// Run `f` on the default (lazily, randomly seeded) Mt19937 state.
fn with_mt<R>(f: impl FnOnce(&mut Mt) -> R) -> R {
    MT.with(|m| {
        let mut m = m.borrow_mut();
        let mt = m.get_or_insert_with(|| Mt::seeded(random_seed(), MT_RAND_MT19937));
        f(mt)
    })
}

/// `php_mt_rand_range`: the engine behind shuffle/array_rand/str_shuffle.
pub(crate) fn mt_range(min: i64, max: i64) -> i64 {
    with_mt(|mt| mt.range(min, max))
}

/// `php_mt_rand_common`: rand()/mt_rand() with a range (honours MT_RAND_PHP).
fn mt_rand_common(min: i64, max: i64) -> i64 {
    with_mt(|mt| {
        if mt.mode == MT_RAND_MT19937 {
            return mt.range(min, max);
        }
        let r = (mt.next32() >> 1) as f64;
        let offset = ((max as f64 - min as f64 + 1.0) * (r / (2147483647.0 + 1.0))) as u64;
        offset.wrapping_add(min as u64) as i64
    })
}

fn fn_mt_srand(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let seed = int_or_null_param(host, args, 0, "mt_srand", "seed")?;
    let mode = int_param(host, args, 1, "mt_srand", "mode", MT_RAND_MT19937)?;
    let mode = if mode == MT_RAND_PHP {
        host.emit_diagnostic(
            crate::E_DEPRECATED,
            "The MT_RAND_PHP variant of Mt19937 is deprecated",
        )?;
        MT_RAND_PHP
    } else {
        MT_RAND_MT19937
    };
    let seed = seed.map(|s| s as u32).unwrap_or_else(random_seed);
    MT.with(|m| *m.borrow_mut() = Some(Mt::seeded(seed, mode)));
    Ok(Value::Null)
}

fn fn_mt_rand(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if args.is_empty() {
        return Ok(Value::Int((with_mt(|mt| mt.next32()) >> 1) as i64));
    }
    if args.len() == 1 {
        return Err(Thrown::error(
            "ArgumentCountError",
            "mt_rand() expects exactly 2 arguments, 1 given",
        ));
    }
    let min = int_param(host, args, 0, "mt_rand", "min", 0)?;
    let max = int_param(host, args, 1, "mt_rand", "max", 0)?;
    if max < min {
        return Err(Thrown::error(
            "ValueError",
            "mt_rand(): Argument #2 ($max) must be greater than or equal to argument #1 ($min)",
        ));
    }
    Ok(Value::Int(mt_rand_common(min, max)))
}

fn fn_rand(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if args.is_empty() {
        return Ok(Value::Int((with_mt(|mt| mt.next32()) >> 1) as i64));
    }
    if args.len() == 1 {
        return Err(Thrown::error(
            "ArgumentCountError",
            "rand() expects exactly 2 arguments, 1 given",
        ));
    }
    let min = int_param(host, args, 0, "rand", "min", 0)?;
    let max = int_param(host, args, 1, "rand", "max", 0)?;
    Ok(Value::Int(if max < min {
        mt_rand_common(max, min)
    } else {
        mt_rand_common(min, max)
    }))
}

// --- filesystem --------------------------------------------------------------

const MAXPATHLEN: usize = libc::PATH_MAX as usize;

fn fn_fnmatch(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let pattern = str_param(host, args, 0, "fnmatch", "pattern")?;
    let filename = str_param(host, args, 1, "fnmatch", "filename")?;
    let flags = int_param(host, args, 2, "fnmatch", "flags", 0)?;
    if pattern.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            "fnmatch(): Argument #1 ($pattern) must not contain any null bytes",
        ));
    }
    if filename.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            "fnmatch(): Argument #2 ($filename) must not contain any null bytes",
        ));
    }
    if filename.len() >= MAXPATHLEN {
        crate::warn(
            host,
            "fnmatch",
            &format!("Filename exceeds the maximum allowed length of {MAXPATHLEN} characters"),
        )?;
        return Ok(Value::Bool(false));
    }
    if pattern.len() >= MAXPATHLEN {
        crate::warn(
            host,
            "fnmatch",
            &format!("Pattern exceeds the maximum allowed length of {MAXPATHLEN} characters"),
        )?;
        return Ok(Value::Bool(false));
    }
    let p = std::ffi::CString::new(pattern).unwrap();
    let f = std::ffi::CString::new(filename).unwrap();
    let rc = unsafe { libc::fnmatch(p.as_ptr(), f.as_ptr(), flags as libc::c_int) };
    Ok(Value::Bool(rc == 0))
}

// --- process introspection ---------------------------------------------------

fn fn_getrusage(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mode = int_param(host, args, 0, "getrusage", "mode", 0)?;
    let who = if mode == 1 {
        libc::RUSAGE_CHILDREN
    } else {
        libc::RUSAGE_SELF
    };
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(who, &mut u) } == -1 {
        return Ok(Value::Bool(false));
    }
    let mut a = PhpArray::new();
    let mut put = |k: &str, v: i64| a.insert(ArrayKey::from_bytes(k.as_bytes()), Value::Int(v));
    put("ru_oublock", u.ru_oublock as i64);
    put("ru_inblock", u.ru_inblock as i64);
    put("ru_msgsnd", u.ru_msgsnd as i64);
    put("ru_msgrcv", u.ru_msgrcv as i64);
    put("ru_maxrss", u.ru_maxrss as i64);
    put("ru_ixrss", u.ru_ixrss as i64);
    put("ru_idrss", u.ru_idrss as i64);
    put("ru_minflt", u.ru_minflt as i64);
    put("ru_majflt", u.ru_majflt as i64);
    put("ru_nsignals", u.ru_nsignals as i64);
    put("ru_nvcsw", u.ru_nvcsw as i64);
    put("ru_nivcsw", u.ru_nivcsw as i64);
    put("ru_nswap", u.ru_nswap as i64);
    put("ru_utime.tv_usec", u.ru_utime.tv_usec as i64);
    put("ru_utime.tv_sec", u.ru_utime.tv_sec as i64);
    put("ru_stime.tv_usec", u.ru_stime.tv_usec as i64);
    put("ru_stime.tv_sec", u.ru_stime.tv_sec as i64);
    Ok(Value::Array(a))
}

fn fn_sys_getloadavg(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut l = [0f64; 3];
    if unsafe { libc::getloadavg(l.as_mut_ptr(), 3) } == -1 {
        return Ok(Value::Bool(false));
    }
    let mut a = PhpArray::new();
    for v in l {
        a.push(Value::Float(v));
    }
    Ok(Value::Array(a))
}

// --- getopt (main/getopt.c + ext/standard/basic_functions.c) -----------------

struct OptSpec {
    ch: u8,
    name: Option<Vec<u8>>,
    need_param: u8,
}

/// `parse_opts`: the leading run of alphanumeric option letters, each
/// optionally followed by `:` (required value) or `::` (optional value).
fn parse_short_opts(s: &[u8]) -> Vec<OptSpec> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() && s[i].is_ascii_alphanumeric() {
        let ch = s[i];
        i += 1;
        let mut need = 0;
        if s.get(i) == Some(&b':') {
            need = 1;
            i += 1;
            if s.get(i) == Some(&b':') {
                need = 2;
                i += 1;
            }
        }
        out.push(OptSpec {
            ch,
            name: None,
            need_param: need,
        });
    }
    out
}

/// One argv string as C sees it (cut at the first NUL), indexable past the end.
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

enum Got {
    Eof,
    Invalid,
    Opt(usize, Option<Vec<u8>>),
}

struct GetoptState {
    optchr: usize,
    dash: bool,
}

/// `php_getopt` for one option (show_err = 0, arg_start = 1).
fn php_getopt(argv: &[Vec<u8>], opts: &[OptSpec], optind: &mut usize, st: &mut GetoptState) -> Got {
    let argc = argv.len();
    if *optind >= argc {
        return Got::Eof;
    }
    let cur = |optind: usize| argv[optind].as_slice();
    if !st.dash {
        if at(cur(*optind), 0) != b'-' {
            return Got::Eof;
        }
        if at(cur(*optind), 1) == 0 {
            return Got::Eof;
        }
    }
    let mut idx: Option<usize> = None;
    let mut arg_start: usize;
    let a = cur(*optind);
    if at(a, 0) == b'-' && at(a, 1) == b'-' {
        if at(a, 2) == 0 {
            *optind += 1;
            return Got::Eof;
        }
        arg_start = 2;
        let mut arg_end = a.len() - 1;
        let search_len = arg_end - arg_start;
        if let Some(p) = a[arg_start..arg_start + search_len]
            .iter()
            .position(|&b| b == b'=')
        {
            arg_end = p;
            arg_start += 1;
        } else {
            arg_end -= 1;
        }
        let rest = &a[2..];
        // strncmp(argv + 2, name, arg_end) == 0 && arg_end == strlen(name)
        let found = opts.iter().position(|o| {
            o.name
                .as_deref()
                .is_some_and(|n| n.len() == arg_end && rest.starts_with(n))
        });
        match found {
            Some(i) => idx = Some(i),
            None => {
                *optind += 1;
                return Got::Invalid;
            }
        }
        st.optchr = 0;
        st.dash = false;
        arg_start += opts[idx.unwrap()].name.as_ref().unwrap().len();
    } else {
        if !st.dash {
            st.dash = true;
            st.optchr = 1;
        }
        if at(a, st.optchr) == b':' {
            st.dash = false;
            *optind += 1;
            return Got::Invalid;
        }
        arg_start = 1 + st.optchr;
    }
    let idx = match idx {
        Some(i) => i,
        None => {
            let c = at(a, st.optchr);
            match opts.iter().position(|o| o.name.is_none() && o.ch == c) {
                Some(i) => i,
                None => {
                    if at(a, st.optchr + 1) == 0 {
                        st.dash = false;
                        *optind += 1;
                    } else {
                        st.optchr += 1;
                    }
                    return Got::Invalid;
                }
            }
        }
    };
    let o = &opts[idx];
    if o.need_param > 0 {
        st.dash = false;
        let mut optarg = None;
        if at(a, arg_start) == 0 {
            *optind += 1;
            if *optind == argc {
                if o.need_param == 1 {
                    return Got::Invalid;
                }
            } else if o.need_param == 1 {
                optarg = Some(argv[*optind].clone());
                *optind += 1;
            }
        } else if at(a, arg_start) == b'=' {
            optarg = Some(a[arg_start + 1..].to_vec());
            *optind += 1;
        } else {
            optarg = Some(a[arg_start..].to_vec());
            *optind += 1;
        }
        Got::Opt(idx, optarg)
    } else {
        if arg_start >= 2 && !(at(a, 0) == b'-' && at(a, 1) == b'-') {
            if at(a, st.optchr + 1) == 0 {
                st.dash = false;
                *optind += 1;
            } else {
                st.optchr += 1;
            }
        } else {
            *optind += 1;
        }
        Got::Opt(idx, None)
    }
}

fn cstr(b: &[u8]) -> Vec<u8> {
    match b.iter().position(|&c| c == 0) {
        Some(p) => b[..p].to_vec(),
        None => b.to_vec(),
    }
}

/// `getopt(string $short_options, array $long_options = [], &$rest_index = null)`.
fn fn_getopt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let short = cstr(&str_param(host, args, 0, "getopt", "short_options")?);
    let longs = match args.get(1) {
        None => PhpArray::new(),
        Some(_) => crate::zend::zpp::array_param(host, args, 1, "getopt")?,
    };
    let set_rest = |v: i64| {
        if let Some(Value::Ref(r)) = args.get(2) {
            *r.borrow_mut() = Value::Int(v);
        }
    };
    set_rest(1);
    let Some(argv) = host.cli_argv() else {
        return Ok(Value::Bool(false));
    };
    let argv: Vec<Vec<u8>> = argv.iter().map(|a| cstr(a)).collect();
    let mut opts = parse_short_opts(&short);
    for (_, v) in longs.iter() {
        let mut name = cstr(&conv::to_string_bytes(&deref(v)));
        let mut need = 0;
        if name.last() == Some(&b':') {
            need = 1;
            name.pop();
            if name.last() == Some(&b':') {
                need = 2;
                name.pop();
            }
        }
        opts.push(OptSpec {
            ch: 0,
            name: Some(name),
            need_param: need,
        });
    }
    let mut out = PhpArray::new();
    let mut optind = 1usize;
    let mut st = GetoptState {
        optchr: 0,
        dash: false,
    };
    loop {
        match php_getopt(&argv, &opts, &mut optind, &mut st) {
            Got::Eof => break,
            Got::Invalid => continue,
            Got::Opt(i, optarg) => {
                let name = match &opts[i].name {
                    Some(n) => n.clone(),
                    None => vec![opts[i].ch],
                };
                let val = optarg.map(Value::str).unwrap_or(Value::Bool(false));
                // A numeric option name ("1") becomes an integer key.
                let key = ArrayKey::from_bytes(&name);
                match out.get_mut(&key) {
                    Some(existing) => {
                        if let Value::Array(a) = existing {
                            a.push(val);
                        } else {
                            let mut a = PhpArray::new();
                            a.push(existing.clone());
                            a.push(val);
                            *existing = Value::Array(a);
                        }
                    }
                    None => out.insert(key, val),
                }
            }
        }
    }
    set_rest(optind as i64);
    Ok(Value::Array(out))
}

// --- readline / stream knobs --------------------------------------------------

/// `readline($prompt)`: one line from stdin without its newline, or false at
/// EOF. With a non-terminal stdin (libedit's non-interactive mode) the
/// prompt is not echoed.
fn fn_readline(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let prompt = match args.first().map(deref) {
        None | Some(Value::Null) => Vec::new(),
        Some(_) => str_param(host, args, 0, "readline", "prompt")?,
    };
    let tty = unsafe { libc::isatty(0) } == 1;
    if tty && !prompt.is_empty() {
        use std::io::Write;
        let mut so = std::io::stdout();
        let _ = so.write_all(&prompt);
        let _ = so.flush();
    }
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    loop {
        let n = unsafe { libc::read(0, b.as_mut_ptr() as *mut libc::c_void, 1) };
        if n <= 0 {
            if line.is_empty() {
                return Ok(Value::Bool(false));
            }
            break;
        }
        if b[0] == b'\n' {
            break;
        }
        line.push(b[0]);
    }
    Ok(Value::str(line))
}

fn fn_readline_add_history(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let _ = str_param(host, args, 0, "readline_add_history", "prompt")?;
    Ok(Value::Bool(true))
}

fn fn_true(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("mt_srand", fn_mt_srand),
            ("srand", fn_mt_srand),
            ("mt_rand", fn_mt_rand),
            ("rand", fn_rand),
            ("fnmatch", fn_fnmatch),
            ("getrusage", fn_getrusage),
            ("sys_getloadavg", fn_sys_getloadavg),
            ("getopt", fn_getopt),
            ("readline", fn_readline),
            ("readline_add_history", fn_readline_add_history),
            ("readline_clear_history", fn_true),
        ],
    );
}

/// The `FNM_*` / `MT_RAND_*` constants.
pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("FNM_NOESCAPE", Value::Int(libc::FNM_NOESCAPE as i64)),
        ("FNM_PATHNAME", Value::Int(libc::FNM_PATHNAME as i64)),
        ("FNM_PERIOD", Value::Int(libc::FNM_PERIOD as i64)),
        ("FNM_CASEFOLD", Value::Int(libc::FNM_CASEFOLD as i64)),
        ("CASE_LOWER", Value::Int(0)),
        ("CASE_UPPER", Value::Int(1)),
        ("MT_RAND_MT19937", Value::Int(MT_RAND_MT19937)),
        ("MT_RAND_PHP", Value::Int(MT_RAND_PHP)),
    ]
}

#[cfg(test)]
mod tests;
