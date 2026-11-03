# Minimal Builtins for rhp Phases 1–4

Status: design. Target semantics: **PHP 8.5** (ground truth: the reference `php`, see `crates/rhp-builtins/src/version.rs`; written against 8.5.8).
All behavior in this document was verified by running the `php` binary; representative
runs are reproduced inline.

This is the minimal native-function + constant set required to make a tree-walking
interpreter testable through phases 1–4 (literals/expressions, variables, control flow,
arrays/strings). It deliberately excludes userland definitions, OOP, and I/O beyond
stdout. It also fixes the **native builtin interface** and lists the functions we
**refuse** by policy.

---

## 1. The native builtin interface

### 1.1 Value model assumptions

Builtins operate on **owned** runtime `Value`s (the AST is arena-borrowed and transient;
values stored in variables/arrays must outlive any statement — so builtins never borrow
from the arena). Assumed shape:

```rust
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),              // PHP int == platform i64 on this target (PHP_INT_SIZE = 8)
    Float(f64),
    Str(Rc<PhpStr>),       // byte string, NOT UTF-8 guaranteed; strlen == byte length
    Array(Rc<RefCell<PhpArray>>),  // ordered int/string keyed map (insertion order)
    // Object(...) arrives with OOP phases; not needed for 1–4
}
```

`PhpStr` is a byte vector (PHP strings are byte strings). `PhpArray` is an ordered hash
map preserving insertion order with PHP's int/string key normalization.

### 1.2 The call signature

```rust
/// A thrown PHP value / fatal condition that unwinds the interpreter.
pub enum Thrown {
    /// A real PHP Throwable has been thrown (Error/TypeError/ValueError/...).
    /// Carries class name + message so phase-5 try/catch can match it.
    Error { class: &'static str, message: String },
    /// Engine-level refusal (unsupported-by-policy); becomes a fatal with a clear note.
    Unsupported { function: &'static str, reason: &'static str },
}

/// Every native builtin has this shape.
pub type NativeFn = fn(interp: &mut Interp, args: &[Value]) -> Result<Value, Thrown>;

pub struct Builtin {
    pub name: &'static str,
    pub func: NativeFn,
    pub min_args: usize,
    pub max_args: Option<usize>,   // None == variadic (sprintf, printf, implode-ish)
}
```

Notes on the shape:

- **`&mut Interp`** is required because some builtins have side effects on interpreter
  state: `echo`/`print`/`printf`/`print_r(_, false)`/`var_dump`/`var_export(_, false)`
  write to the output sink; `settype` mutates a variable (see §1.4); future builtins read
  config (precision). Pure functions simply ignore it.
- **`args: &[Value]`** — arguments are evaluated by the caller (left→right) into owned
  values before dispatch. Builtins read them by reference and return a fresh owned
  `Value`. By-reference parameters (`settype`, `sort`, …) are special-cased (§1.4).
- **`Result<Value, Thrown>`** — a builtin returns a value or unwinds. Argument-count and
  argument-type violations are **not** Rust panics; they return
  `Err(Thrown::Error{..})` so they behave like PHP's `ArgumentCountError` / `TypeError` /
  `ValueError` (all verified below).
- Functions that are "void" in PHP (`echo`, `var_dump`) return `Value::Null` (PHP returns
  `void`/null); `print` returns `Value::Int(1)`.

### 1.3 Dispatch and arity/type checking

The call evaluator resolves a function-call name (case-insensitive in PHP) against the
builtin table, checks `min_args`/`max_args`, then calls `func`. Arity failures map to:

```
ArgumentCountError: strlen() expects exactly 1 argument, 0 given
```
(verified: `php -r 'strlen();'`). Per-argument type coercion/validation is done **inside**
each builtin because PHP's rules are per-function (coercive scalar type juggling for
`strlen(123)` → `int(3)`, but a hard `TypeError` for `count(5)`).

### 1.4 By-reference builtins (`settype`)

`settype($var, $type)` mutates its first argument in place and returns `bool`. The call
evaluator must recognize the handful of by-ref builtins and pass an **lvalue handle**
(slot) instead of a value. Minimal approach for phases 1–4: a small allowlist mapping
`"settype" => &[0]` (ref param indices); the evaluator resolves arg 0 to a mutable slot
and the builtin writes back through it. A clean generalization is a second entry point:

```rust
pub type NativeRefFn = fn(&mut Interp, args: &mut [Slot]) -> Result<Value, Thrown>;
```

