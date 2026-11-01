//! Filesystem builtins with exact php semantics (ext/standard/file.c, dir.c,
//! filestat.c, main/php_glob.c): directory streams (`opendir` family and the
//! `Directory` class), `glob`, `file()`, the `stat` family, descriptor ops on
//! open streams (`flock`/`ftruncate`/`fseek`/`ftell`/`fstat`), `tempnam`, and
//! the stream-path helpers (`stream_is_local`, `stream_resolve_include_path`).
//!
//! Registered after `file_fns`/`stream_fns`, so these entries replace any
//! older approximations there.

use std::collections::HashMap;
use std::ffi::CString;

use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::streams::{split_scheme, StatInfo};
use crate::{Builtin, Host, StreamKind, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn str_arg(args: &[Value], i: usize) -> Vec<u8> {
    args.get(i)
        .map(|v| conv::to_string_bytes(&deref(v)))
        .unwrap_or_default()
}

fn int_arg(args: &[Value], i: usize, default: i64) -> i64 {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => default,
        Some(v) => conv::to_int(&v),
    }
}

/// php's zpp type name for an argument in a TypeError ("string", "int", the
/// class name for objects, …).
fn given(host: &dyn Host, v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        // zend_zval_value_name: booleans show their value.
        Value::Bool(true) => "true".into(),
        Value::Bool(false) => "false".into(),
        Value::Int(_) => "int".into(),
        Value::Float(_) => "float".into(),
        Value::Str(_) => "string".into(),
        Value::Array(_) => "array".into(),
        Value::Closure(_) => "Closure".into(),
        Value::Object(_) => host
            .object_class_name(v)
            .map(|n| String::from_utf8_lossy(&n).into_owned())
            .unwrap_or_else(|| "object".into()),
        Value::Resource(_) => "resource".into(),
        Value::Ref(r) => given(host, &r.borrow()),
    }
}

/// A `Z_PARAM_PATH` argument: a string without NUL bytes.
fn path_param(args: &[Value], i: usize, fname: &str, pname: &str) -> Result<Vec<u8>, Thrown> {
    let p = str_arg(args, i);
    if p.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{fname}(): Argument #{} (${pname}) must not contain any null bytes",
                i + 1
            ),
        ));
    }
    Ok(p)
}

/// A `resource $stream` argument that must be a live stream.
pub(crate) fn stream_param(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<(Value, StreamKind), Thrown> {
    let v = args.get(i).map(deref).unwrap_or(Value::Null);
    if !matches!(v, Value::Resource(_)) {
        return Err(Thrown::type_error(format!(
            "{fname}(): Argument #{} (${pname}) must be of type resource, {} given",
            i + 1,
            given(host, &v)
        )));
    }
    match host.stream_kind(&v) {
        Some(k) => Ok((v, k)),
        None => Err(Thrown::type_error(format!(
            "{fname}(): Argument #{} (${pname}) must be an open stream resource",
            i + 1
        ))),
    }
}

fn arr_from_stat(s: &StatInfo) -> Value {
    let vals = [
        ("dev", s.dev),
        ("ino", s.ino),
        ("mode", s.mode as i64),
        ("nlink", s.nlink),
        ("uid", s.uid),
        ("gid", s.gid),
        ("rdev", s.rdev),
        ("size", s.size as i64),
        ("atime", s.atime),
        ("mtime", s.mtime),
        ("ctime", s.ctime),
        ("blksize", s.blksize),
        ("blocks", s.blocks),
    ];
    let mut a = PhpArray::new();
    for (_, v) in vals.iter() {
        a.push(Value::Int(*v));
    }
    for (k, v) in vals.iter() {
        a.insert(ArrayKey::from_bytes(k.as_bytes()), Value::Int(*v));
    }
    Value::Array(a)
}

/// php's `php_stream_context_from_zval(zcontext, 0)`: with no explicit
/// context argument the default context is fetched (allocating its resource
/// on first use).
pub(crate) fn touch_ctx(host: &mut dyn Host, args: &[Value], idx: usize) {
    if matches!(args.get(idx).map(deref), None | Some(Value::Null)) {
        host.default_stream_context();
    }
}

fn fn_stream_context_get_default(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ctx = host.default_stream_context();
    if let Some(Value::Array(opts)) = args.first().map(deref) {
        for (wk, wv) in opts.iter() {
            if let Value::Array(inner) = wv {
                let w = conv::to_string_bytes(&crate::key_to_value(wk));
                for (ok, ov) in inner.iter() {
                    let o = conv::to_string_bytes(&crate::key_to_value(ok));
                    host.stream_context_set_option(&ctx, &w, &o, ov.clone());
                }
            }
        }
    }
    Ok(ctx)
}

fn fn_fopen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(args, 0, "fopen", "filename")?;
    let mode = str_arg(args, 1);
    if path.is_empty() {
        return Err(Thrown::error("ValueError", "Path must not be empty"));
    }
    touch_ctx(host, args, 3);
    crate::streams::clear_errno();
    match host.stream_open(&path, &mode) {
        Some(v) => Ok(v),
        None => {
            if crate::streams::split_scheme(&path).0 == b"file"
                && crate::streams::parse_fopen_mode(&mode).is_none()
            {
                crate::warn_arg(
                    host,
                    "fopen",
                    &path,
                    &format!(
                        "Failed to open stream: `{}' is not a valid mode for fopen",
                        String::from_utf8_lossy(&mode)
                    ),
                )?;
            } else {
                crate::open_failed(host, "fopen", &path)?;
            }
            Ok(Value::Bool(false))
        }
    }
}

// --- directory streams -------------------------------------------------------

fn do_opendir(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
) -> Result<Option<(Vec<u8>, Value)>, Thrown> {
    let path = path_param(args, 0, fname, "directory")?;
    touch_ctx(host, args, 1);
    crate::streams::clear_errno();
    let Some(entries) = host.stream_opendir(&path) else {
        dir_open_failed(host, fname, &path)?;
        return Ok(None);
    };
    Ok(host.dir_open(&path, entries).map(|h| (path, h)))
}

/// "fname(path): Failed to open directory: <strerror>".
fn dir_open_failed(host: &mut dyn Host, fname: &str, path: &[u8]) -> Result<(), Thrown> {
    let reason = match crate::streams::last_errno() {
        Some(c) => crate::streams::strerror(c),
        None => "operation failed".into(),
    };
    crate::warn_arg(
        host,
        fname,
        path,
        &format!("Failed to open directory: {reason}"),
    )
}

fn fn_opendir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(do_opendir(host, args, "opendir")?
        .map(|(_, h)| h)
        .unwrap_or(Value::Bool(false)))
}

fn fn_dir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some((path, h)) = do_opendir(host, args, "dir")? else {
        return Ok(Value::Bool(false));
    };
    Ok(host
        .instantiate(
            b"Directory",
            vec![
                (b"path".to_vec(), Value::str(path)),
                (b"handle".to_vec(), h),
            ],
        )
        .unwrap_or(Value::Bool(false)))
}

/// Resolve the `?resource $dir_handle` argument of readdir/rewinddir/closedir.
fn dir_param(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Value, Thrown> {
    let v = args.first().map(deref).unwrap_or(Value::Null);
    let v = match v {
        Value::Null => {
            crate::deprecated(
                host,
                fname,
                "Passing null is deprecated, instead the last opened directory stream should be provided",
            )?;
            match host.last_dir() {
                Some(d) => d,
                None => return Err(Thrown::type_error("No resource supplied")),
            }
        }
        Value::Resource(_) => v,
        other => {
            return Err(Thrown::type_error(format!(
                "{fname}(): Argument #1 ($dir_handle) must be of type resource or null, {} given",
                given(host, &other)
            )))
        }
    };
    match host.stream_kind(&v) {
        Some(StreamKind::Dir) => Ok(v),
        Some(_) => Err(Thrown::type_error(format!(
            "{fname}(): Argument #1 ($dir_handle) must be a valid Directory resource"
        ))),
        None => Err(Thrown::type_error(format!(
            "{fname}(): Argument #1 ($dir_handle) must be an open stream resource"
        ))),
    }
}

fn fn_readdir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = dir_param(host, args, "readdir")?;
    Ok(host
        .dir_read(&h)
        .map(Value::str)
        .unwrap_or(Value::Bool(false)))
}

fn fn_rewinddir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = dir_param(host, args, "rewinddir")?;
    host.dir_rewind(&h);
    Ok(Value::Null)
}

fn fn_closedir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = dir_param(host, args, "closedir")?;
    host.stream_close(&h);
    Ok(Value::Null)
}

