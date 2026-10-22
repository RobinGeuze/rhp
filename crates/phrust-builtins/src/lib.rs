//! Native function + constant registry, and the narrow `Host` interface the
//! interpreter exposes to builtins.
//!
//! Builtins operate on owned `Value`s only — they never touch the arena/AST,
//! so this crate depends on `phrust-values` alone (no cycle through the tiers).
//! The interpreter talks back through [`Host`].

use std::collections::HashMap;
use std::sync::OnceLock;

use phrust_values::conv;
use phrust_values::{PhpArray, Value};

pub use phrust_values::Thrown;

pub mod dump;

mod arr_fns;
mod datetime_fns;
mod filter_fns;
mod hash_fns;
mod math_fns;
mod mb_fns;
mod misc_fns;
mod serialize_fns;
mod sort_fns;
mod str_fns;
mod strarr2_fns;
mod strcmp_fns;
mod webenc_fns;

/// The interpreter surface a builtin may use. Implemented by the VM and the oracle.
pub trait Host {
    /// Append bytes to the single output sink (stdout / output buffer).
    fn write_out(&mut self, bytes: &[u8]);

    /// A class's name by id (for `get_class` etc.). Default: not available.
    fn class_name(&self, _class_id: u32) -> Option<Vec<u8>> {
        None
    }

    /// A class's parent's name by id (for `get_parent_class`). Default: none.
    fn class_parent_name(&self, _class_id: u32) -> Option<Vec<u8>> {
        None
    }

    /// Invoke a PHP callable (for `array_map`/`array_filter`/…). Default: refused.
    fn call_callable(&mut self, _callable: &Value, _args: &[Value]) -> Result<Value, Thrown> {
        Err(Thrown::unsupported("callback", "callbacks are not available in this host"))
    }

    /// Whether a value is callable (`is_callable`): closures, function names,
    /// `[obj_or_class, method]` pairs, and `__invoke` objects. Default: closures only.
    fn is_callable_value(&self, callable: &Value) -> bool {
        matches!(deref(callable), Value::Closure(_))
    }
}

/// A native builtin function over owned values.
pub type NativeFn = fn(&mut dyn Host, &[Value]) -> Result<Value, Thrown>;

/// Registry entry: a native function plus its arity bounds.
pub struct Builtin {
    pub name: &'static str,
    pub func: NativeFn,
    pub min_args: usize,
    /// `None` = variadic (no upper bound).
    pub max_args: Option<usize>,
}

macro_rules! builtins {
    ($($name:literal => $func:path, $min:literal, $max:expr);* $(;)?) => {{
        let mut m: HashMap<&'static str, Builtin> = HashMap::new();
        $(
            m.insert($name, Builtin { name: $name, func: $func, min_args: $min, max_args: $max });
        )*
        m
    }};
}