but a one-off is fine until sorting functions arrive.

### 1.5 Constants

Constants are not functions; register them in the global constant table at startup:

| Constant | Value (PHP 8.5.8) | Rust |
|---|---|---|
| `PHP_EOL` | `"\n"` (on this platform) | `Value::Str("\n")` |
| `PHP_INT_MAX` | `9223372036854775807` | `Value::Int(i64::MAX)` |
| `PHP_INT_MIN` | `-9223372036854775808` | `Value::Int(i64::MIN)` |
| `PHP_INT_SIZE` | `8` | `Value::Int(8)` |
| `COUNT_NORMAL` | `0` | `Value::Int(0)` |
| `COUNT_RECURSIVE` | `1` | `Value::Int(1)` |

`PHP_INT_MAX + 1` overflows to `float(9.223372036854776E+18)` — that's arithmetic
promotion, handled by the operator evaluator, not here.

---

## 2. echo / print are NOT function calls

Both are **language constructs** parsed as dedicated AST nodes (mago-syntax:
`Statement::Echo` holds a list of expressions; `print` is an `Expression` /
unary-prefix construct), so they never reach the builtin dispatch table.

| | `echo` | `print` |
|---|---|---|
| Grammar | statement, **comma-separated list** of exprs: `echo $a, $b;` | **expression** with a single operand: `print $a;` |
| Return value | none (not usable in an expression) | **always `int(1)`** — usable: `$x = (print "A") + 5;` → `$x == 6` |
| Arity | 1..N args | exactly 1 |
| Parens | `echo("x")` works but parens are just grouping, **not** a call; `echo("a","b")` is a syntax error | `print("x")` likewise grouping |

Verified:
```
$ php -r '$x=(print "A")+5; echo "\n",$x,"\n";'   =>  A\n6
$ php -r 'echo "a","b","c","\n";'                  =>  abc
```

**Engine handling:** the statement evaluator for `echo` iterates its expression list,
converts each to string (PHP string-cast rules, which may throw on arrays →
`Array to string conversion` warning, but yields `"Array"`), and writes to the output
sink. `print` is evaluated as an expression: string-cast its operand, write it, yield
`Value::Int(1)`. Both go through the **same** string-cast + output-sink path the
builtins use, so there's one place that owns stdout.

---

## 3. The minimal builtin set

String-cast / dump helpers below assume PHP's standard conversions:
`null→""`, `true→"1"`, `false→""`, `int→decimal`, `float→shortest round-trip`
(`var_dump`/`var_export` use serialize_precision = -1: `0.1`→`0.1`, `1/3`→`0.3333333333333333`).

### 3.1 Type inspection

**`gettype(mixed $value): string`** — legacy names.
Returns: `"integer"`, `"double"`, `"string"`, `"boolean"`, `"array"`, `"NULL"`,
`"object"` (later), `"resource"`, `"unknown type"`.
```
gettype(1) => "integer"   gettype(1.0) => "double"   gettype(null) => "NULL"
```

**`settype(mixed &$var, string $type): bool`** — by-ref (§1.4). Valid `$type`:
`"int"/"integer"`, `"float"/"double"`, `"string"`, `"bool"/"boolean"`, `"array"`,
`"null"`. Returns `true`; an invalid type string throws `ValueError`.
```
$v="123"; settype($v,"integer"); // int(123)
$v=3.9;   settype($v,"int");      // int(3)
$v="x";   settype($v,"array");    // ["x"]
settype($v,"bogus"); // ValueError: settype(): Argument #2 ($type) must be a valid type
```

**`is_int` / `is_string` / `is_array` / `is_bool` / `is_float` / `is_null`
`(mixed $value): bool`** — exact type test, no coercion.
```
is_int(1) => true   is_int(1.0) => false   is_float(1.0) => true   is_null(null) => true
```
(`is_integer`/`is_long` and `is_double` are aliases; register as aliases if cheap.)

**`is_numeric(mixed $value): bool`** — true for ints, floats, and numeric strings.
PHP 8 accepts **leading AND trailing** whitespace; rejects hex strings.
```
is_numeric("123")=>true  is_numeric("12e3")=>true  is_numeric(" 12")=>true
is_numeric("12 ")=>true  (PHP 8 change)   is_numeric("0x1A")=>false   is_numeric("abc")=>false
```

### 3.2 Scalar conversions