/// `__directory_op($handle, $op)` — backs `Directory::read/rewind/close`.
fn fn_directory_op(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = args.first().map(deref).unwrap_or(Value::Null);
    let op = str_arg(args, 1);
    let opname = String::from_utf8_lossy(&op).into_owned();
    if !matches!(h, Value::Resource(_)) {
        return Err(Thrown::error(
            "Error",
            "Internal directory stream has been altered",
        ));
    }
    if host.stream_kind(&h) != Some(StreamKind::Dir) {
        return Err(Thrown::type_error(format!(
            "Directory::{opname}(): cannot use Directory resource after it has been closed"
        )));
    }
    match op.as_slice() {
        b"read" => Ok(host
            .dir_read(&h)
            .map(Value::str)
            .unwrap_or(Value::Bool(false))),
        b"rewind" => {
            host.dir_rewind(&h);
            Ok(Value::Null)
        }
        _ => {
            host.stream_close(&h);
            Ok(Value::Null)
        }
    }
}

pub const PRELUDE_PHP: &str = r#"
final class Directory {
    public readonly string $path;
    public readonly mixed $handle;
    public function close(): void {
        if (func_num_args()) throw new ArgumentCountError('Directory::close() expects exactly 0 arguments, ' . func_num_args() . ' given');
        __directory_op($this->handle, "close");
    }
    public function rewind(): void {
        if (func_num_args()) throw new ArgumentCountError('Directory::rewind() expects exactly 0 arguments, ' . func_num_args() . ' given');
        __directory_op($this->handle, "rewind");
    }
    public function read(): string|false {
        if (func_num_args()) throw new ArgumentCountError('Directory::read() expects exactly 0 arguments, ' . func_num_args() . ' given');
        return __directory_op($this->handle, "read");
    }
}
"#;

fn fn_scandir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(args, 0, "scandir", "directory")?;
    if path.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            "scandir(): Argument #1 ($directory) must not be empty",
        ));
    }
    let flags = int_arg(args, 1, 0);
    crate::streams::clear_errno();
    let Some(mut names) = host.stream_opendir(&path) else {
        let errno = crate::streams::last_errno();
        dir_open_failed(host, "scandir", &path)?;
        let (code, text) = match errno {
            Some(c) => (c, crate::streams::strerror(c)),
            None => (0, "operation failed".into()),
        };
        crate::warn(host, "scandir", &format!("(errno {code}): {text}"))?;
        return Ok(Value::Bool(false));
    };
    host.consume_resource_ids(1);
    if flags == 0 {
        names.sort();
    } else if flags != 2 {
        names.sort_by(|a, b| b.cmp(a));
    }
    let mut a = PhpArray::new();
    for n in names {
        a.push(Value::str(n));
    }
    Ok(Value::Array(a))
}

// --- glob (a port of php's bundled OpenBSD glob, main/php_glob.c) ------------

const GLOB_ERR: i64 = 0x0004;
const GLOB_MARK: i64 = 0x0008;
const GLOB_NOCHECK: i64 = 0x0010;
const GLOB_NOSORT: i64 = 0x0020;
const GLOB_BRACE: i64 = 0x0080;
const GLOB_MAGCHAR: i64 = 0x0100;
const GLOB_NOESCAPE: i64 = 0x1000;
const GLOB_ONLYDIR: i64 = 1 << 30;
const GLOB_AVAILABLE: i64 =
    GLOB_BRACE | GLOB_MARK | GLOB_NOSORT | GLOB_NOCHECK | GLOB_NOESCAPE | GLOB_ERR | GLOB_ONLYDIR;
const GLOB_NOMATCH: i32 = -3;

type Ch = u16;
const M_QUOTE: Ch = 0x8000;
const M_PROTECT: Ch = 0x4000;
const M_ASCII: Ch = 0x00ff;
const M_ALL: Ch = b'*' as Ch | M_QUOTE;
const M_END: Ch = b']' as Ch | M_QUOTE;
const M_NOT: Ch = b'!' as Ch | M_QUOTE;
const M_ONE: Ch = b'?' as Ch | M_QUOTE;
const M_RNG: Ch = b'-' as Ch | M_QUOTE;
const M_SET: Ch = b'[' as Ch | M_QUOTE;
const M_CLASS: Ch = b':' as Ch | M_QUOTE;

const CCLASSES: [&str; 12] = [
    "alnum", "alpha", "blank", "cntrl", "digit", "graph", "lower", "print", "punct", "space",
    "upper", "xdigit",
];

fn cclass_match(idx: Ch, k: Ch) -> bool {
    if k > 0x7f {
        return false;
    }
    let c = k as u8;
    match idx {
        0 => c.is_ascii_alphanumeric(),
        1 => c.is_ascii_alphabetic(),
        2 => c == b' ' || c == b'\t',
        3 => c.is_ascii_control(),
        4 => c.is_ascii_digit(),
        5 => c.is_ascii_graphic(),
        6 => c.is_ascii_lowercase(),
        7 => c.is_ascii_graphic() || c == b' ',
        8 => c.is_ascii_punctuation(),
        9 => matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r'),
        10 => c.is_ascii_uppercase(),
        11 => c.is_ascii_hexdigit(),
        _ => false,
    }
}

fn ismeta(c: Ch) -> bool {
    c & M_QUOTE != 0
}

struct Glob {
    flags: i64,
    paths: Vec<Vec<u8>>,
}

impl Glob {
    fn run(&mut self, pattern: &[u8]) -> i32 {
        let mut buf: Vec<Ch> = Vec::with_capacity(pattern.len());
        if self.flags & GLOB_NOESCAPE != 0 {
            buf.extend(pattern.iter().map(|&b| b as Ch));
        } else {
            let mut i = 0;
            while i < pattern.len() {
                let c = pattern[i];
                i += 1;
                if c == b'\\' {
                    if i < pattern.len() {
                        buf.push(pattern[i] as Ch | M_PROTECT);
                        i += 1;
                    } else {
                        buf.push(b'\\' as Ch | M_PROTECT);
                    }
                } else {
                    buf.push(c as Ch);
                }
            }
        }
        if self.flags & GLOB_BRACE != 0 {
            self.globexp1(&buf)
        } else {
            self.glob0(&buf)
        }
    }

    fn globexp1(&mut self, pattern: &[Ch]) -> i32 {
        const LB: Ch = b'{' as Ch;
        const RB: Ch = b'}' as Ch;
        if pattern == [LB, RB] {
            return self.glob0(pattern);
        }
        if let Some(i) = pattern.iter().position(|&c| c == LB) {
            return self.globexp2(i, pattern);
        }
        self.glob0(pattern)
    }