/// The builtin function table, keyed by lowercase name (PHP function names are
/// case-insensitive).
pub fn registry() -> &'static HashMap<&'static str, Builtin> {
    static REG: OnceLock<HashMap<&'static str, Builtin>> = OnceLock::new();
    REG.get_or_init(|| {
        let mut m = builtins! {
            "var_dump"       => fn_var_dump, 1, None;
            "print_r"        => fn_print_r, 1, Some(2);
            "var_export"     => fn_var_export, 1, Some(2);
            "gettype"        => fn_gettype, 1, Some(1);
            "get_debug_type" => fn_get_debug_type, 1, Some(1);
            "is_numeric"     => fn_is_numeric, 1, Some(1);
            "is_int"         => fn_is_int, 1, Some(1);
            "is_integer"     => fn_is_int, 1, Some(1);
            "is_long"        => fn_is_int, 1, Some(1);
            "is_float"       => fn_is_float, 1, Some(1);
            "is_double"      => fn_is_float, 1, Some(1);
            "is_string"      => fn_is_string, 1, Some(1);
            "is_bool"        => fn_is_bool, 1, Some(1);
            "is_null"        => fn_is_null, 1, Some(1);
            "is_array"       => fn_is_array, 1, Some(1);
            "is_scalar"      => fn_is_scalar, 1, Some(1);
            "is_object"      => fn_is_object, 1, Some(1);
            "is_callable"    => fn_is_callable, 1, Some(1);
            "intval"         => fn_intval, 1, Some(2);
            "floatval"       => fn_floatval, 1, Some(1);
            "doubleval"      => fn_floatval, 1, Some(1);
            "strval"         => fn_strval, 1, Some(1);
            "boolval"        => fn_boolval, 1, Some(1);
            "strlen"         => fn_strlen, 1, Some(1);
            "count"          => fn_count, 1, Some(2);
            "sizeof"         => fn_count, 1, Some(2);
            "intdiv"         => fn_intdiv, 2, Some(2);
            "str_repeat"     => fn_str_repeat, 2, Some(2);
            "abs"            => fn_abs, 1, Some(1);
            "max"            => fn_max, 1, None;
            "min"            => fn_min, 1, None;
            "array_sum"      => fn_array_sum, 1, Some(1);
            "array_product"  => fn_array_product, 1, Some(1);
            "array_keys"     => fn_array_keys, 1, Some(1);
            "array_values"   => fn_array_values, 1, Some(1);
            "array_merge"    => fn_array_merge, 0, None;
            "array_reverse"  => fn_array_reverse, 1, Some(2);
            "range"          => fn_range, 2, Some(3);
            "array_map"      => fn_array_map, 2, None;
            "array_filter"   => fn_array_filter, 1, Some(3);
            "array_reduce"   => fn_array_reduce, 2, Some(3);
            "call_user_func" => fn_call_user_func, 1, None;
            "call_user_func_array" => fn_call_user_func_array, 2, Some(2);
            "in_array"       => fn_in_array, 2, Some(3);
            "array_key_exists" => fn_array_key_exists, 2, Some(2);
            "implode"        => fn_implode, 1, Some(2);
            "join"           => fn_implode, 1, Some(2);
            "explode"        => fn_explode, 2, Some(3);
            "strtoupper"     => fn_strtoupper, 1, Some(1);
            "strtolower"     => fn_strtolower, 1, Some(1);
            "str_contains"   => fn_str_contains, 2, Some(2);
            "ord"            => fn_ord, 1, Some(1);
            "chr"            => fn_chr, 1, Some(1);
            "get_class"      => fn_get_class, 1, Some(1);
            "get_parent_class" => fn_get_parent_class, 0, Some(1);
            "get_object_vars" => fn_get_object_vars, 1, Some(1);
        };
        // Builtin modules produced by the stdlib workflow (each verified vs php).
        str_fns::register(&mut m);
        math_fns::register(&mut m);
        arr_fns::register(&mut m);
        misc_fns::register(&mut m);
        hash_fns::register(&mut m);
        webenc_fns::register(&mut m);
        datetime_fns::register(&mut m);
        strarr2_fns::register(&mut m);
        sort_fns::register(&mut m);
        serialize_fns::register(&mut m);
        filter_fns::register(&mut m);
        strcmp_fns::register(&mut m);
        mb_fns::register(&mut m);
        m
    })
}

/// Case-insensitive builtin lookup.
pub fn lookup(name: &str) -> Option<&'static Builtin> {
    let lower = name.to_ascii_lowercase();
    registry().get(lower.as_str())
}

