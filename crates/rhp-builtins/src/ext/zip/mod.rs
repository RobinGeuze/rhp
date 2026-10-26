//! The `zip` extension: a read/extract subset of `ZipArchive` — enough for
//! Composer's dist downloader (`open`/`count`/`numFiles`/`getNameIndex`/
//! `statIndex`/`getFromName`/`extractTo`/`close`). Backed by `zip_ext::ZipState`
//! on the VM via `__zip_*` builtins. Writing archives is not implemented.

use std::collections::HashMap;

use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn fn_zip_open(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = conv::to_string_bytes(&deref(&args[0]));
    Ok(Value::Int(host.zip_open(&path)))
}

fn fn_zip_count(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let id = conv::to_int(&deref(&args[0]));
    Ok(Value::Int(host.zip_count(id)))
}

fn fn_zip_name_index(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let id = conv::to_int(&deref(&args[0]));
    let i = conv::to_int(&deref(&args[1]));
    Ok(host.zip_name_index(id, i).map(Value::str).unwrap_or(Value::Bool(false)))
}

fn fn_zip_stat_index(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let id = conv::to_int(&deref(&args[0]));
    let i = conv::to_int(&deref(&args[1]));
    match host.zip_stat_index(id, i) {
        Some((name, size, comp)) => {
            let mut a = PhpArray::new();
            a.insert(ArrayKey::from_bytes(b"name"), Value::str(name));
            a.insert(ArrayKey::from_bytes(b"index"), Value::Int(i));
            a.insert(ArrayKey::from_bytes(b"crc"), Value::Int(0));
            a.insert(ArrayKey::from_bytes(b"size"), Value::Int(size as i64));
            a.insert(ArrayKey::from_bytes(b"mtime"), Value::Int(0));
            a.insert(ArrayKey::from_bytes(b"comp_size"), Value::Int(comp as i64));
            a.insert(ArrayKey::from_bytes(b"comp_method"), Value::Int(if comp == size { 0 } else { 8 }));
            Ok(Value::Array(a))
        }
        None => Ok(Value::Bool(false)),
    }
}

fn fn_zip_get_from_name(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let id = conv::to_int(&deref(&args[0]));
    let name = conv::to_string_bytes(&deref(&args[1]));
    Ok(host.zip_get_from_name(id, &name).map(Value::str).unwrap_or(Value::Bool(false)))
}

fn fn_zip_extract_to(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let id = conv::to_int(&deref(&args[0]));
    let dest = conv::to_string_bytes(&deref(&args[1]));
    Ok(Value::Bool(host.zip_extract_to(id, &dest)))
}

fn fn_zip_close(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let id = conv::to_int(&deref(&args[0]));
    host.zip_close(id);
    Ok(Value::Bool(true))
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(m: &mut HashMap<&'static str, Builtin>, name: &'static str, func: crate::NativeFn, min: usize, max: Option<usize>) {
        m.insert(name, Builtin { name, func, min_args: min, max_args: max });
    }
    put(m, "__zip_open", fn_zip_open, 1, Some(1));
    put(m, "__zip_count", fn_zip_count, 1, Some(1));
    put(m, "__zip_name_index", fn_zip_name_index, 2, Some(2));
    put(m, "__zip_stat_index", fn_zip_stat_index, 2, Some(2));
    put(m, "__zip_get_from_name", fn_zip_get_from_name, 2, Some(2));
    put(m, "__zip_extract_to", fn_zip_extract_to, 2, Some(2));
    put(m, "__zip_close", fn_zip_close, 1, Some(1));
}

pub const PRELUDE_PHP: &str = r#"
class ZipArchive {
    public $numFiles = 0;
    public $status = 0;
    public $statusSys = 0;
    public $filename = '';
    private $__id = -1;

    const CREATE = 1; const EXCL = 2; const CHECKCONS = 4; const OVERWRITE = 8;
    const FL_NOCASE = 1; const FL_NODIR = 2; const FL_COMPRESSED = 4; const FL_UNCHANGED = 8;
    const FL_OVERWRITE = 8192;
    const CM_DEFAULT = -1; const CM_STORE = 0; const CM_DEFLATE = 8;
    const ER_OK = 0; const ER_MULTIDISK = 1; const ER_RENAME = 2; const ER_CLOSE = 3;
    const ER_SEEK = 4; const ER_READ = 5; const ER_WRITE = 6; const ER_CRC = 7;
    const ER_ZIPCLOSED = 8; const ER_NOENT = 9; const ER_EXISTS = 10; const ER_OPEN = 11;
    const ER_TMPOPEN = 12; const ER_ZLIB = 13; const ER_MEMORY = 14; const ER_CHANGED = 15;
    const ER_COMPNOTSUPP = 16; const ER_EOF = 17; const ER_INVAL = 18; const ER_NOZIP = 19;
    const ER_INTERNAL = 20; const ER_INCONS = 21; const ER_REMOVE = 22; const ER_DELETED = 23;

    public function open($filename, $flags = 0) {
        $this->filename = $filename;
        $id = __zip_open($filename);
        if ($id < 0) { $this->status = -$id; return -$id; }
        $this->__id = $id;
        $this->numFiles = __zip_count($id);
        return true;
    }
    public function count(): int { return $this->__id < 0 ? 0 : __zip_count($this->__id); }
    public function getNameIndex($index, $flags = 0) { return __zip_name_index($this->__id, $index); }
    public function statIndex($index, $flags = 0) { return __zip_stat_index($this->__id, $index); }
    public function getFromName($name, $len = 0, $flags = 0) { return __zip_get_from_name($this->__id, $name); }
    public function extractTo($pathto, $files = null) { return __zip_extract_to($this->__id, $pathto); }
    public function close() { if ($this->__id >= 0) { __zip_close($this->__id); $this->__id = -1; } return true; }
    public function getStatusString() { return 'No error'; }
}
"#;