    fn globexp2(&mut self, brace: usize, pattern: &[Ch]) -> i32 {
        const LB: Ch = b'{' as Ch;
        const RB: Ch = b'}' as Ch;
        const LBK: Ch = b'[' as Ch;
        const RBK: Ch = b']' as Ch;
        const COMMA: Ch = b',' as Ch;
        let at = |i: usize| -> Ch { pattern.get(i).copied().unwrap_or(0) };
        let prefix = &pattern[..brace];
        // find the balanced closing brace
        let mut depth = 0i32;
        let mut pe = brace + 1;
        while at(pe) != 0 {
            let c = at(pe);
            if c == LBK {
                let pm = pe;
                pe += 1;
                while at(pe) != RBK && at(pe) != 0 {
                    pe += 1;
                }
                if at(pe) == 0 {
                    pe = pm;
                }
            } else if c == LB {
                depth += 1;
            } else if c == RB {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            pe += 1;
        }
        if depth != 0 || at(pe) == 0 {
            return self.glob0(prefix);
        }
        let mut depth = 0i32;
        let mut pl = brace + 1;
        let mut pm = brace + 1;
        while pm <= pe {
            let c = at(pm);
            if c == LBK {
                let pb = pm;
                pm += 1;
                while at(pm) != RBK && at(pm) != 0 {
                    pm += 1;
                }
                if at(pm) == 0 {
                    pm = pb;
                }
            } else if c == LB {
                depth += 1;
            } else if (c == RB && depth == 0) || (c == COMMA && depth == 0) {
                let mut np: Vec<Ch> = prefix.to_vec();
                np.extend_from_slice(&pattern[pl..pm]);
                np.extend_from_slice(&pattern[pe + 1..]);
                let rv = self.globexp1(&np);
                if rv != 0 && rv != GLOB_NOMATCH {
                    return rv;
                }
                pl = pm + 1;
            } else if c == RB {
                depth -= 1;
            }
            pm += 1;
        }
        0
    }

    fn glob0(&mut self, pattern: &[Ch]) -> i32 {
        let at = |i: usize| -> Ch { pattern.get(i).copied().unwrap_or(0) };
        let oldpathc = self.paths.len();
        let mut out: Vec<Ch> = Vec::with_capacity(pattern.len());
        let mut i = 0usize;
        let mut magchar = false;
        while i < pattern.len() && pattern[i] != 0 {
            let c = pattern[i];
            i += 1;
            match c {
                x if x == b'[' as Ch => {
                    // (php 8.5.8's bundled glob does not treat a leading '^'
                    // as negation; only '!'.)
                    let c2 = at(i);
                    if c2 == b'!' as Ch {
                        i += 1;
                    }
                    let has_close =
                        at(i) != 0 && pattern[(i + 1).min(pattern.len())..].contains(&(b']' as Ch));
                    if !has_close {
                        out.push(b'[' as Ch);
                        if c2 == b'!' as Ch {
                            i -= 1;
                        }
                        continue;
                    }
                    out.push(M_SET);
                    if c2 == b'!' as Ch {
                        out.push(M_NOT);
                    }
                    let mut c = at(i);
                    i += 1;
                    loop {
                        if c == b'[' as Ch && at(i) == b':' as Ch {
                            let mut err;
                            loop {
                                err = self.charclass(pattern, &mut i, &mut out);
                                if err != 0 {
                                    break;
                                }
                                c = at(i);
                                i += 1;
                                if !(c == b'[' as Ch && at(i) == b':' as Ch) {
                                    break;
                                }
                            }
                            if err == -1 && self.flags & GLOB_NOCHECK == 0 {
                                return GLOB_NOMATCH;
                            }
                            if c == b']' as Ch {
                                break;
                            }
                        }
                        out.push(c & M_ASCII);
                        if at(i) == b'-' as Ch {
                            let c3 = at(i + 1);
                            if c3 != b']' as Ch {
                                out.push(M_RNG);
                                out.push(c3 & M_ASCII);
                                i += 2;
                            }
                        }
                        c = at(i);
                        i += 1;
                        if c == b']' as Ch {
                            break;
                        }
                    }
                    magchar = true;
                    out.push(M_END);
                }
                x if x == b'?' as Ch => {
                    magchar = true;
                    out.push(M_ONE);
                }
                x if x == b'*' as Ch => {
                    magchar = true;
                    if out.last() != Some(&M_ALL) {
                        out.push(M_ALL);
                    }
                }
                _ => out.push(c & M_ASCII),
            }
        }
        if magchar {
            self.flags |= GLOB_MAGCHAR;
        }
        if !out.is_empty() {
            let mut pathbuf: Vec<Ch> = Vec::new();
            let err = self.glob2(&mut pathbuf, &out);
            if err != 0 {
                return err;
            }
        }
        if self.paths.len() == oldpathc {
            if self.flags & GLOB_NOCHECK != 0 {
                self.paths.push(pattern.iter().map(|&c| c as u8).collect());
                return 0;
            }
            return GLOB_NOMATCH;
        }
        if self.flags & GLOB_NOSORT == 0 {
            self.paths[oldpathc..].sort();
        }
        0
    }

    /// `[:class:]` inside a bracket expression; 0 ok, 1 not a class, -1 invalid.
    fn charclass(&self, pattern: &[Ch], i: &mut usize, out: &mut Vec<Ch>) -> i32 {
        // *i points at ':' (the char after '[')
        let start = *i + 1;
        let rest = &pattern[start.min(pattern.len())..];
        let Some(colon) = rest.iter().position(|&c| c == b':' as Ch) else {
            return 1;
        };
        if rest.get(colon + 1).copied() != Some(b']' as Ch) {
            return 1;
        }
        let name: Vec<u8> = rest[..colon].iter().map(|&c| c as u8).collect();
        let Some(idx) = CCLASSES
            .iter()
            .position(|n| n.as_bytes() == name.as_slice())
        else {
            return -1;
        };
        out.push(M_CLASS);
        out.push(idx as Ch);
        *i = start + colon + 2;
        0
    }

    fn to_bytes(p: &[Ch]) -> Vec<u8> {
        p.iter().map(|&c| c as u8).collect()
    }

    fn glob2(&mut self, pathbuf: &mut Vec<Ch>, mut pattern: &[Ch]) -> i32 {
        loop {
            if pattern.is_empty() {
                let path = Self::to_bytes(pathbuf);
                let Some(sb) = crate::streams::sys_stat(&path, true) else {
                    return 0;
                };
                let mut path = path;
                if self.flags & GLOB_MARK != 0
                    && path.last() != Some(&b'/')
                    && (sb.is_dir
                        || (sb.is_link()
                            && crate::streams::sys_stat(&path, false).is_some_and(|s| s.is_dir)))
                {
                    path.push(b'/');
                }
                self.paths.push(path);
                return 0;
            }
            // next segment
            let mut anymeta = false;
            let mut p = 0;
            while p < pattern.len() && pattern[p] != b'/' as Ch {
                if ismeta(pattern[p]) {
                    anymeta = true;
                }
                p += 1;
            }
            if !anymeta {
                pathbuf.extend_from_slice(&pattern[..p]);
                pattern = &pattern[p..];
                while let Some(&c) = pattern.first() {
                    if c != b'/' as Ch {
                        break;
                    }
                    pathbuf.push(c);
                    pattern = &pattern[1..];
                }
            } else {
                return self.glob3(pathbuf, &pattern[..p], &pattern[p..]);
            }
        }
    }

    fn glob3(&mut self, pathbuf: &mut Vec<Ch>, pattern: &[Ch], rest: &[Ch]) -> i32 {
        let dir = if pathbuf.is_empty() {
            b".".to_vec()
        } else {
            Self::to_bytes(pathbuf)
        };
        // php passes no errfunc, so an unreadable directory is skipped even
        // with GLOB_ERR (the bundled glob only honours GLOB_ERR via errfunc).
        let Some(entries) = crate::streams::sys_readdir(&dir) else {
            return 0;
        };
        let base = pathbuf.len();
        for name in entries {
            if name.first() == Some(&b'.') && pattern.first() != Some(&(b'.' as Ch)) {
                continue;
            }
            let nm: Vec<Ch> = name.iter().map(|&b| b as Ch).collect();
            if !glob_match(&nm, pattern) {
                continue;
            }
            pathbuf.truncate(base);
            pathbuf.extend_from_slice(&nm);
            let err = self.glob2(pathbuf, rest);
            pathbuf.truncate(base);
            if err != 0 {
                return err;
            }
        }
        0
    }
}

/// php_glob's non-recursive `match()` (Russ Cox's algorithm).
fn glob_match(name: &[Ch], pat: &[Ch]) -> bool {
    let (mut n, mut p) = (0usize, 0usize);
    let mut next: Option<(usize, usize)> = None; // (pat, name)
    'outer: loop {
        let mut failed = false;
        while p < pat.len() {
            let c = pat[p];
            p += 1;
            match c {
                M_ALL => {
                    while p < pat.len() && pat[p] == M_ALL {
                        p += 1;
                    }
                    if p == pat.len() {
                        return true;
                    }
                    if n >= name.len() {
                        return false;
                    }
                    next = Some((p - 1, n + 1));
                }
                M_ONE => {
                    if n >= name.len() {
                        failed = true;
                        break;
                    }
                    n += 1;
                }
                M_SET => {
                    let mut ok = false;
                    if n >= name.len() {
                        failed = true;
                        break;
                    }
                    let k = name[n];
                    n += 1;
                    let negate = pat.get(p) == Some(&M_NOT);
                    if negate {
                        p += 1;
                    }
                    while let Some(&c) = pat.get(p) {
                        p += 1;
                        if c == M_END {
                            break;
                        }
                        if c == M_CLASS {
                            let idx = pat.get(p).copied().unwrap_or(0);
                            if (idx as usize) < CCLASSES.len() && cclass_match(idx, k) {
                                ok = true;
                            }
                            p += 1;
                        }
                        if pat.get(p) == Some(&M_RNG) {
                            let hi = pat.get(p + 1).copied().unwrap_or(0);
                            if c <= k && k <= hi {
                                ok = true;
                            }
                            p += 2;
                        } else if c == k {
                            ok = true;
                        }
                    }
                    if ok == negate {
                        failed = true;
                        break;
                    }
                }
                _ => {
                    if n >= name.len() || name[n] != c {
                        failed = true;
                        break;
                    }
                    n += 1;
                }
            }
        }
        if !failed && n >= name.len() {
            return true;
        }
        match next {
            Some((np, nn)) if nn <= name.len() => {
                p = np;
                n = nn;
                next = None;
                // re-arm: the M_ALL at `np` will set next again
                continue 'outer;
            }
            _ => return false,
        }
    }
}