/// Engine constants (`PHP_EOL`, `PHP_INT_MAX`, …).
pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("PHP_EOL", Value::str("\n")),
        ("PHP_INT_MAX", Value::Int(i64::MAX)),
        ("PHP_INT_MIN", Value::Int(i64::MIN)),
        ("PHP_INT_SIZE", Value::Int(8)),
        ("PHP_FLOAT_EPSILON", Value::Float(f64::EPSILON)),
        ("PHP_FLOAT_MAX", Value::Float(f64::MAX)),
        ("PHP_FLOAT_MIN", Value::Float(f64::MIN_POSITIVE)),
        ("PHP_VERSION", Value::str("8.5.8")),
        ("PHP_MAJOR_VERSION", Value::Int(8)),
        ("PHP_MINOR_VERSION", Value::Int(5)),
        ("PHP_OS", Value::str("Darwin")),
        ("PHP_OS_FAMILY", Value::str("Darwin")),
        ("NAN", Value::Float(f64::NAN)),
        ("INF", Value::Float(f64::INFINITY)),
        ("E_ALL", Value::Int(32767)),
        ("E_WARNING", Value::Int(2)),
        ("E_NOTICE", Value::Int(8)),
        ("COUNT_NORMAL", Value::Int(0)),
        ("COUNT_RECURSIVE", Value::Int(1)),
        ("STR_PAD_RIGHT", Value::Int(1)),
        ("STR_PAD_LEFT", Value::Int(0)),
        ("STR_PAD_BOTH", Value::Int(2)),
        ("SORT_REGULAR", Value::Int(0)),
        ("SORT_NUMERIC", Value::Int(1)),
        ("SORT_STRING", Value::Int(2)),
        ("SORT_DESC", Value::Int(3)),
        ("SORT_ASC", Value::Int(4)),
        ("SORT_LOCALE_STRING", Value::Int(5)),
        ("SORT_NATURAL", Value::Int(6)),
        ("SORT_FLAG_CASE", Value::Int(8)),
        ("MB_CASE_UPPER", Value::Int(0)),
        ("MB_CASE_LOWER", Value::Int(1)),
        ("MB_CASE_TITLE", Value::Int(2)),
        ("MB_CASE_FOLD", Value::Int(3)),
        ("MB_CASE_UPPER_SIMPLE", Value::Int(4)),
        ("MB_CASE_LOWER_SIMPLE", Value::Int(5)),
        ("MB_CASE_TITLE_SIMPLE", Value::Int(6)),
        ("MB_CASE_FOLD_SIMPLE", Value::Int(7)),
        ("FILTER_DEFAULT", Value::Int(516)),
        ("FILTER_UNSAFE_RAW", Value::Int(516)),
        ("FILTER_VALIDATE_INT", Value::Int(257)),
        ("FILTER_VALIDATE_BOOLEAN", Value::Int(258)),
        ("FILTER_VALIDATE_BOOL", Value::Int(258)),
        ("FILTER_VALIDATE_FLOAT", Value::Int(259)),
        ("FILTER_VALIDATE_REGEXP", Value::Int(272)),
        ("FILTER_VALIDATE_DOMAIN", Value::Int(277)),
        ("FILTER_VALIDATE_URL", Value::Int(273)),
        ("FILTER_VALIDATE_EMAIL", Value::Int(274)),
        ("FILTER_VALIDATE_IP", Value::Int(275)),
        ("FILTER_VALIDATE_MAC", Value::Int(276)),
        ("FILTER_SANITIZE_ENCODED", Value::Int(514)),
        ("FILTER_SANITIZE_SPECIAL_CHARS", Value::Int(515)),
        ("FILTER_SANITIZE_EMAIL", Value::Int(517)),
        ("FILTER_SANITIZE_URL", Value::Int(518)),
        ("FILTER_SANITIZE_NUMBER_INT", Value::Int(519)),
        ("FILTER_SANITIZE_NUMBER_FLOAT", Value::Int(520)),
        ("FILTER_SANITIZE_FULL_SPECIAL_CHARS", Value::Int(522)),
        ("FILTER_SANITIZE_ADD_SLASHES", Value::Int(523)),
        ("FILTER_FLAG_NONE", Value::Int(0)),
        ("FILTER_NULL_ON_FAILURE", Value::Int(134217728)),
        ("FILTER_FLAG_ALLOW_OCTAL", Value::Int(1)),
        ("FILTER_FLAG_ALLOW_HEX", Value::Int(2)),
        ("FILTER_FLAG_ALLOW_THOUSAND", Value::Int(8192)),
        ("FILTER_FLAG_ALLOW_SCIENTIFIC", Value::Int(16384)),
        ("FILTER_FLAG_IPV4", Value::Int(1048576)),
        ("FILTER_FLAG_IPV6", Value::Int(2097152)),
        ("FILTER_FLAG_NO_PRIV_RANGE", Value::Int(8388608)),
        ("FILTER_FLAG_NO_RES_RANGE", Value::Int(4194304)),
        ("FILTER_FLAG_PATH_REQUIRED", Value::Int(262144)),
        ("FILTER_FLAG_QUERY_REQUIRED", Value::Int(524288)),
        ("FILTER_FLAG_HOSTNAME", Value::Int(1048576)),
        ("FILTER_FLAG_EMAIL_UNICODE", Value::Int(1048576)),
        ("ENT_QUOTES", Value::Int(3)),
        ("ENT_COMPAT", Value::Int(2)),
        ("ENT_NOQUOTES", Value::Int(0)),
        ("ENT_HTML401", Value::Int(0)),
        ("ENT_HTML5", Value::Int(48)),
        ("PHP_ROUND_HALF_UP", Value::Int(1)),
        ("PHP_ROUND_HALF_DOWN", Value::Int(2)),
        ("ARRAY_FILTER_USE_KEY", Value::Int(2)),
        ("ARRAY_FILTER_USE_BOTH", Value::Int(1)),
        ("M_PI", Value::Float(std::f64::consts::PI)),
        ("M_E", Value::Float(std::f64::consts::E)),
        ("M_SQRT2", Value::Float(std::f64::consts::SQRT_2)),
        ("PHP_FLOAT_DIG", Value::Int(15)),
    ]
}

