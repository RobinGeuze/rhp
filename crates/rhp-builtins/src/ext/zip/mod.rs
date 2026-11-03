//! The `zip` extension: `ZipArchive` reading *and* writing, backed by the
//! in-memory archive model in `zip_ext` (open archives keyed by id in a
//! thread-local table). The PHP-level class (below, in the prelude) holds the
//! id and calls one dispatcher builtin, `__zip($op, $id, ...$args)`.

use std::collections::HashMap;

use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::zip_ext::{self, Archive};
use crate::{Builtin, Host, Thrown};

/// `ZipArchive`'s handlers (`zip_object_handlers`: no clone).
pub(crate) static CLASSES: &[crate::object::ClassHandlers] = &[crate::object::ClassHandlers {
    class: "ZipArchive",
    handlers: &crate::object::ObjectHandlers::UNCLONEABLE,
}];

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn arg_bytes(args: &[Value], i: usize) -> Vec<u8> {
    args.get(i)
        .map(|v| conv::to_string_bytes(&deref(v)))
        .unwrap_or_default()
}

fn arg_int(args: &[Value], i: usize) -> i64 {
    args.get(i).map(|v| conv::to_int(&deref(v))).unwrap_or(0)
}

/// php's `expand_filepath`: make relative paths absolute against the cwd and
/// fold `.`/`..` segments, resolving symlinks in the part that exists.
fn expand_filepath(path: &[u8]) -> Vec<u8> {
    let p = String::from_utf8_lossy(path).into_owned();
    if p.contains("://") {
        return path.to_vec();
    }
    let full = if p.starts_with('/') {
        p
    } else {
        let cwd = std::env::current_dir()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("{cwd}/{p}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in full.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    // CWD_FILEPATH: symlinks in the existing leading part are resolved
    // (tsrm_realpath_r), the rest is kept as written.
    for keep in (1..=parts.len()).rev() {
        let head = format!("/{}", parts[..keep].join("/"));
        if let Ok(real) = std::fs::canonicalize(&head) {
            let mut out = real.to_string_lossy().into_owned();
            for seg in &parts[keep..] {
                out.push('/');
                out.push_str(seg);
            }
            return out.into_bytes();
        }
    }
    format!("/{}", parts.join("/")).into_bytes()
}

fn stat_array(i: i64, e: &zip_ext::Entry) -> Value {
    let (crc, comp_size, method) = e.stat_bits();
    let mut a = PhpArray::new();
    a.insert(ArrayKey::from_bytes(b"name"), Value::str(e.name.clone()));
    a.insert(ArrayKey::from_bytes(b"index"), Value::Int(i));
    a.insert(ArrayKey::from_bytes(b"crc"), Value::Int(crc as i64));
    a.insert(ArrayKey::from_bytes(b"size"), Value::Int(e.size() as i64));
    a.insert(ArrayKey::from_bytes(b"mtime"), Value::Int(e.mtime));
    a.insert(
        ArrayKey::from_bytes(b"comp_size"),
        Value::Int(comp_size as i64),
    );
    a.insert(
        ArrayKey::from_bytes(b"comp_method"),
        Value::Int(method as i64),
    );
    a.insert(ArrayKey::from_bytes(b"encryption_method"), Value::Int(0));
    Value::Array(a)
}

/// Run `f` on archive `id`, recording a failing op's error code as its status.
fn on<R>(id: i64, f: impl FnOnce(&mut Archive) -> R) -> Option<R> {
    zip_ext::with(id, f)
}

fn fail(id: i64, code: i64) -> Value {
    on(id, |a| a.status = code);
    Value::Bool(false)
}

fn file_stat(path: &str) -> Option<(i64, u32)> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(path).ok()?;
    Some((m.mtime(), m.mode()))
}

fn fn_zip(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = arg_bytes(args, 0);
    let id = arg_int(args, 1);
    let v = match op.as_slice() {
        b"open" => {
            let path = arg_bytes(args, 2);
            let flags = arg_int(args, 3);
            let resolved = expand_filepath(&path);
            let rs = String::from_utf8_lossy(&resolved).into_owned();
            let bytes = if rs.contains("://") {
                host.stream_get_contents(&resolved)
            } else {
                match std::fs::metadata(&rs) {
                    Ok(m) if m.is_dir() => return Ok(Value::Int(zip_ext::ER_READ_DIR)),
                    Ok(_) => match std::fs::read(&rs) {
                        Ok(b) => Some(b),
                        Err(_) => return Ok(Value::Int(zip_ext::ER_OPEN)),
                    },
                    Err(_) => None,
                }
            };
            match Archive::open(resolved.clone(), bytes, flags) {
                Ok(a) => {
                    zip_ext::insert(id, a);
                    Value::str(resolved)
                }
                Err(code) => Value::Int(code),
            }
        }
        b"close" => match zip_ext::remove(id) {
            Some(a) => match a.commit() {
                Ok(()) => Value::Int(0),
                Err(c) => Value::Int(c),
            },
            None => Value::Int(0),
        },
        b"discard" => {
            zip_ext::remove(id);
            Value::Null
        }
        b"is_open" => Value::Bool(zip_ext::with(id, |_| ()).is_some()),
        b"num" => Value::Int(on(id, |a| a.entries.len() as i64).unwrap_or(0)),
        b"status" => Value::Int(on(id, |a| a.status).unwrap_or(0)),
        b"clear_error" => {
            on(id, |a| a.status = 0);
            Value::Null
        }
        b"add_string" | b"add_dir" | b"add_file" => {
            let mut name = arg_bytes(args, 2);
            let flags = arg_int(args, 4);
            let overwrite = flags & 8192 != 0;
            let (data, mtime, attr) = match op.as_slice() {
                b"add_string" => (arg_bytes(args, 3), zip_ext::now(), 0o100666u32 << 16),
                b"add_dir" => {
                    if name.last() != Some(&b'/') {
                        name.push(b'/');
                    }
                    (Vec::new(), zip_ext::now(), 0o40777u32 << 16)
                }
                _ => {
                    let path = String::from_utf8_lossy(&arg_bytes(args, 3)).into_owned();
                    let start = arg_int(args, 5).max(0) as usize;
                    let length = arg_int(args, 6);
                    let Some((mtime, mode)) = file_stat(&path) else {
                        host.raise_diagnostic(
                            crate::E_WARNING,
                            "ZipArchive::addFile(): No such file or directory",
                        );
                        return Ok(Value::Bool(false));
                    };
                    let Ok(mut data) = std::fs::read(&path) else {
                        return Ok(Value::Bool(false));
                    };
                    if start > 0 || length > 0 {
                        let s = start.min(data.len());
                        let e = if length > 0 {
                            (s + length as usize).min(data.len())
                        } else {
                            data.len()
                        };
                        data = data[s..e].to_vec();
                    }
                    (data, mtime, (mode & 0xFFFF) << 16)
                }
            };
            let from_file = op.as_slice() == b"add_file";
            match on(id, |a| a.add(name, data, mtime, attr, overwrite)) {
                Some(Ok(i)) => {
                    if from_file {
                        on(id, |a| a.entries[i].from_file = true);
                    }
                    Value::Int(i as i64)
                }
                Some(Err(c)) => fail(id, c),
                None => Value::Bool(false),
            }
        }
        b"replace_file" => {
            let path = String::from_utf8_lossy(&arg_bytes(args, 2)).into_owned();
            let index = arg_int(args, 3);
            match (file_stat(&path), std::fs::read(&path)) {
                (Some((mtime, _)), Ok(data)) => {
                    Value::Bool(on(id, |a| a.replace(index, data, mtime)).unwrap_or(false))
                }
                _ => {
                    host.raise_diagnostic(
                        crate::E_WARNING,
                        "ZipArchive::replaceFile(): No such file or directory",
                    );
                    Value::Bool(false)
                }
            }
        }
        b"locate" => {
            let name = arg_bytes(args, 2);
            let flags = arg_int(args, 3);
            match on(id, |a| a.locate(&name, flags & 1 != 0, flags & 2 != 0)).flatten() {
                Some(i) => Value::Int(i as i64),
                None => fail(id, zip_ext::ER_NOENT),
            }
        }
        b"name" => {
            let i = arg_int(args, 2);
            on(id, |a| a.live(i).map(|e| Value::str(e.name.clone())))
                .flatten()
                .unwrap_or(Value::Bool(false))
        }
        b"stat" => {
            let i = arg_int(args, 2);
            on(id, |a| a.live(i).map(|e| stat_array(i, e)))
                .flatten()
                .unwrap_or(Value::Bool(false))
        }
        b"get" => {
            let i = arg_int(args, 2);
            let len = arg_int(args, 3);
            if on(id, |a| a.live(i).map(|e| e.from_file)).flatten() == Some(true) {
                return Ok(fail(id, zip_ext::ER_CHANGED));
            }
            match on(id, |a| a.live(i).map(|e| e.contents())).flatten() {
                Some(Some(mut d)) => {
                    if len > 0 && (len as usize) < d.len() {
                        d.truncate(len as usize);
                    }
                    Value::str(d)
                }
                Some(None) => fail(id, zip_ext::ER_COMPNOTSUPP),
                None => Value::Bool(false),
            }
        }
        b"delete" => {
            let i = arg_int(args, 2);
            Value::Bool(on(id, |a| a.delete(i)).unwrap_or(false))
        }
        b"rename" => {
            let i = arg_int(args, 2);
            let name = arg_bytes(args, 3);
            match on(id, |a| a.rename(i, name)) {
                Some(Ok(())) => Value::Bool(true),
                Some(Err(c)) => fail(id, c),
                None => Value::Bool(false),
            }
        }
        b"set_attr" => {
            let i = arg_int(args, 2);
            let opsys = arg_int(args, 3) as u8;
            let attr = arg_int(args, 4) as u32;
            Value::Bool(
                on(id, |a| {
                    let ok = a.live_mut(i).map(|e| {
                        e.opsys = opsys;
                        e.attr = attr;
                    });
                    if ok.is_some() {
                        a.touch();
                    }
                    ok.is_some()
                })
                .unwrap_or(false),
            )
        }
        b"get_attr" => {
            let i = arg_int(args, 2);
            match on(id, |a| a.live(i).map(|e| (e.opsys, e.attr))).flatten() {
                Some((o, at)) => {
                    let mut r = PhpArray::new();
                    r.push(Value::Int(o as i64));
                    r.push(Value::Int(at as i64));
                    Value::Array(r)
                }
                None => Value::Bool(false),
            }
        }
        b"set_comment" => {
            let i = arg_int(args, 2);
            let c = arg_bytes(args, 3);
            Value::Bool(
                on(id, |a| {
                    let ok = a.live_mut(i).map(|e| e.comment = c).is_some();
                    if ok {
                        a.touch();
                    }
                    ok
                })
                .unwrap_or(false),
            )
        }
        b"get_comment" => {
            let i = arg_int(args, 2);
            on(id, |a| a.live(i).map(|e| Value::str(e.comment.clone())))
                .flatten()
                .unwrap_or(Value::Bool(false))
        }
        b"set_archive_comment" => {
            let c = arg_bytes(args, 2);
            on(id, |a| {
                a.comment = c;
                a.touch();
            });
            Value::Bool(true)
        }
        b"get_archive_comment" => {
            on(id, |a| Value::str(a.comment.clone())).unwrap_or(Value::Bool(false))
        }
        b"set_mtime" => {
            let i = arg_int(args, 2);
            let t = arg_int(args, 3);
            Value::Bool(
                on(id, |a| {
                    let ok = a.live_mut(i).map(|e| e.mtime = t).is_some();
                    if ok {
                        a.touch();
                    }
                    ok
                })
                .unwrap_or(false),
            )
        }
        b"set_compression" => {
            let i = arg_int(args, 2);
            let m = arg_int(args, 3) as i32;
            if !matches!(m, -1 | 0 | 8) {
                return Ok(fail(id, zip_ext::ER_COMPNOTSUPP));
            }
            Value::Bool(
                on(id, |a| {
                    let ok = a.live_mut(i).map(|e| e.method = m).is_some();
                    if ok {
                        a.touch();
                    }
                    ok
                })
                .unwrap_or(false),
            )
        }
        b"unchange" => {
            let i = arg_int(args, 2);
            Value::Bool(on(id, |a| a.unchange(i)).unwrap_or(false))
        }
        b"unchange_all" => {
            on(id, |a| a.unchange_all());
            Value::Bool(true)
        }
        b"unchange_archive" => {
            on(id, |a| a.unchange_archive());
            Value::Bool(true)
        }
        b"extract" => {
            let dest = String::from_utf8_lossy(&arg_bytes(args, 2)).into_owned();
            let dest = dest.trim_end_matches('/').to_string();
            let dest = if dest.is_empty() {
                "/".to_string()
            } else {
                dest
            };
            // files: null → all; string / array → by name.
            let names: Option<Vec<Vec<u8>>> = match args.get(3).map(deref) {
                None | Some(Value::Null) => None,
                Some(Value::Array(a)) => Some(
                    a.iter()
                        .map(|(_, v)| conv::to_string_bytes(&deref(v)))
                        .collect(),
                ),
                Some(v) => Some(vec![conv::to_string_bytes(&v)]),
            };
            if !std::path::Path::new(&dest).is_dir() && std::fs::create_dir_all(&dest).is_err() {
                return Ok(Value::Bool(false));
            }
            let ok = on(id, |a| match &names {
                None => a
                    .entries
                    .iter()
                    .filter(|e| !e.deleted)
                    .all(|e| zip_ext::extract_entry(e, &dest)),
                Some(ns) => ns.iter().all(|n| match a.locate(n, false, false) {
                    Some(i) => zip_ext::extract_entry(&a.entries[i], &dest),
                    None => false,
                }),
            });
            Value::Bool(ok.unwrap_or(false))
        }
        b"status_string" => {
            let code = arg_int(args, 2);
            let s = zip_ext::ERROR_STRINGS
                .get(code as usize)
                .copied()
                .unwrap_or("Unknown error");
            Value::str(s)
        }
        _ => Value::Null,
    };
    Ok(v)
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    m.insert(
        "__zip",
        Builtin {
            name: "__zip",
            func: fn_zip,
            min_args: 2,
            max_args: None,
        },
    );
}

pub const PRELUDE_PHP: &str = concat!(
    r#"
class ZipArchive implements Countable {
    public $lastId = -1;
    public $status = 0;
    public $statusSys = 0;
    public $numFiles = 0;
    public $filename = '';
    public $comment = '';

    const CREATE = 1; const EXCL = 2; const CHECKCONS = 4; const OVERWRITE = 8; const RDONLY = 16;
    const FL_NOCASE = 1; const FL_NODIR = 2; const FL_COMPRESSED = 4; const FL_UNCHANGED = 8;
    const FL_RECOMPRESS = 16; const FL_ENCRYPTED = 32; const FL_OVERWRITE = 8192;
    const FL_LOCAL = 256; const FL_CENTRAL = 512; const FL_ENC_GUESS = 0; const FL_ENC_RAW = 64;
    const FL_ENC_STRICT = 128; const FL_ENC_UTF_8 = 2048; const FL_ENC_CP437 = 4096;
    const FL_OPEN_FILE_NOW = 1073741824;
    const CM_DEFAULT = -1; const CM_STORE = 0; const CM_SHRINK = 1; const CM_REDUCE_1 = 2;
    const CM_REDUCE_2 = 3; const CM_REDUCE_3 = 4; const CM_REDUCE_4 = 5; const CM_IMPLODE = 6;
    const CM_DEFLATE = 8; const CM_DEFLATE64 = 9; const CM_PKWARE_IMPLODE = 10; const CM_BZIP2 = 12;
    const CM_LZMA = 14; const CM_LZMA2 = 33; const CM_ZSTD = 93; const CM_XZ = 95; const CM_TERSE = 18;
    const CM_LZ77 = 19; const CM_WAVPACK = 97; const CM_PPMD = 98;
    const ER_OK = 0; const ER_MULTIDISK = 1; const ER_RENAME = 2; const ER_CLOSE = 3;
    const ER_SEEK = 4; const ER_READ = 5; const ER_WRITE = 6; const ER_CRC = 7;
    const ER_ZIPCLOSED = 8; const ER_NOENT = 9; const ER_EXISTS = 10; const ER_OPEN = 11;
    const ER_TMPOPEN = 12; const ER_ZLIB = 13; const ER_MEMORY = 14; const ER_CHANGED = 15;
    const ER_COMPNOTSUPP = 16; const ER_EOF = 17; const ER_INVAL = 18; const ER_NOZIP = 19;
    const ER_INTERNAL = 20; const ER_INCONS = 21; const ER_REMOVE = 22; const ER_DELETED = 23;
    const ER_ENCRNOTSUPP = 24; const ER_RDONLY = 25; const ER_NOPASSWD = 26; const ER_WRONGPASSWD = 27;
    const ER_OPNOTSUPP = 28; const ER_INUSE = 29; const ER_TELL = 30; const ER_COMPRESSED_DATA = 31;
    const ER_CANCELLED = 32; const ER_DATA_LENGTH = 33; const ER_NOT_ALLOWED = 34; const ER_TRUNCATED_ZIP = 35;
    const AFL_RDONLY = 2; const AFL_IS_TORRENTZIP = 4; const AFL_WANT_TORRENTZIP = 8;
    const AFL_CREATE_OR_KEEP_FILE_FOR_EMPTY_ARCHIVE = 16;
    const OPSYS_DOS = 0; const OPSYS_AMIGA = 1; const OPSYS_OPENVMS = 2; const OPSYS_UNIX = 3;
    const OPSYS_VM_CMS = 4; const OPSYS_ATARI_ST = 5; const OPSYS_OS_2 = 6; const OPSYS_MACINTOSH = 7;
    const OPSYS_Z_SYSTEM = 8; const OPSYS_CPM = 9; const OPSYS_WINDOWS_NTFS = 10; const OPSYS_MVS = 11;
    const OPSYS_VSE = 12; const OPSYS_ACORN_RISC = 13; const OPSYS_VFAT = 14; const OPSYS_ALTERNATE_MVS = 15;
    const OPSYS_BEOS = 16; const OPSYS_TANDEM = 17; const OPSYS_OS_400 = 18; const OPSYS_OS_X = 19;
    const OPSYS_DEFAULT = 3;
    const EM_NONE = 0; const EM_TRAD_PKWARE = 1; const EM_AES_128 = 257; const EM_AES_192 = 258;
    const EM_AES_256 = 259; const EM_UNKNOWN = 65535;
    const LIBZIP_VERSION = '"#,
    crate::version::libzip!(),
    r#"';
    const LENGTH_TO_END = 0; const LENGTH_UNCHECKED = -2;

    // The open archive is keyed by spl_object_id($this) on the Rust side, so
    // no handle property shows up in var_dump / casts.
    private function __h() { return spl_object_id($this); }
    private function __chk() {
        if (!__zip('is_open', $this->__h())) { throw new ValueError('Invalid or uninitialized Zip object'); }
        return $this->__h();
    }
    private function __sync() {
        $this->numFiles = __zip('num', $this->__h());
        $this->status = __zip('status', $this->__h());
        $this->comment = (string) __zip('get_archive_comment', $this->__h());
    }
    private function __idx($name, $flags = 0) {
        $i = __zip('locate', $this->__h(), (string) $name, $flags);
        return $i === false ? -1 : $i;
    }

    public function open(string $filename, int $flags = 0): bool|int {
        if ($filename === '') { throw new ValueError('ZipArchive::open(): Argument #1 ($filename) must not be empty'); }
        if (str_contains($filename, "\0")) { throw new ValueError('ZipArchive::open(): Argument #1 ($filename) must not contain any null bytes'); }
        __zip('close', $this->__h());
        $r = __zip('open', $this->__h(), $filename, $flags);
        if (is_int($r)) { return $r; }
        $this->filename = $r;
        $this->__sync();
        return true;
    }
    public function setPassword(string $password): bool { $this->__chk(); return true; }
    public function close(): bool {
        $id = $this->__chk();
        $code = __zip('close', $id);
        
        $this->numFiles = 0; $this->filename = ''; $this->comment = ''; $this->lastId = -1;
        $this->status = $code;
        return $code === 0;
    }
    public function count(): int { $this->__chk(); return __zip('num', $this->__h()); }
    public function getStatusString(): string {
        if (!__zip('is_open', $this->__h())) { return __zip('status_string', 0, $this->status); }
        return __zip('status_string', 0, __zip('status', $this->__h()));
    }
    public function clearError(): void { if (__zip('is_open', $this->__h())) { __zip('clear_error', $this->__h()); } $this->status = 0; }
    private function __added($r) {
        $this->__sync();
        if ($r === false) { return false; }
        $this->lastId = $r;
        return true;
    }
    public function addEmptyDir(string $dirname, int $flags = 0): bool {
        $this->__chk();
        return $this->__added(__zip('add_dir', $this->__h(), $dirname, '', $flags));
    }
    public function addFromString(string $name, string $content, int $flags = ZipArchive::FL_OVERWRITE): bool {
        $this->__chk();
        return $this->__added(__zip('add_string', $this->__h(), $name, $content, $flags));
    }
    public function addFile(string $filepath, string $entryname = '', int $start = 0, int $length = ZipArchive::LENGTH_TO_END, int $flags = ZipArchive::FL_OVERWRITE): bool {
        $this->__chk();
        if ($filepath === '') { throw new ValueError('ZipArchive::addFile(): Argument #1 ($filepath) must not be empty'); }
        if (str_contains($filepath, "\0")) { throw new ValueError('ZipArchive::addFile(): Argument #1 ($filepath) must not contain any null bytes'); }
        if ($entryname === '') { $entryname = $filepath; }
        return $this->__added(__zip('add_file', $this->__h(), $entryname, $filepath, $flags, $start, $length));
    }
    public function replaceFile(string $filepath, int $index, int $start = 0, int $length = ZipArchive::LENGTH_TO_END, int $flags = 0): bool {
        $this->__chk();
        $r = __zip('replace_file', $this->__h(), $filepath, $index);
        $this->__sync();
        return $r;
    }
    public function addGlob(string $pattern, int $flags = 0, array $options = []): array|false {
        $this->__chk();
        $files = glob($pattern, $flags);
        if ($files === false) { return false; }
        return $this->__addList($files, $options);
    }
    public function addPattern(string $pattern, string $path = '.', array $options = []): array|false {
        $this->__chk();
        $files = [];
        foreach ((array) @scandir($path) as $f) {
            if ($f === '.' || $f === '..' || !preg_match($pattern, $f)) { continue; }
            $files[] = rtrim($path, '/') . '/' . $f;
        }
        return $this->__addList($files, $options);
    }
    private function __addList(array $files, array $options): array {
        $added = [];
        foreach ($files as $f) {
            if (is_dir($f)) { continue; }
            $entry = $f;
            if (isset($options['remove_path']) && str_starts_with($entry, $options['remove_path'])) {
                $entry = ltrim(substr($entry, strlen($options['remove_path'])), '/');
            }
            if (!empty($options['remove_all_path'])) { $entry = basename($entry); }
            if (isset($options['add_path'])) { $entry = $options['add_path'] . $entry; }
            if ($this->addFile($f, $entry)) { $added[] = $f; }
        }
        return $added;
    }
    public function renameIndex(int $index, string $new_name): bool {
        $this->__chk();
        if ($new_name === '') { throw new ValueError('ZipArchive::renameIndex(): Argument #2 ($new_name) must not be empty'); }
        $r = __zip('rename', $this->__h(), $index, $new_name); $this->__sync(); return $r;
    }
    public function renameName(string $name, string $new_name): bool {
        $this->__chk();
        if ($new_name === '') { throw new ValueError('ZipArchive::renameName(): Argument #2 ($new_name) must not be empty'); }
        $r = __zip('rename', $this->__h(), $this->__idx($name), $new_name); $this->__sync(); return $r;
    }
    public function setArchiveComment(string $comment): bool { $this->__chk(); $r = __zip('set_archive_comment', $this->__h(), $comment); $this->__sync(); return $r; }
    public function getArchiveComment(int $flags = 0): string|false { $this->__chk(); return __zip('get_archive_comment', $this->__h()); }
    public function setArchiveFlag(int $flag, int $value): bool { $this->__chk(); return true; }
    public function getArchiveFlag(int $flag, int $flags = 0): int { $this->__chk(); return 0; }
    public function setCommentIndex(int $index, string $comment): bool { $this->__chk(); return __zip('set_comment', $this->__h(), $index, $comment); }
    public function setCommentName(string $name, string $comment): bool { $this->__chk(); return __zip('set_comment', $this->__h(), $this->__idx($name), $comment); }
    public function setMtimeIndex(int $index, int $timestamp, int $flags = 0): bool { $this->__chk(); return __zip('set_mtime', $this->__h(), $index, $timestamp); }
    public function setMtimeName(string $name, int $timestamp, int $flags = 0): bool { $this->__chk(); return __zip('set_mtime', $this->__h(), $this->__idx($name), $timestamp); }
    public function getCommentIndex(int $index, int $flags = 0): string|false { $this->__chk(); return __zip('get_comment', $this->__h(), $index); }
    public function getCommentName(string $name, int $flags = 0): string|false { $this->__chk(); return __zip('get_comment', $this->__h(), $this->__idx($name)); }
    public function deleteIndex(int $index): bool { $this->__chk(); $r = __zip('delete', $this->__h(), $index); $this->__sync(); return $r; }
    public function deleteName(string $name): bool { $this->__chk(); $r = __zip('delete', $this->__h(), $this->__idx($name)); $this->__sync(); return $r; }
    public function statName(string $name, int $flags = 0): array|false { $this->__chk(); return __zip('stat', $this->__h(), $this->__idx($name, $flags)); }
    public function statIndex(int $index, int $flags = 0): array|false { $this->__chk(); return __zip('stat', $this->__h(), $index); }
    public function locateName(string $name, int $flags = 0): int|false { $this->__chk(); return __zip('locate', $this->__h(), $name, $flags); }
    public function getNameIndex(int $index, int $flags = 0): string|false { $this->__chk(); return __zip('name', $this->__h(), $index); }
    public function unchangeArchive(): bool { $this->__chk(); $r = __zip('unchange_archive', $this->__h()); $this->__sync(); return $r; }
    public function unchangeAll(): bool { $this->__chk(); $r = __zip('unchange_all', $this->__h()); $this->__sync(); return $r; }
    public function unchangeIndex(int $index): bool { $this->__chk(); $r = __zip('unchange', $this->__h(), $index); $this->__sync(); return $r; }
    public function unchangeName(string $name): bool { $this->__chk(); $r = __zip('unchange', $this->__h(), $this->__idx($name)); $this->__sync(); return $r; }
    public function extractTo(string $pathto, array|string|null $files = null): bool { $this->__chk(); return __zip('extract', $this->__h(), $pathto, $files); }
    public function getFromName(string $name, int $len = 0, int $flags = 0): string|false {
        $this->__chk();
        if ($len < 0) { throw new ValueError('ZipArchive::getFromName(): Argument #2 ($len) must be greater than or equal to 0'); }
        $i = $this->__idx($name, $flags);
        $r = $i < 0 ? false : __zip('get', $this->__h(), $i, $len);
        $this->__sync();
        return $r;
    }
    public function getFromIndex(int $index, int $len = 0, int $flags = 0): string|false {
        $this->__chk();
        if ($len < 0) { throw new ValueError('ZipArchive::getFromIndex(): Argument #2 ($len) must be greater than or equal to 0'); }
        $r = __zip('get', $this->__h(), $index, $len);
        $this->__sync();
        return $r;
    }
    private function __stream($data) {
        if ($data === false) { return false; }
        $s = fopen('php://memory', 'w+b');
        fwrite($s, $data);
        rewind($s);
        return $s;
    }
    public function getStreamIndex(int $index, int $flags = 0) { $this->__chk(); $r = __zip('get', $this->__h(), $index, 0); $this->__sync(); return $this->__stream($r); }
    public function getStreamName(string $name, int $flags = 0) { $this->__chk(); return $this->getStreamIndex($this->__idx($name, $flags)); }
    public function getStream(string $name) { $this->__chk(); return $this->getStreamIndex($this->__idx($name)); }
    public function setExternalAttributesName(string $name, int $opsys, int $attr, int $flags = 0): bool { $this->__chk(); return __zip('set_attr', $this->__h(), $this->__idx($name), $opsys, $attr); }
    public function setExternalAttributesIndex(int $index, int $opsys, int $attr, int $flags = 0): bool { $this->__chk(); return __zip('set_attr', $this->__h(), $index, $opsys, $attr); }
    public function getExternalAttributesName(string $name, &$opsys, &$attr, int $flags = 0): bool {
        $this->__chk();
        $r = __zip('get_attr', $this->__h(), $this->__idx($name));
        if ($r === false) { return false; }
        [$opsys, $attr] = $r;
        return true;
    }
    public function getExternalAttributesIndex(int $index, &$opsys, &$attr, int $flags = 0): bool {
        $this->__chk();
        $r = __zip('get_attr', $this->__h(), $index);
        if ($r === false) { return false; }
        [$opsys, $attr] = $r;
        return true;
    }
    public function setCompressionName(string $name, int $method, int $compflags = 0): bool { $this->__chk(); return __zip('set_compression', $this->__h(), $this->__idx($name), $method); }
    public function setCompressionIndex(int $index, int $method, int $compflags = 0): bool { $this->__chk(); return __zip('set_compression', $this->__h(), $index, $method); }
    public function setEncryptionName(string $name, int $method, ?string $password = null): bool { $this->__chk(); return $method === 0; }
    public function setEncryptionIndex(int $index, int $method, ?string $password = null): bool { $this->__chk(); return $method === 0; }
    public function registerProgressCallback(float $rate, callable $callback): bool { $this->__chk(); return true; }
    public function registerCancelCallback(callable $callback): bool { $this->__chk(); return true; }
    public static function isCompressionMethodSupported(int $method, bool $enc = true): bool { return in_array($method, [-1, 0, 8], true); }
    public static function isEncryptionMethodSupported(int $method, bool $enc = true): bool { return $method === 0; }
}
"#
);