fn fn_glob(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let _ = host;
    let pattern = path_param(args, 0, "glob", "pattern")?;
    let flags = int_arg(args, 1, 0);
    if pattern.len() >= 1024 {
        return Ok(Value::Bool(false));
    }
    if (GLOB_AVAILABLE & flags) != flags {
        return Ok(Value::Bool(false));
    }
    let mut g = Glob {
        flags: flags & !GLOB_ONLYDIR,
        paths: Vec::new(),
    };
    let ret = g.run(&pattern);
    if ret != 0 && ret != GLOB_NOMATCH {
        return Ok(Value::Bool(false));
    }
    let mut a = PhpArray::new();
    for p in g.paths {
        if flags & GLOB_ONLYDIR != 0 {
            match crate::streams::sys_stat(&p, false) {
                Some(s) if s.is_dir => {}
                _ => continue,
            }
        }
        a.push(Value::str(p));
    }
    Ok(Value::Array(a))
}

// --- file() --------------------------------------------------------------------

fn fn_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(args, 0, "file", "filename")?;
    let flags = int_arg(args, 1, 0);
    const USE_INCLUDE_PATH: i64 = 1;
    const IGNORE_NEW_LINES: i64 = 2;
    const SKIP_EMPTY_LINES: i64 = 4;
    const NO_DEFAULT_CONTEXT: i64 = 16;
    if flags & !(USE_INCLUDE_PATH | IGNORE_NEW_LINES | SKIP_EMPTY_LINES | NO_DEFAULT_CONTEXT) != 0 {
        return Err(Thrown::error(
            "ValueError",
            "file(): Argument #2 ($flags) must be a valid flag value",
        ));
    }
    if flags & NO_DEFAULT_CONTEXT == 0 {
        touch_ctx(host, args, 2);
    }
    let include_new_line = flags & IGNORE_NEW_LINES == 0;
    let skip_blank = flags & SKIP_EMPTY_LINES != 0;
    crate::streams::clear_errno();
    let data = if flags & USE_INCLUDE_PATH != 0 {
        match resolve_include_path(&path) {
            Some(p) => host.stream_get_contents(&p),
            None => host.stream_get_contents(&path),
        }
    } else {
        host.stream_get_contents(&path)
    };
    let Some(buf) = data else {
        if crate::streams::last_errno() == Some(libc::EISDIR) {
            host.consume_resource_ids(1);
            crate::notice(
                host,
                "file",
                "Read of 8192 bytes failed with errno=21 Is a directory",
            )?;
            return Ok(Value::Array(PhpArray::new()));
        }
        crate::open_failed(host, "file", &path)?;
        return Ok(Value::Bool(false));
    };
    host.consume_resource_ids(crate::streams::open_resource_ids(&path));
    let mut a = PhpArray::new();
    if buf.is_empty() {
        return Ok(Value::Array(a));
    }
    let e = buf.len();
    let mut s = 0usize;
    let find = |from: usize| {
        buf[from..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|i| from + i)
    };
    match find(0) {
        None => {
            a.push(Value::str(buf.clone()));
            return Ok(Value::Array(a));
        }
        Some(first) => {
            let mut p = first;
            if include_new_line {
                loop {
                    p += 1;
                    a.push(Value::str(buf[s..p].to_vec()));
                    s = p;
                    match find(p) {
                        Some(n) => p = n,
                        None => break,
                    }
                }
            } else {
                loop {
                    let windows_eol = usize::from(p != 0 && buf[p - 1] == b'\r');
                    if skip_blank && p - s - windows_eol == 0 {
                        p += 1;
                        s = p;
                    } else {
                        a.push(Value::str(buf[s..p - windows_eol].to_vec()));
                        p += 1;
                        s = p;
                    }
                    match find(p) {
                        Some(n) => p = n,
                        None => break,
                    }
                }
            }
        }
    }
    if s != e {
        a.push(Value::str(buf[s..e].to_vec()));
    }
    Ok(Value::Array(a))
}

// --- stat family -----------------------------------------------------------------

fn stat_path(host: &dyn Host, args: &[Value], link: bool) -> Option<StatInfo> {
    let p = str_arg(args, 0);
    if p.is_empty() || p.contains(&0) {
        return None;
    }
    if link {
        host.stream_lstat(&p)
    } else {
        host.stream_stat(&p)
    }
}

/// php_stat's failure warning (not for the existence checks).
fn stat_failed(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    link: bool,
) -> Result<Value, Thrown> {
    let p = str_arg(args, 0);
    if !p.is_empty() && !p.contains(&0) {
        crate::warn(
            host,
            fname,
            &format!(
                "{}stat failed for {}",
                if link { "L" } else { "" },
                String::from_utf8_lossy(&p)
            ),
        )?;
    }
    Ok(Value::Bool(false))
}

fn fn_stat(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match stat_path(host, args, false) {
        Some(s) => Ok(arr_from_stat(&s)),
        None => stat_failed(host, args, "stat", false),
    }
}

fn fn_lstat(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match stat_path(host, args, true) {
        Some(s) => Ok(arr_from_stat(&s)),
        None => stat_failed(host, args, "lstat", true),
    }
}

macro_rules! stat_field_fn {
    ($name:ident, $fname:literal, $link:expr, $warn:expr, |$s:ident| $e:expr) => {
        fn $name(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            match stat_path(host, args, $link) {
                Some($s) => Ok($e),
                None if $warn => stat_failed(host, args, $fname, $link),
                None => Ok(Value::Bool(false)),
            }
        }
    };
}

stat_field_fn!(fn_fileinode, "fileinode", false, true, |s| Value::Int(
    s.ino
));
stat_field_fn!(fn_fileowner, "fileowner", false, true, |s| Value::Int(
    s.uid
));
stat_field_fn!(fn_filegroup, "filegroup", false, true, |s| Value::Int(
    s.gid
));
stat_field_fn!(fn_fileatime, "fileatime", false, true, |s| Value::Int(
    s.atime
));
stat_field_fn!(fn_filemtime, "filemtime", false, true, |s| Value::Int(
    s.mtime
));
stat_field_fn!(fn_filectime, "filectime", false, true, |s| Value::Int(
    s.ctime
));
stat_field_fn!(fn_filesize, "filesize", false, true, |s| Value::Int(
    s.size as i64
));
stat_field_fn!(fn_fileperms, "fileperms", false, true, |s| Value::Int(
    s.mode as i64
));
stat_field_fn!(fn_is_link, "is_link", true, false, |s| Value::Bool(
    s.is_link()
));

fn exists_check(host: &dyn Host, args: &[Value], link: bool) -> Option<StatInfo> {
    stat_path(host, args, link)
}

fn fn_is_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(
        exists_check(host, args, false).is_some_and(|s| s.is_regular),
    ))
}

fn fn_is_dir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(
        exists_check(host, args, false).is_some_and(|s| s.is_dir),
    ))
}

#[allow(clippy::unnecessary_cast)]
fn fn_filetype(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(s) = stat_path(host, args, true) else {
        return stat_failed(host, args, "filetype", true);
    };
    let t = s.mode & libc::S_IFMT as u32;
    let name: &str = if t == libc::S_IFLNK as u32 {
        "link"
    } else if t == libc::S_IFIFO as u32 {
        "fifo"
    } else if t == libc::S_IFCHR as u32 {
        "char"
    } else if t == libc::S_IFDIR as u32 {
        "dir"
    } else if t == libc::S_IFBLK as u32 {
        "block"
    } else if t == libc::S_IFREG as u32 {
        "file"
    } else if t == libc::S_IFSOCK as u32 {
        "socket"
    } else {
        "unknown"
    };
    Ok(Value::str(name))
}

/// is_readable / is_writable / is_executable: `access(2)` for plain files;
/// for other wrappers php checks the stat mode bits against the uid/gid.
fn access_check(host: &dyn Host, args: &[Value], amode: libc::c_int) -> bool {
    let p = str_arg(args, 0);
    if p.is_empty() || p.contains(&0) {
        return false;
    }
    let (scheme, _) = split_scheme(&p);
    if scheme == b"file" {
        let local = crate::streams::file_path(&p).to_vec();
        return match CString::new(local) {
            Ok(c) => unsafe { libc::access(c.as_ptr(), amode) == 0 },
            Err(_) => false,
        };
    }
    let Some(s) = host.stream_stat(&p) else {
        return false;
    };
    // php_stat (filestat.c): owner bits if we own it, group bits for our
    // primary or any supplementary group, else "other" bits. The root
    // shortcut only applies to the plain-files wrapper (handled above).
    let (uid, gid) = unsafe { (libc::getuid() as i64, libc::getgid() as i64) };
    let in_group = |g: i64| {
        if g == gid {
            return true;
        }
        let n = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
        if n <= 0 {
            return false;
        }
        let mut gids: Vec<libc::gid_t> = vec![0; n as usize];
        let n = unsafe { libc::getgroups(n, gids.as_mut_ptr()) };
        gids.iter().take(n.max(0) as usize).any(|&x| x as i64 == g)
    };
    let shift = if s.uid == uid {
        6
    } else if in_group(s.gid) {
        3
    } else {
        0
    };
    let bit = match amode {
        libc::R_OK => 4,
        libc::W_OK => 2,
        _ => 1,
    };
    s.mode & (bit << shift) != 0
}