/// If `name` is a deliberately-refused high-risk function, return why.
pub fn is_refused(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "extract" => Some("writes arbitrary variables into the local scope"),
        "compact" => Some("resolves variable names dynamically from strings"),
        "create_function" => Some("compiles a string into a function (eval-equivalent)"),
        "assert" => Some("string assertions are eval-equivalent"),
        "exec" | "system" | "passthru" | "shell_exec" | "proc_open" | "popen" => {
            Some("executes external shell commands")
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// builtin implementations
// ---------------------------------------------------------------------------

use phrust_semantics::{deref, ops};

fn fn_var_dump(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = Vec::new();
    for a in args {
        dump::var_dump(&mut out, a, 0);
    }
    host.write_out(&out);
    Ok(Value::Null)
}

fn fn_print_r(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = Vec::new();
    dump::print_r(&mut out, &args[0], 0);
    let return_mode = args.get(1).map(conv::to_bool).unwrap_or(false);
    if return_mode {
        Ok(Value::str(out))
    } else {
        host.write_out(&out);
        Ok(Value::Bool(true))
    }
}

fn fn_var_export(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = Vec::new();
    dump::var_export(&mut out, &args[0], 0);
    let return_mode = args.get(1).map(conv::to_bool).unwrap_or(false);
    if return_mode {
        Ok(Value::str(out))
    } else {
        host.write_out(&out);
        Ok(Value::Null)
    }
}

fn fn_gettype(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(deref(&args[0]).type_name()))
}

fn fn_get_debug_type(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = match deref(&args[0]) {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        Value::Closure(_) => "Closure",
        Value::Object(_) => "object",
        Value::Ref(_) => unreachable!(),
    };
    Ok(Value::str(name))
}

fn fn_is_numeric(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(conv::is_numeric(&args[0])))
}

macro_rules! is_type {
    ($name:ident, $pat:pat) => {
        fn $name(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            Ok(Value::Bool(matches!(deref(&args[0]), $pat)))
        }
    };
}
is_type!(fn_is_int, Value::Int(_));
is_type!(fn_is_float, Value::Float(_));
is_type!(fn_is_string, Value::Str(_));
is_type!(fn_is_bool, Value::Bool(_));
is_type!(fn_is_null, Value::Null);
is_type!(fn_is_array, Value::Array(_));
is_type!(fn_is_object, Value::Object(_) | Value::Closure(_));

fn fn_is_callable(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.is_callable_value(&args[0])))
}
is_type!(fn_is_scalar, Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Bool(_));

fn fn_intval(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // base only applies to string args; default 10 is the common path.
    Ok(Value::Int(conv::to_int(&args[0])))
}

fn fn_floatval(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(conv::to_float(&args[0])))
}

fn fn_strval(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(conv::to_string_bytes(&args[0])))
}

fn fn_boolval(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(conv::to_bool(&args[0])))
}

fn fn_strlen(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(conv::to_string_bytes(&args[0]).len() as i64))
}

fn fn_count(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match deref(&args[0]) {
        Value::Array(a) => Ok(Value::Int(a.len() as i64)),
        other => Err(Thrown::type_error(format!(
            "count(): Argument #1 ($value) must be of type Countable|array, {} given",
            other.type_name_lower()
        ))),
    }
}

