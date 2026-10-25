//! A slice of the `SPL` extension: the autoload registry
//! (`spl_autoload_register` and friends). The callables live on the [`Host`]
//! (the VM), which invokes them when a class reference fails to resolve.

use std::collections::HashMap;

use rhp_values::{conv, PhpArray, Value};

use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

/// `spl_autoload_register(?callable $callback = null, bool $throw = true, bool $prepend = false): bool`
fn fn_spl_autoload_register(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match args.first().map(deref) {
        // The default SPL autoloader is not supported; registering it is a no-op.
        None | Some(Value::Null) => Ok(Value::Bool(true)),
        Some(cb) => {
            if !host.is_callable_value(&cb) {
                return Err(Thrown::error(
                    "TypeError",
                    "spl_autoload_register(): Argument #1 ($callback) must be a valid callback",
                ));
            }
            let prepend = args.get(2).map(|v| conv::to_bool(&deref(v))).unwrap_or(false);
            host.register_autoloader(cb, prepend);
            Ok(Value::Bool(true))
        }
    }
}

/// `spl_autoload_unregister(callable $callback): bool` — not yet supported
/// (callable identity comparison); reports success without removing.
fn fn_spl_autoload_unregister(_host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
}

/// `spl_autoload_functions(): array`
fn fn_spl_autoload_functions(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    for cb in host.autoload_functions() {
        a.push(cb);
    }
    Ok(Value::Array(a))
}

/// `spl_autoload_call(string $class): void`
fn fn_spl_autoload_call(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = conv::to_string_bytes(&deref(&args[0]));
    host.autoload_call(&name)?;
    Ok(Value::Null)
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(name, Builtin { name, func, min_args: min, max_args: max });
    }
    put(m, "spl_autoload_register", fn_spl_autoload_register, 0, Some(3));
    put(m, "spl_autoload_unregister", fn_spl_autoload_unregister, 1, Some(1));
    put(m, "spl_autoload_functions", fn_spl_autoload_functions, 0, Some(0));
    put(m, "spl_autoload_call", fn_spl_autoload_call, 1, Some(1));
}

/// SPL / core data-structure classes implemented in PHP (ArrayIterator and
/// friends), backed by a plain array.
pub const PRELUDE_PHP: &str = r#"
class ArrayIterator implements Iterator, ArrayAccess, Countable {
    private $__s; private $__k; private $__p = 0;
    public function __construct($array = [], $flags = 0) { $this->__s = $array; $this->__k = array_keys($array); }
    public function current(): mixed { return $this->__s[$this->__k[$this->__p]] ?? null; }
    public function key(): mixed { return $this->__k[$this->__p] ?? null; }
    public function next(): void { $this->__p++; }
    public function rewind(): void { $this->__k = array_keys($this->__s); $this->__p = 0; }
    public function valid(): bool { return $this->__p < count($this->__k); }
    public function count(): int { return count($this->__s); }
    public function offsetExists($k): bool { return isset($this->__s[$k]); }
    public function offsetGet($k): mixed { return $this->__s[$k] ?? null; }
    public function offsetSet($k, $v): void { if ($k === null) { $this->__s[] = $v; } else { $this->__s[$k] = $v; } }
    public function offsetUnset($k): void { unset($this->__s[$k]); }
    public function getArrayCopy() { return $this->__s; }
    public function append($v) { $this->__s[] = $v; }
}
class ArrayObject implements IteratorAggregate, ArrayAccess, Countable {
    private $__s;
    public function __construct($array = [], $flags = 0) { $this->__s = $array; }
    public function getIterator(): Iterator { return new ArrayIterator($this->__s); }
    public function count(): int { return count($this->__s); }
    public function offsetExists($k): bool { return isset($this->__s[$k]); }
    public function offsetGet($k): mixed { return $this->__s[$k] ?? null; }
    public function offsetSet($k, $v): void { if ($k === null) { $this->__s[] = $v; } else { $this->__s[$k] = $v; } }
    public function offsetUnset($k): void { unset($this->__s[$k]); }
    public function getArrayCopy() { return $this->__s; }
    public function append($v) { $this->__s[] = $v; }
}
"#;