fn fn_is_readable(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(access_check(host, args, libc::R_OK)))
}
fn fn_is_writable(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(access_check(host, args, libc::W_OK)))
}
fn fn_is_executable(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(access_check(host, args, libc::X_OK)))
}

// --- descriptor ops on open streams ---------------------------------------------

fn fn_fstat(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fstat", "stream")?;
    Ok(host
        .stream_fstat(&h)
        .map(|s| arr_from_stat(&s))
        .unwrap_or(Value::Bool(false)))
}

fn fn_flock(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "flock", "stream")?;
    let op = int_arg(args, 1, 0);
    let act = op & 3;
    if !(1..=3).contains(&act) {
        return Err(Thrown::error(
            "ValueError",
            "flock(): Argument #2 ($operation) must be one of LOCK_SH, LOCK_EX, or LOCK_UN",
        ));
    }
    let wb = args.get(2).and_then(|v| match v {
        Value::Ref(r) => Some(r.clone()),
        _ => None,
    });
    if let Some(r) = &wb {
        *r.borrow_mut() = Value::Int(0);
    }
    // Socket streams do not implement php's LOCKING option.
    if host.stream_sock_info(&h).is_some() {
        return Ok(Value::Bool(false));
    }
    let Some(fd) = host.stream_fd(&h) else {
        return Ok(Value::Bool(false));
    };
    let mut lop = match act {
        1 => libc::LOCK_SH,
        2 => libc::LOCK_EX,
        _ => libc::LOCK_UN,
    };
    if op & 4 != 0 {
        lop |= libc::LOCK_NB;
    }
    let r = unsafe { libc::flock(fd, lop) };
    if r != 0 {
        let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if errno == libc::EWOULDBLOCK {
            if let Some(r) = &wb {
                *r.borrow_mut() = Value::Int(1);
            }
        }
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

fn fn_ftruncate(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "ftruncate", "stream")?;
    let size = int_arg(args, 1, 0);
    if size < 0 {
        return Err(Thrown::error(
            "ValueError",
            "ftruncate(): Argument #2 ($size) must be greater than or equal to 0",
        ));
    }
    // Socket streams have no truncate support (php's TRUNCATE_SUPPORTED probe).
    if host.stream_sock_info(&h).is_some() {
        crate::warn(host, "ftruncate", "Can't truncate this stream!")?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(host.stream_truncate(&h, size).unwrap_or(false)))
}

fn fn_fseek(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fseek", "stream")?;
    let off = int_arg(args, 1, 0);
    let whence = int_arg(args, 2, 0) as i32;
    if host.stream_seek(&h, off, whence) {
        return Ok(Value::Int(0));
    }
    // php emulates a forward relative seek on an unseekable stream by
    // reading (failing quietly at eof); anything else warns.
    let forward = whence == libc::SEEK_CUR && off >= 0;
    if !forward && host.stream_meta(&h).is_some_and(|m| !m.seekable) {
        crate::warn(host, "fseek", "Stream does not support seeking")?;
    }
    Ok(Value::Int(-1))
}

fn fn_ftell(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "ftell", "stream")?;
    Ok(host
        .stream_tell(&h)
        .map(Value::Int)
        .unwrap_or(Value::Bool(false)))
}

fn fn_rewind(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "rewind", "stream")?;
    if host.stream_rewind(&h) {
        return Ok(Value::Bool(true));
    }
    // php's seek of an unseekable stream (pipe, socket) warns.
    if host.stream_meta(&h).is_some_and(|m| !m.seekable) {
        crate::warn(host, "rewind", "Stream does not support seeking")?;
    }
    Ok(Value::Bool(false))
}

fn fn_fgets(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fgets", "stream")?;
    let line = match args.get(1).map(deref) {
        None | Some(Value::Null) => host.stream_gets(&h),
        Some(v) => {
            let len = conv::to_int(&v);
            if len <= 0 {
                return Err(Thrown::error(
                    "ValueError",
                    "fgets(): Argument #2 ($length) must be greater than 0",
                ));
            }
            if len == 1 {
                // php: a zero-byte read returns false? no — it returns "" only
                // when not at eof; match php_stream_get_line's maxlen=0 path.
                return Ok(Value::Bool(false));
            }
            host.stream_gets_max(&h, (len - 1) as usize)
        }
    };
    Ok(line.map(Value::str).unwrap_or(Value::Bool(false)))
}

fn fn_fread(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fread", "stream")?;
    let len = int_arg(args, 1, 0);
    if len <= 0 {
        return Err(Thrown::error(
            "ValueError",
            "fread(): Argument #2 ($length) must be greater than 0",
        ));
    }
    Ok(host
        .stream_read(&h, len)
        .map(Value::str)
        .unwrap_or(Value::Bool(false)))
}

fn fn_fwrite(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fwrite", "stream")?;
    let mut data = str_arg(args, 1);
    match args.get(2).map(deref) {
        None | Some(Value::Null) => {}
        Some(v) => {
            let n = conv::to_int(&v);
            if n <= 0 {
                return Ok(Value::Int(0));
            }
            if (n as usize) < data.len() {
                data.truncate(n as usize);
            }
        }
    }
    if data.is_empty() {
        return Ok(Value::Int(0));
    }
    Ok(host
        .stream_write(&h, &data)
        .map(|n| Value::Int(n as i64))
        .unwrap_or(Value::Bool(false)))
}

fn fn_fclose(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, kind) = stream_param(host, args, 0, "fclose", "stream")?;
    if kind == StreamKind::Dir {
        // php: "cannot close the provided stream, as it must not be manually
        // closed" (directory handles are closed with closedir()).
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(host.stream_close(&h)))
}

fn fn_feof(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "feof", "stream")?;
    Ok(Value::Bool(host.stream_eof(&h)))
}

fn fn_fflush(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fflush", "stream")?;
    Ok(Value::Bool(host.stream_flush(&h)))
}

fn fn_fgetc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fgetc", "stream")?;
    match host.stream_read(&h, 1) {
        Some(b) if !b.is_empty() => Ok(Value::str(b)),
        _ => Ok(Value::Bool(false)),
    }
}

fn fn_stream_get_contents(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "stream_get_contents", "stream")?;
    let maxlen = int_arg(args, 1, -1);
    let offset = int_arg(args, 2, -1);
    if maxlen < -1 {
        return Err(Thrown::error(
            "ValueError",
            "stream_get_contents(): Argument #2 ($length) must be greater than or equal to -1",
        ));
    }
    if offset >= 0 && !host.stream_seek(&h, offset, 0) {
        // php: "Failed to seek to position N in the stream".
        return Ok(Value::Bool(false));
    }
    if maxlen == 0 {
        return Ok(Value::str(""));
    }
    Ok(host
        .stream_read(&h, maxlen)
        .map(Value::str)
        .unwrap_or(Value::Bool(false)))
}

pub(crate) fn fn_stream_get_meta_data(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "stream_get_meta_data", "stream")?;
    let Some(m) = host.stream_meta(&h) else {
        return Ok(Value::Bool(false));
    };
    let mut a = PhpArray::new();
    let mut put = |k: &str, v: Value| {
        a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
    };
    // A TEMP stream answers META_DATA_API itself, so php skips these three.
    if m.stream_type != "TEMP" {
        put("timed_out", Value::Bool(m.timed_out));
        put("blocked", Value::Bool(m.blocked));
        put("eof", Value::Bool(m.eof));
    }
    // Pipes have no wrapper and no uri; directory streams no uri.
    if !m.wrapper_type.is_empty() {
        put("wrapper_type", Value::str(m.wrapper_type));
    }
    put("stream_type", Value::str(m.stream_type));
    put("mode", Value::str(m.mode));
    put("unread_bytes", Value::Int(m.unread_bytes));
    put("seekable", Value::Bool(m.seekable));
    if !m.uri.is_empty() {
        put("uri", Value::str(m.uri));
    }
    Ok(Value::Array(a))
}

// --- hard links -----------------------------------------------------------------------