fn fn_intdiv(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = conv::to_int(&args[0]);
    let b = conv::to_int(&args[1]);
    if b == 0 {
        return Err(Thrown::error("DivisionByZeroError", "Division by zero"));
    }
    if a == i64::MIN && b == -1 {
        return Err(Thrown::error(
            "ArithmeticError",
            "Division of PHP_INT_MIN by -1 is not an integer",
        ));
    }
    Ok(Value::Int(a / b))
}

fn fn_str_repeat(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let n = conv::to_int(&args[1]).max(0) as usize;
    Ok(Value::str(s.repeat(n)))
}

fn fn_abs(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match conv::to_number(&deref(&args[0])).0 {
        conv::Num::Int(n) => Ok(match n.checked_abs() {
            Some(v) => Value::Int(v),
            None => Value::Float((n as f64).abs()),
        }),
        conv::Num::Float(f) => Ok(Value::Float(f.abs())),
    }
}

fn fn_max(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    extremum(args, true)
}
fn fn_min(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    extremum(args, false)
}

fn extremum(args: &[Value], want_max: bool) -> Result<Value, Thrown> {
    // max(array) / max(a, b, ...). Comparison uses loose ordering via conv.
    let items: Vec<Value> = if args.len() == 1 {
        match deref(&args[0]) {
            Value::Array(a) => a.iter().map(|(_, v)| v.clone()).collect(),
            other => vec![other],
        }
    } else {
        args.to_vec()
    };
    if items.is_empty() {
        return Err(Thrown::type_error("max(): Argument #1 ($value) must contain at least one element"));
    }
    let mut best = items[0].clone();
    for v in &items[1..] {
        // PHP max/min use the standard loose comparison.
        let gt = ops::loose_compare(v, &best).is_gt();
        if gt == want_max {
            best = v.clone();
        }
    }
    Ok(best)
}

fn fn_array_sum(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[0]) else {
        return Err(Thrown::type_error("array_sum(): Argument #1 ($array) must be of type array"));
    };
    let mut acc = Value::Int(0);
    for (_, v) in a.iter() {
        acc = ops::add(&acc, v)?;
    }
    Ok(acc)
}

fn fn_array_product(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[0]) else {
        return Err(Thrown::type_error("array_product(): Argument #1 ($array) must be of type array"));
    };
    let mut acc = Value::Int(1);
    for (_, v) in a.iter() {
        acc = ops::mul(&acc, v)?;
    }
    Ok(acc)
}

fn fn_array_keys(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[0]) else {
        return Err(Thrown::type_error("array_keys(): Argument #1 ($array) must be of type array"));
    };
    let mut out = PhpArray::new();
    for (k, _) in a.iter() {
        out.push(match k {
            phrust_values::ArrayKey::Int(n) => Value::Int(*n),
            phrust_values::ArrayKey::Str(s) => Value::Str(s.clone()),
        });
    }
    Ok(Value::Array(out))
}

fn fn_array_values(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[0]) else {
        return Err(Thrown::type_error("array_values(): Argument #1 ($array) must be of type array"));
    };
    let mut out = PhpArray::new();
    for (_, v) in a.iter() {
        out.push(v.clone());
    }
    Ok(Value::Array(out))
}

fn fn_array_merge(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = PhpArray::new();
    for arg in args {
        let Value::Array(a) = deref(arg) else {
            return Err(Thrown::type_error("array_merge(): Arguments must be of type array"));
        };
        for (k, v) in a.iter() {
            match k {
                // integer keys are renumbered; string keys overwrite
                phrust_values::ArrayKey::Int(_) => out.push(v.clone()),
                phrust_values::ArrayKey::Str(_) => out.insert(k.clone(), v.clone()),
            }
        }
    }
    Ok(Value::Array(out))
}

fn key_to_value(k: &phrust_values::ArrayKey) -> Value {
    match k {
        phrust_values::ArrayKey::Int(n) => Value::Int(*n),
        phrust_values::ArrayKey::Str(s) => Value::Str(s.clone()),
    }
}

fn as_array(v: &Value, who: &str) -> Result<PhpArray, Thrown> {
    match deref(v) {
        Value::Array(a) => Ok(a),
        _ => Err(Thrown::type_error(format!("{who}(): argument must be of type array"))),
    }
}