**`intval(mixed $value, int $base = 10): int`** — base applies **only to string input**.
Leading-numeric parse, else 0. `$base = 0` auto-detects `0x`/`0b` prefixes (but **not**
`0o`; a leading `0` means octal).
```
intval("12abc")=>12   intval("abc")=>0   intval("0x1A",16)=>26   intval("100",2)=>4
intval("0x1A",0)=>26  intval("0b101",0)=>5  intval("017",0)=>15  intval(3.99)=>3
intval(true)=>1       intval(null)=>0
```

**`floatval(mixed $value): float`** — leading-numeric parse of strings.
```
floatval("1.5e2xx") => 150.0
```

**`strval(mixed $value): string`** — PHP string cast. Array → `"Array"` + warning.
```
strval(3.14)=>"3.14"  strval(true)=>"1"  strval(false)=>""  strval([1]) => "Array"(+warn)
```

**`boolval(mixed $value): bool`** — PHP truthiness. False-y: `0`, `0.0`, `""`, `"0"`,
`[]`, `null`, `false`. Everything else true (note `"0.0"` and `"0 "` are **true**).
```
boolval("0")=>false  boolval("0.0")=>true  boolval([])=>false  boolval([0])=>true
```

### 3.3 Output / debugging

**`var_dump(mixed ...$values): void`** → writes typed, indented dump; returns null.
```
var_dump(42);        => int(42)
var_dump(3.0);       => float(3)
var_dump("hi");      => string(2) "hi"      // length is BYTE length
var_dump(null);      => NULL
var_dump([1,"a"=>2]) =>
array(2) {
  [0]=>
  int(1)
  ["a"]=>
  int(2)
}
```

**`print_r(mixed $value, bool $return = false): string|true`** — human-readable. With
`$return = true` returns the string instead of printing (then returns the string);
otherwise prints and returns `true`.
```
print_r([1,"a"=>2,[3,4]]) =>
Array
(
    [0] => 1
    [a] => 2
    [1] => Array
        (
            [0] => 3
            [1] => 4
        )

)
print_r(42, true) => "42"   (string)
```

**`var_export(mixed $value, bool $return = false): string|null`** — valid-PHP-literal
form. Note whole floats print as `N.0` (round-trippable), strings single-quoted, assoc
arrays as `array ( k => v, )`.
```
var_export([1,"a"=>2]) =>
array (
  0 => 1,
  'a' => 2,
)
var_export("hi") => 'hi'    var_export(true) => true    var_export(null) => NULL
var_export(3.0) => 3.0      var_export(3.14) => 3.14     var_export(0.1) => 0.1
```

### 3.4 String / array utilities

**`strlen(string $string): int`** — **byte** length (coerces scalar args).
```
strlen("héllo") => 6   // é is 2 UTF-8 bytes
strlen(123)     => 3   // coerced to "123"
```

**`count(Countable|array $value, int $mode = COUNT_NORMAL): int`** — `TypeError` on a
non-countable scalar (not a warning in PHP 8).
```
count([1,2,3]) => 3
count([1,[2,3]], COUNT_RECURSIVE) => 4
count(5) => TypeError: count(): Argument #1 ($value) must be of type Countable|array, int given
```

**`str_repeat(string $string, int $times): string`** — `$times` must be >= 0.
```
str_repeat("ab",3) => "ababab"   str_repeat("x",0) => ""
```

**`implode(string $separator, array $array): string`** — glue is first. The legacy
`implode($array, $sep)` order is **removed in PHP 8** (passing array first → `TypeError`).
Single-arg `implode($array)` (no glue) is allowed.
```
implode(",", [1,2,3]) => "1,2,3"    implode([1,2,3]) => "123"
implode([1,2], ",")   => TypeError (array given for $separator)
```

**`explode(string $separator, string $string, int $limit = PHP_INT_MAX): array`** —
empty separator throws `ValueError`.
```
explode(",", "a,b,c")    => ["a","b","c"]
explode(",", "a,b,c", 2) => ["a","b,c"]
explode(",", "")         => [""]
explode("", "abc")       => ValueError: explode(): Argument #1 ($separator) must not be empty
```

### 3.5 Formatting: sprintf / printf

**`sprintf(string $format, mixed ...$args): string`**
**`printf(string $format, mixed ...$args): int`** — printf writes and returns the number
of bytes written.

Directive grammar to support: `%[argnum$][flags][width][.precision]specifier`.