/// `link($target, $link)`: a hard link; plain paths only.
fn fn_link(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let target = path_param(args, 0, "link", "target")?;
    let link = path_param(args, 1, "link", "link")?;
    if target.is_empty() || link.is_empty() {
        return Ok(Value::Bool(false));
    }
    let is_url = |p: &[u8]| split_scheme(p).0 != b"file" || p.starts_with(b"file://");
    if is_url(&target) || is_url(&link) {
        return Ok(Value::Bool(false));
    }
    let (Ok(t), Ok(l)) = (CString::new(target), CString::new(link)) else {
        return Ok(Value::Bool(false));
    };
    if unsafe { libc::link(t.as_ptr(), l.as_ptr()) } != 0 {
        let e = std::io::Error::last_os_error();
        let text = crate::streams::strerror(e.raw_os_error().unwrap_or(0));
        crate::warn(host, "link", &text)?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

/// `linkinfo($path)`: `st_dev` of the link itself, or -1.
fn fn_linkinfo(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(args, 0, "linkinfo", "path")?;
    Ok(Value::Int(
        crate::streams::sys_stat(&path, true)
            .map(|s| s.dev)
            .unwrap_or(-1),
    ))
}

// --- mkdir / copy ------------------------------------------------------------------

fn sys_mkdir(path: &[u8], mode: i64) -> Result<(), i32> {
    let c = CString::new(path.to_vec()).map_err(|_| libc::EINVAL)?;
    if unsafe { libc::mkdir(c.as_ptr(), mode as libc::mode_t) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
    }
}

/// `mkdir($directory, $permissions = 0777, $recursive = false)` — php's
/// plain-files mkdir: the mode is honoured (minus umask); a recursive mkdir
/// creates each missing component and fails if the leaf already exists.
fn fn_mkdir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(args, 0, "mkdir", "directory")?;
    let mode = int_arg(args, 1, 0o777);
    let recursive = args
        .get(2)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    touch_ctx(host, args, 3);
    let (scheme, _) = split_scheme(&path);
    if scheme != b"file" {
        return Ok(Value::Bool(false));
    }
    let local = crate::streams::file_path(&path).to_vec();
    let fail = |host: &mut dyn Host, e: i32| -> Result<Value, Thrown> {
        crate::warn(host, "mkdir", &crate::streams::strerror(e))?;
        Ok(Value::Bool(false))
    };
    if !recursive {
        return match sys_mkdir(&local, mode) {
            Ok(()) => Ok(Value::Bool(true)),
            Err(e) => fail(host, e),
        };
    }
    // Every cumulative prefix ending before a '/', then the whole path.
    let mut prefixes: Vec<Vec<u8>> = Vec::new();
    for (i, &b) in local.iter().enumerate() {
        if b == b'/' && i > 0 && local[i - 1] != b'/' {
            prefixes.push(local[..i].to_vec());
        }
    }
    let trimmed_end = local
        .iter()
        .rposition(|&b| b != b'/')
        .map(|i| i + 1)
        .unwrap_or(local.len());
    let leaf = local[..trimmed_end].to_vec();
    prefixes.retain(|p| p.len() < leaf.len());
    for p in &prefixes {
        if crate::streams::sys_stat(p, false).is_some() {
            continue;
        }
        match sys_mkdir(p, mode) {
            Ok(()) => {}
            Err(e) if e == libc::EEXIST => {}
            Err(e) => return fail(host, e),
        }
    }
    match sys_mkdir(&leaf, mode) {
        Ok(()) => Ok(Value::Bool(true)),
        Err(e) => fail(host, e),
    }
}

/// `copy($from, $to)` — php_copy_file_ctx: refuses directories and copying a
/// file onto itself; reads through the stream layer (so `phar://` sources
/// work) and writes like `fopen($to, "wb")` (an existing target keeps its
/// mode, a new one gets 0666 & ~umask).
fn fn_copy(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let from = path_param(args, 0, "copy", "from")?;
    let to = path_param(args, 1, "copy", "to")?;
    touch_ctx(host, args, 2);
    if let Some(s) = host.stream_stat(&from) {
        if s.is_dir {
            crate::warn(
                host,
                "copy",
                "The first argument to copy() function cannot be a directory",
            )?;
            return Ok(Value::Bool(false));
        }
        if let Some(d) = host.stream_stat(&to) {
            if d.is_dir {
                crate::warn(
                    host,
                    "copy",
                    "The second argument to copy() function cannot be a directory",
                )?;
                return Ok(Value::Bool(false));
            }
            if s.ino != 0 && d.ino != 0 {
                if s.ino == d.ino && s.dev == d.dev {
                    return Ok(Value::Bool(false));
                }
            } else if realpath_bytes(&from).is_some()
                && realpath_bytes(&from) == realpath_bytes(&to)
            {
                return Ok(Value::Bool(false));
            }
        }
    }
    crate::streams::clear_errno();
    let Some(data) = host.stream_get_contents(&from) else {
        crate::open_failed(host, "copy", &from)?;
        return Ok(Value::Bool(false));
    };
    host.consume_resource_ids(1);
    let (scheme, _) = split_scheme(&to);
    crate::streams::clear_errno();
    if scheme == b"file" {
        let Some(mut f) = crate::streams::LocalFile::open(&to, b"wb") else {
            crate::open_failed(host, "copy", &to)?;
            return Ok(Value::Bool(false));
        };
        host.consume_resource_ids(1);
        return Ok(Value::Bool(f.write(&data).is_some()));
    }
    if host.stream_put_contents(&to, &data).is_none() {
        crate::open_failed(host, "copy", &to)?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

// --- temp files / paths ------------------------------------------------------------

/// php_get_temporary_directory(): $TMPDIR (trailing slash dropped), else
/// P_tmpdir.
pub fn temp_dir() -> Vec<u8> {
    if let Some(s) = std::env::var_os("TMPDIR") {
        use std::os::unix::ffi::OsStrExt;
        let b = s.as_bytes();
        if !b.is_empty() {
            let mut v = b.to_vec();
            if v.last() == Some(&b'/') {
                v.pop();
            }
            return v;
        }
    }
    if cfg!(target_os = "macos") {
        b"/var/tmp/".to_vec()
    } else {
        b"/tmp".to_vec()
    }
}

fn fn_sys_get_temp_dir(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(temp_dir()))
}

/// php's tsrm_realpath: canonical absolute path of an existing file; "" is
/// the cwd.
fn realpath_bytes(p: &[u8]) -> Option<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    let path = if p.is_empty() { b"." as &[u8] } else { p };
    let os = std::ffi::OsStr::from_bytes(path);
    std::fs::canonicalize(os)
        .ok()
        .map(|pb| pb.as_os_str().as_bytes().to_vec())
}

fn fn_realpath(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let p = path_param(args, 0, "realpath", "path")?;
    Ok(realpath_bytes(&p)
        .map(Value::str)
        .unwrap_or(Value::Bool(false)))
}

fn open_temp(dir: &[u8], pfx: &[u8]) -> Option<Vec<u8>> {
    if dir.is_empty() {
        return None;
    }
    let real = realpath_bytes(dir)?;
    let mut rnd = [0u8; 8];
    getrandom_fill(&mut rnd);
    let mut random = u64::from_le_bytes(rnd);
    const ALPHA: &[u8] = b"0123456789abcdefghijklmnopqrstuv";
    let mut name = real.clone();
    if name.last() != Some(&b'/') {
        name.push(b'/');
    }
    name.extend_from_slice(pfx);
    for _ in 0..13 {
        name.push(ALPHA[(random % 32) as usize]);
        random /= 32;
    }
    name.extend_from_slice(b"XXXXXX");
    let c = CString::new(name).ok()?;
    let raw = c.into_raw();
    let fd = unsafe { libc::mkstemp(raw) };
    let c = unsafe { CString::from_raw(raw) };
    if fd < 0 {
        return None;
    }
    unsafe { libc::close(fd) };
    Some(c.into_bytes())
}

fn getrandom_fill(buf: &mut [u8]) {
    use std::io::Read;
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(buf).is_ok() {
            return;
        }
    }
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
        ^ (std::process::id() as u64) << 32;
    buf.copy_from_slice(&t.to_le_bytes()[..buf.len().min(8)]);
}