fn fn_array_map(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cb = &args[0];
    let cb_null = matches!(deref(cb), Value::Null);
    let arrays: Vec<PhpArray> =
        args[1..].iter().map(|a| as_array(a, "array_map")).collect::<Result<_, _>>()?;
    if arrays.len() == 1 {
        // single array: preserve keys
        let mut out = PhpArray::new();
        for (k, v) in arrays[0].iter() {
            let r = if cb_null { v.clone() } else { host.call_callable(cb, &[v.clone()])? };
            out.insert(k.clone(), r);
        }
        return Ok(Value::Array(out));
    }
    // multiple arrays: zip by position, reindex
    let max_len = arrays.iter().map(|a| a.len()).max().unwrap_or(0);
    let mut out = PhpArray::new();
    for i in 0..max_len {
        let tuple: Vec<Value> =
            arrays.iter().map(|a| a.get_index(i).map(|(_, v)| v.clone()).unwrap_or(Value::Null)).collect();
        if cb_null {
            let mut t = PhpArray::new();
            for v in tuple {
                t.push(v);
            }
            out.push(Value::Array(t));
        } else {
            out.push(host.call_callable(cb, &tuple)?);
        }
    }
    Ok(Value::Array(out))
}

fn fn_array_filter(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = as_array(&args[0], "array_filter")?;
    let cb = args.get(1).filter(|c| !matches!(deref(c), Value::Null));
    let mode = args.get(2).map(conv::to_int).unwrap_or(0);
    let mut out = PhpArray::new();
    for (k, v) in a.iter() {
        let keep = match cb {
            None => conv::to_bool(v),
            Some(cb) => {
                let call_args: Vec<Value> = match mode {
                    2 => vec![key_to_value(k)],                       // ARRAY_FILTER_USE_KEY
                    1 => vec![v.clone(), key_to_value(k)],            // ARRAY_FILTER_USE_BOTH
                    _ => vec![v.clone()],
                };
                conv::to_bool(&host.call_callable(cb, &call_args)?)
            }
        };
        if keep {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

fn fn_array_reduce(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = as_array(&args[0], "array_reduce")?;
    let cb = &args[1];
    let mut acc = args.get(2).map(deref).unwrap_or(Value::Null);
    for (_, v) in a.iter() {
        acc = host.call_callable(cb, &[acc, v.clone()])?;
    }
    Ok(acc)
}

fn fn_call_user_func(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    host.call_callable(&args[0], &args[1..])
}

fn fn_call_user_func_array(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let params = as_array(&args[1], "call_user_func_array")?;
    let argv: Vec<Value> = params.iter().map(|(_, v)| v.clone()).collect();
    host.call_callable(&args[0], &argv)
}

fn fn_array_reverse(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[0]) else {
        return Err(Thrown::type_error("array_reverse(): Argument #1 ($array) must be of type array"));
    };
    let preserve = args.get(1).map(conv::to_bool).unwrap_or(false);
    let mut out = PhpArray::new();
    let entries: Vec<_> = a.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    for (k, v) in entries.into_iter().rev() {
        match k {
            phrust_values::ArrayKey::Int(_) if !preserve => out.push(v),
            _ => out.insert(k, v),
        }
    }
    Ok(Value::Array(out))
}

fn fn_range(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // Numeric ranges only (char ranges deferred).
    let start = conv::to_number(&deref(&args[0])).0;
    let end = conv::to_number(&deref(&args[1])).0;
    let step = args.get(2).map(|v| conv::to_number(&deref(v)).0.as_f64().abs()).unwrap_or(1.0);
    let step = if step == 0.0 { 1.0 } else { step };
    let mut out = PhpArray::new();
    let use_float = matches!(start, conv::Num::Float(_))
        || matches!(end, conv::Num::Float(_))
        || step.fract() != 0.0;
    let (s, e) = (start.as_f64(), end.as_f64());
    if s <= e {
        let mut x = s;
        while x <= e + 1e-9 {
            out.push(if use_float { Value::Float(x) } else { Value::Int(x as i64) });
            x += step;
        }
    } else {
        let mut x = s;
        while x >= e - 1e-9 {
            out.push(if use_float { Value::Float(x) } else { Value::Int(x as i64) });
            x -= step;
        }
    }
    Ok(Value::Array(out))
}

fn fn_in_array(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let needle = deref(&args[0]);
    let Value::Array(a) = deref(&args[1]) else {
        return Err(Thrown::type_error("in_array(): Argument #2 ($haystack) must be of type array"));
    };
    let strict = args.get(2).map(conv::to_bool).unwrap_or(false);
    let found = a.iter().any(|(_, v)| {
        if strict {
            ops::strict_eq(&needle, v)
        } else {
            ops::loose_eq(&needle, v)
        }
    });
    Ok(Value::Bool(found))
}

fn fn_array_key_exists(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[1]) else {
        return Err(Thrown::type_error("array_key_exists(): Argument #2 ($array) must be of type array"));
    };
    let Some(key) = phrust_values::ArrayKey::from_value(&args[0]) else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(a.get(&key).is_some()))
}