| Spec | Meaning | Example |
|---|---|---|
| `%s` | string | `sprintf("%s","hi")` → `hi` |
| `%d` | signed decimal int (floats **truncated**, strings leading-num) | `sprintf("%d",3.9)`→`3`, `sprintf("%d","42abc")`→`42` |
| `%f` | float, default 6 decimals | `sprintf("%f",3.14)`→`3.140000` |
| `%b` | binary | `sprintf("%b",5)`→`101` |
| `%x`/`%X` | hex lower/upper | `sprintf("%x",255)`→`ff`, `%X`→`FF` |
| `%o` | octal | `sprintf("%o",8)`→`10` |
| `%e`/`%E` | scientific | `sprintf("%e",12345.678)`→`1.234568e+4` |
| `%g`/`%G` | shortest of e/f | `sprintf("%g",0.00001234)`→`1.234e-5` |
| `%c` | char from codepoint byte | `sprintf("%c",65)`→`A` |
| `%%` | literal percent | `%%`→`%` |

Flags/width/precision verified:
```
sprintf("%04d", 7)     => "0007"    (zero pad, width 4)
sprintf("%5.2f", 3.14159) => " 3.14"  (width 5, 2 decimals)
sprintf("%-5d|", 7)    => "7    |"  (left-justify)
sprintf("%+d", 7)      => "+7"      (force sign)
sprintf("%'*10d", 42)  => "********42"  (custom pad char after ')
sprintf("%1$s %2$s %1$s", "a","b") => "a b a"  (positional args)
```
Array passed to `%s` → `"Array"` + `Array to string conversion` warning.

---

## 4. Refused functions (policy)

These are **not** implemented. The parser/resolver detects them by name and the engine
raises a controlled fatal rather than silently misbehaving. They defeat static analysis,
inject scope mutation, or evaluate code — all incompatible with a tree-walking engine that
wants a known variable set and no runtime code generation.

Error mechanism: resolving a call to any name in the refused set returns
`Err(Thrown::Unsupported { function, reason })`, which the top-level loop renders as a
fatal:
```
Fatal error: rhp: <function>() is not supported (<reason>). [RHP_UNSUPPORTED]
```
This is distinct from PHP's own "undefined function" `Error`, so tests can assert on the
`RHP_UNSUPPORTED` marker and we never accidentally pretend to support them.

| Function | Why refused | Note on real PHP |
|---|---|---|
| `eval` | runtime code execution; no AST to walk ahead of time | language construct, not even a function |
| `create_function` | runtime code gen | **already removed in PHP 8** — real PHP throws `Error: Call to undefined function` |
| `assert` **with a string argument** | string form historically eval'd code | in PHP 8 the string is **no longer evaluated** (it's just truthy / used as description): `assert("1==2")` returns `true`. We refuse the string form to avoid the footgun; `assert(bool)` MAY be supported later |
| `extract` | injects arbitrary names into the current symbol table from array keys → unknown variable set | works in PHP; `extract(["a"=>1])` creates `$a` |
| `compact` | reads variables by name from strings → couples scope to string data | works in PHP; `compact("a","b")` → `["a"=>..,"b"=>..]` |
| **variable-variables into scope** (`$$name = ...`) | AST-level construct that writes a slot named by a runtime string | not a function — detected in the assignment evaluator; refuse the write form (`$$x = v`) with the same `RHP_UNSUPPORTED` fatal |

Detection points:
- `eval`, variable-variable assignment: **AST node kinds** (reject in the evaluator).
- `create_function`, `extract`, `compact`: **names in the builtin resolver** (reject at
  call resolution).
- `assert`: resolver inspects arg 0; if it is (or evaluates to) a string literal, refuse;
  otherwise (future) allow the boolean form.

---

## 5. Implementation order for phases 1–4

1. Output sink + string-cast + `echo`/`print` (statement/expression nodes). → testable "hello world".
2. `var_dump`, `gettype`, `is_*`, constants. → inspect values in tests.
3. `intval/floatval/strval/boolval`, `settype`, `is_numeric`. → conversion coverage.
4. `strlen`, `count`, `str_repeat`, `implode`, `explode`. → string/array phase.
5. `sprintf`/`printf`, `print_r`, `var_export`. → formatting + golden-output diffing vs `php`.
6. Wire the **refused** set early (step 1) so unsupported code fails loudly from day one.