fn fn_tempnam(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let dir = path_param(args, 0, "tempnam", "directory")?;
    let prefix = path_param(args, 1, "tempnam", "prefix")?;
    // php_basename(prefix), truncated to 63 bytes.
    let end = prefix
        .iter()
        .rposition(|&b| b != b'/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let start = prefix[..end]
        .iter()
        .rposition(|&b| b == b'/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut pfx = prefix[start..end].to_vec();
    pfx.truncate(63);
    if !dir.is_empty() {
        if let Some(p) = open_temp(&dir, &pfx) {
            return Ok(Value::str(p));
        }
    }
    match open_temp(&temp_dir(), &pfx) {
        Some(p) => {
            if !dir.is_empty() {
                crate::notice(
                    host,
                    "tempnam",
                    "file created in the system's temporary directory",
                )?;
            }
            Ok(Value::str(p))
        }
        None => Ok(Value::Bool(false)),
    }
}

/// chown / chgrp / lchown / lchgrp: a user/group name or id; plain paths.
fn do_chown(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    group: bool,
    link: bool,
) -> Result<Value, Thrown> {
    let path = path_param(args, 0, fname, "filename")?;
    let who = args.get(1).map(deref).unwrap_or(Value::Null);
    let id: u32 = match &who {
        Value::Int(n) => *n as u32,
        Value::Str(s) => {
            let Ok(c) = CString::new(s.as_bytes().to_vec()) else {
                return Ok(Value::Bool(false));
            };
            let found = unsafe {
                if group {
                    let g = libc::getgrnam(c.as_ptr());
                    (!g.is_null()).then(|| (*g).gr_gid)
                } else {
                    let p = libc::getpwnam(c.as_ptr());
                    (!p.is_null()).then(|| (*p).pw_uid)
                }
            };
            match found {
                Some(id) => id,
                None => {
                    let what = if group { "group" } else { "user" };
                    crate::warn(
                        host,
                        fname,
                        &format!(
                            "Unable to find {what} ID for {}",
                            String::from_utf8_lossy(s.as_bytes())
                        ),
                    )?;
                    return Ok(Value::Bool(false));
                }
            }
        }
        other => {
            let what = if group { "group" } else { "user" };
            return Err(Thrown::type_error(format!(
                "{fname}(): Argument #2 (${what}) must be of type string|int, {} given",
                given(host, other)
            )));
        }
    };
    let Ok(c) = CString::new(crate::streams::file_path(&path).to_vec()) else {
        return Ok(Value::Bool(false));
    };
    let (uid, gid) = if group {
        (u32::MAX, id)
    } else {
        (id, u32::MAX)
    };
    let r = unsafe {
        if link {
            libc::lchown(c.as_ptr(), uid, gid)
        } else {
            libc::chown(c.as_ptr(), uid, gid)
        }
    };
    if r != 0 {
        let e = std::io::Error::last_os_error();
        crate::warn(
            host,
            fname,
            &crate::streams::strerror(e.raw_os_error().unwrap_or(0)),
        )?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

fn fn_chown(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    do_chown(host, args, "chown", false, false)
}
fn fn_chgrp(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    do_chown(host, args, "chgrp", true, false)
}
fn fn_lchown(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    do_chown(host, args, "lchown", false, true)
}
fn fn_lchgrp(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    do_chown(host, args, "lchgrp", true, true)
}

/// `readfile($filename)`: copy a file to the output; bytes written or false.
fn fn_readfile(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(args, 0, "readfile", "filename")?;
    touch_ctx(host, args, 2);
    crate::streams::clear_errno();
    let data = if args.get(1).is_some_and(|v| conv::to_bool(&deref(v))) {
        match resolve_include_path(&path) {
            Some(p) => host.stream_get_contents(&p),
            None => host.stream_get_contents(&path),
        }
    } else {
        host.stream_get_contents(&path)
    };
    let Some(data) = data else {
        if crate::streams::last_errno() == Some(libc::EISDIR) {
            // The open succeeds; the first read fails (php returns -1).
            host.consume_resource_ids(1);
            crate::notice(
                host,
                "readfile",
                "Read of 8192 bytes failed with errno=21 Is a directory",
            )?;
            return Ok(Value::Int(-1));
        }
        crate::open_failed(host, "readfile", &path)?;
        return Ok(Value::Bool(false));
    };
    host.consume_resource_ids(1);
    host.write_out(&data);
    Ok(Value::Int(data.len() as i64))
}

/// `fgetcsv($stream, $length = null, $separator = ",", $enclosure = "\"",
/// $escape = "\\")`: one CSV record, reading on while an enclosure is open;
/// false at EOF, `[null]` for a blank line.
fn fn_fgetcsv(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fgetcsv", "stream")?;
    if args.len() < 5 {
        crate::deprecated(
            host,
            "fgetcsv",
            "the $escape parameter must be provided as its default value will change",
        )?;
    }
    let ch = |i: usize, d: u8| -> Option<u8> {
        match args.get(i).map(deref) {
            None | Some(Value::Null) => Some(d),
            Some(v) => conv::to_string_bytes(&v).first().copied(),
        }
    };
    let sep = ch(2, b',').unwrap_or(b',');
    let enc = ch(3, b'"').unwrap_or(b'"');
    let esc = ch(4, b'\\');
    let Some(mut buf) = host.stream_gets(&h) else {
        return Ok(Value::Bool(false));
    };
    // An enclosure left open at the end of the line continues on the next.
    let open = |b: &[u8]| {
        let (mut inq, mut i) = (false, 0);
        while i < b.len() {
            let c = b[i];
            if inq && Some(c) == esc && Some(c) != Some(enc) && i + 1 < b.len() {
                i += 2;
                continue;
            }
            if c == enc {
                inq = !inq;
            }
            i += 1;
        }
        inq
    };
    while open(&buf) {
        match host.stream_gets(&h) {
            Some(more) => buf.extend_from_slice(&more),
            None => break,
        }
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    } else if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    Ok(Value::Array(crate::parse_fns::parse_csv(
        &buf, sep, enc, esc,
    )))
}

/// php_fputcsv: a field is enclosed when it holds the separator, the
/// enclosure, the escape character or whitespace; enclosures inside are
/// doubled (unless escaped). Returns the bytes written.
fn fn_fputcsv(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fputcsv", "stream")?;
    if args.len() < 5 {
        crate::deprecated(
            host,
            "fputcsv",
            "the $escape parameter must be provided as its default value will change",
        )?;
    }
    let Value::Array(fields) = args.get(1).map(deref).unwrap_or(Value::Null) else {
        return Err(Thrown::type_error(
            "fputcsv(): Argument #2 ($fields) must be of type array",
        ));
    };
    let ch = |i: usize, d: u8| -> Option<u8> {
        match args.get(i).map(deref) {
            None | Some(Value::Null) => Some(d),
            Some(v) => conv::to_string_bytes(&v).first().copied(),
        }
    };
    let sep = ch(2, b',').unwrap_or(b',');
    let enc = ch(3, b'"').unwrap_or(b'"');
    let esc = ch(4, b'\\');
    let eol = match args.get(5).map(deref) {
        None | Some(Value::Null) => b"\n".to_vec(),
        Some(v) => conv::to_string_bytes(&v),
    };
    let mut line = Vec::new();
    for (i, (_, v)) in fields.iter().enumerate() {
        if i > 0 {
            line.push(sep);
        }
        let f = conv::to_string_bytes(&deref(v));
        let needs = f.iter().any(|&c| {
            c == sep || c == enc || Some(c) == esc || matches!(c, b'\n' | b'\r' | b'\t' | b' ')
        });
        if needs {
            line.push(enc);
            let mut escaped = false;
            for &c in &f {
                if Some(c) == esc && esc.is_some() {
                    escaped = true;
                } else if !escaped && c == enc {
                    line.push(enc);
                } else {
                    escaped = false;
                }
                line.push(c);
            }
            line.push(enc);
        } else {
            line.extend_from_slice(&f);
        }
    }
    line.extend_from_slice(&eol);
    Ok(match host.stream_write(&h, &line) {
        Some(n) => Value::Int(n as i64),
        None => Value::Bool(false),
    })
}

/// `tmpfile()`: a new unlinked temporary file opened "w+b".
fn fn_tmpfile(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let Some(path) = open_temp(&temp_dir(), b"php") else {
        return Ok(Value::Bool(false));
    };
    let r = host.stream_open(&path, b"w+b");
    if let Ok(c) = CString::new(path) {
        unsafe { libc::unlink(c.as_ptr()) };
    }
    Ok(r.unwrap_or(Value::Bool(false)))
}

/// `stream_get_line($stream, $length, $ending = "")`: read up to `$length`
/// bytes (0 = the default chunk), stopping before `$ending` (consumed).
fn fn_stream_get_line(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "stream_get_line", "stream")?;
    let mut max = int_arg(args, 1, 0);
    if max < 0 {
        return Err(Thrown::error(
            "ValueError",
            "stream_get_line(): Argument #2 ($length) must be greater than or equal to 0",
        ));
    }
    if max == 0 {
        max = 8192;
    }
    let ending = str_arg(args, 2);
    let mut out = Vec::new();
    while (out.len() as i64) < max {
        let Some(b) = host.stream_read(&h, 1) else {
            break;
        };
        if b.is_empty() {
            break;
        }
        out.extend_from_slice(&b);
        if !ending.is_empty() && out.ends_with(&ending) {
            out.truncate(out.len() - ending.len());
            return Ok(Value::str(out));
        }
    }
    if out.is_empty() && host.stream_eof(&h) {
        return Ok(Value::Bool(false));
    }
    Ok(Value::str(out))
}

/// `fpassthru($stream)`: output the rest of the stream; bytes written.
fn fn_fpassthru(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fpassthru", "stream")?;
    let data = host.stream_read(&h, -1).unwrap_or_default();
    host.write_out(&data);
    Ok(Value::Int(data.len() as i64))
}

fn fn_stream_is_local(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = args.first().map(deref).unwrap_or(Value::Null);
    if let Value::Resource(_) = v {
        let (h, _) = stream_param(host, args, 0, "stream_is_local", "stream")?;
        // A stream without a wrapper (pipe, socket) is not local in php.
        let wrapperless = host
            .stream_meta(&h)
            .is_some_and(|m| m.wrapper_type.is_empty());
        return Ok(Value::Bool(!wrapperless));
    }
    let url = conv::to_string_bytes(&v);
    let (scheme, _) = split_scheme(&url);
    let lower = scheme.to_ascii_lowercase();
    // Wrappers with is_url=1 in php: http, https, ftp, ftps, data (rfc2397).
    let is_url = matches!(
        lower.as_slice(),
        b"http" | b"https" | b"ftp" | b"ftps" | b"data"
    );
    // php also treats "data:" (no slashes) as the rfc2397 wrapper.
    let is_data = url.len() >= 5 && url[..5].eq_ignore_ascii_case(b"data:");
    Ok(Value::Bool(!(is_url || is_data)))
}

/// php_resolve_path() against the default include_path (".").
fn resolve_include_path(name: &[u8]) -> Option<Vec<u8>> {
    if name.contains(&0) {
        return None;
    }
    // Paths with a protocol: only file:// resolves.
    let mut p = 0;
    while p < name.len()
        && (name[p].is_ascii_alphanumeric() || matches!(name[p], b'+' | b'-' | b'.'))
    {
        p += 1;
    }
    if p > 1
        && name.get(p) == Some(&b':')
        && name.get(p + 1) == Some(&b'/')
        && name.get(p + 2) == Some(&b'/')
    {
        let (scheme, _) = split_scheme(name);
        if scheme.eq_ignore_ascii_case(b"file") {
            return realpath_bytes(crate::streams::file_path(name));
        }
        return None;
    }
    if name.starts_with(b"./") || name.starts_with(b"../") || name.starts_with(b"/") {
        return realpath_bytes(name);
    }
    let mut t = b"./".to_vec();
    t.extend_from_slice(name);
    realpath_bytes(&t)
}

fn fn_stream_resolve_include_path(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = path_param(args, 0, "stream_resolve_include_path", "filename")?;
    Ok(resolve_include_path(&name)
        .map(Value::str)
        .unwrap_or(Value::Bool(false)))
}

pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("SEEK_SET", Value::Int(0)),
        ("SEEK_CUR", Value::Int(1)),
        ("SEEK_END", Value::Int(2)),
        ("SCANDIR_SORT_ASCENDING", Value::Int(0)),
        ("SCANDIR_SORT_DESCENDING", Value::Int(1)),
        ("SCANDIR_SORT_NONE", Value::Int(2)),
        ("GLOB_BRACE", Value::Int(GLOB_BRACE)),
        ("GLOB_ERR", Value::Int(GLOB_ERR)),
        ("GLOB_MARK", Value::Int(GLOB_MARK)),
        ("GLOB_NOCHECK", Value::Int(GLOB_NOCHECK)),
        ("GLOB_NOESCAPE", Value::Int(GLOB_NOESCAPE)),
        ("GLOB_NOSORT", Value::Int(GLOB_NOSORT)),
        ("GLOB_ONLYDIR", Value::Int(GLOB_ONLYDIR)),
        ("GLOB_AVAILABLE_FLAGS", Value::Int(GLOB_AVAILABLE)),
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
    put(m, "fopen", fn_fopen, 2, Some(4));
    put(
        m,
        "stream_context_get_default",
        fn_stream_context_get_default,
        0,
        Some(1),
    );
    put(m, "mkdir", fn_mkdir, 1, Some(4));
    put(m, "link", fn_link, 2, Some(2));
    put(m, "linkinfo", fn_linkinfo, 1, Some(1));
    put(m, "copy", fn_copy, 2, Some(3));
    put(m, "opendir", fn_opendir, 1, Some(2));
    put(m, "dir", fn_dir, 1, Some(2));
    put(m, "readdir", fn_readdir, 0, Some(1));
    put(m, "rewinddir", fn_rewinddir, 0, Some(1));
    put(m, "closedir", fn_closedir, 0, Some(1));
    put(m, "__directory_op", fn_directory_op, 2, Some(2));
    put(m, "scandir", fn_scandir, 1, Some(3));
    put(m, "glob", fn_glob, 1, Some(2));
    put(m, "file", fn_file, 1, Some(3));
    put(m, "stat", fn_stat, 1, Some(1));
    put(m, "lstat", fn_lstat, 1, Some(1));
    put(m, "fstat", fn_fstat, 1, Some(1));
    put(m, "fileinode", fn_fileinode, 1, Some(1));
    put(m, "fileowner", fn_fileowner, 1, Some(1));
    put(m, "filegroup", fn_filegroup, 1, Some(1));
    put(m, "fileatime", fn_fileatime, 1, Some(1));
    put(m, "filemtime", fn_filemtime, 1, Some(1));
    put(m, "filectime", fn_filectime, 1, Some(1));
    put(m, "filesize", fn_filesize, 1, Some(1));
    put(m, "fileperms", fn_fileperms, 1, Some(1));
    put(m, "is_link", fn_is_link, 1, Some(1));
    put(m, "is_file", fn_is_file, 1, Some(1));
    put(m, "is_dir", fn_is_dir, 1, Some(1));
    put(m, "filetype", fn_filetype, 1, Some(1));
    put(m, "is_readable", fn_is_readable, 1, Some(1));
    put(m, "is_writable", fn_is_writable, 1, Some(1));
    put(m, "is_writeable", fn_is_writable, 1, Some(1));
    put(m, "is_executable", fn_is_executable, 1, Some(1));
    put(m, "flock", fn_flock, 2, Some(3));
    put(m, "ftruncate", fn_ftruncate, 2, Some(2));
    put(m, "fseek", fn_fseek, 2, Some(3));
    put(m, "ftell", fn_ftell, 1, Some(1));
    put(m, "rewind", fn_rewind, 1, Some(1));
    put(m, "fgets", fn_fgets, 1, Some(2));
    put(m, "fread", fn_fread, 2, Some(2));
    put(m, "fwrite", fn_fwrite, 2, Some(3));
    put(m, "fputs", fn_fwrite, 2, Some(3));
    put(m, "fclose", fn_fclose, 1, Some(1));
    put(m, "feof", fn_feof, 1, Some(1));
    put(m, "fflush", fn_fflush, 1, Some(1));
    put(m, "fgetc", fn_fgetc, 1, Some(1));
    put(m, "stream_get_contents", fn_stream_get_contents, 1, Some(3));
    put(
        m,
        "stream_get_meta_data",
        fn_stream_get_meta_data,
        1,
        Some(1),
    );
    put(m, "sys_get_temp_dir", fn_sys_get_temp_dir, 0, Some(0));
    put(m, "realpath", fn_realpath, 1, Some(1));
    put(m, "tempnam", fn_tempnam, 2, Some(2));
    put(m, "fgetcsv", fn_fgetcsv, 1, Some(5));
    put(m, "fputcsv", fn_fputcsv, 2, Some(6));
    put(m, "tmpfile", fn_tmpfile, 0, Some(0));
    put(m, "stream_get_line", fn_stream_get_line, 2, Some(3));
    put(m, "chown", fn_chown, 2, Some(2));
    put(m, "chgrp", fn_chgrp, 2, Some(2));
    put(m, "lchown", fn_lchown, 2, Some(2));
    put(m, "lchgrp", fn_lchgrp, 2, Some(2));
    put(m, "readfile", fn_readfile, 1, Some(3));
    put(m, "fpassthru", fn_fpassthru, 1, Some(1));
    put(m, "stream_is_local", fn_stream_is_local, 1, Some(1));
    put(
        m,
        "stream_resolve_include_path",
        fn_stream_resolve_include_path,
        1,
        Some(1),
    );
}