fn fn_implode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // implode($array) or implode($glue, $array)
    let (glue, arr) = match (deref(&args[0]), args.get(1).map(deref)) {
        (Value::Array(a), _) => (Vec::new(), a),
        (g, Some(Value::Array(a))) => (conv::to_string_bytes(&g), a),
        _ => return Err(Thrown::type_error("implode(): Argument must be of type array")),
    };
    let mut out = Vec::new();
    for (i, (_, v)) in arr.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(&glue);
        }
        out.extend_from_slice(&conv::to_string_bytes(v));
    }
    Ok(Value::str(out))
}

fn fn_explode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let delim = conv::to_string_bytes(&deref(&args[0]));
    let s = conv::to_string_bytes(&deref(&args[1]));
    if delim.is_empty() {
        return Err(Thrown::error("ValueError", "explode(): Argument #1 ($separator) cannot be empty"));
    }
    let mut arr = PhpArray::new();
    let mut start = 0;
    while let Some(pos) = find_sub(&s[start..], &delim) {
        arr.push(Value::str(s[start..start + pos].to_vec()));
        start += pos + delim.len();
    }
    arr.push(Value::str(s[start..].to_vec()));
    Ok(Value::Array(arr))
}

fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

fn fn_strtoupper(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut b = conv::to_string_bytes(&deref(&args[0]));
    b.make_ascii_uppercase();
    Ok(Value::str(b))
}

fn fn_strtolower(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut b = conv::to_string_bytes(&deref(&args[0]));
    b.make_ascii_lowercase();
    Ok(Value::str(b))
}

fn fn_str_contains(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let haystack = conv::to_string_bytes(&deref(&args[0]));
    let needle = conv::to_string_bytes(&deref(&args[1]));
    Ok(Value::Bool(needle.is_empty() || find_sub(&haystack, &needle).is_some()))
}

fn fn_get_class(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match deref(&args[0]) {
        Value::Object(h) => {
            let cid = h.borrow().class;
            Ok(host.class_name(cid).map(Value::str).unwrap_or(Value::Bool(false)))
        }
        _ => Err(Thrown::type_error("get_class(): Argument #1 ($object) must be of type object")),
    }
}

fn fn_get_parent_class(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match args.first().map(deref) {
        Some(Value::Object(h)) => {
            let cid = h.borrow().class;
            Ok(host.class_parent_name(cid).map(Value::str).unwrap_or(Value::Bool(false)))
        }
        _ => Ok(Value::Bool(false)),
    }
}

fn fn_get_object_vars(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match deref(&args[0]) {
        Value::Object(h) => {
            let mut arr = PhpArray::new();
            for (k, v) in h.borrow().properties.iter() {
                arr.insert(phrust_values::ArrayKey::Str(k.clone()), v.clone());
            }
            Ok(Value::Array(arr))
        }
        _ => Err(Thrown::type_error("get_object_vars(): Argument #1 must be of type object")),
    }
}

fn fn_ord(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let b = conv::to_string_bytes(&deref(&args[0]));
    Ok(Value::Int(b.first().copied().unwrap_or(0) as i64))
}

fn fn_chr(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = conv::to_int(&deref(&args[0])).rem_euclid(256) as u8;
    Ok(Value::str(vec![n]))
}

// small helper on Value for error messages
trait TypeNameLower {
    fn type_name_lower(&self) -> &'static str;
}
impl TypeNameLower for Value {
    fn type_name_lower(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) | Value::Closure(_) => "object",
            Value::Ref(r) => r.borrow().type_name_lower(),
        }
    }
}
