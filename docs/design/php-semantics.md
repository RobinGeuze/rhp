# PHP 8.5 Ground-Truth Semantics (phrust compatibility spec)

Captured by running the local `php` binary: **PHP 8.5.8 (cli) (NTS), Zend Engine v4.5.8**, Homebrew build, on darwin.

Every snippet below was executed with `php -r '<snippet>'` (or as a script file). The output shown is the **exact stdout** unless a line is explicitly labeled a warning/error (those go to **stderr** in the CLI SAPI). These are the semantics our tree-walking interpreter must reproduce; the golden fixtures in `tests/golden/` assert a subset of them.

Key ini defaults that govern number formatting:

| ini setting | value | governs |
|---|---|---|
| `precision` | `14` | `echo`, `print`, `(string)`, string interpolation, `.` concat of floats |
| `serialize_precision` | `-1` | `var_dump`, `var_export`, `json_encode`, `serialize` (`-1` = shortest round-trippable repr) |

This split is the single most important float rule: **`echo` of a float uses 14 significant digits; `var_dump`/`var_export`/`json_encode` use the shortest string that round-trips to the same double.** That is why `echo 0.1+0.2` → `0.3` but `var_dump(0.1+0.2)` → `float(0.30000000000000004)`.

---

## 1. `echo` / `print` output format

`echo` converts each operand to string and writes it; no separator, no trailing newline. `print` is an expression that always evaluates to `int(1)`.

| snippet | exact stdout |
|---|---|
| `echo 42;` | `42` |
| `echo -42;` | `-42` |
| `echo 1.0;` | `1` |
| `echo 0.1+0.2;` | `0.3` |
| `echo 1/3;` | `0.33333333333333` (14 sig digits) |
| `echo 1e20;` | `1.0E+20` |
| `echo 1.5e-10;` | `1.5E-10` |
| `echo INF;` | `INF` |
| `echo -INF;` | `-INF` |
| `echo NAN;` | `NAN` **+ warning** (see below) |
| `echo -0.0;` | `-0` |
| `echo true;` | `1` |
| `echo false;` | `` (empty string) |
| `echo null;` | `` (empty string) |
| `echo "hello";` | `hello` |
| `$r = print "hi";` then `echo "|$r";` | `hi|1` |
| `echo 0.123456789012345678;` | `0.12345678901235` |

NAN to string emits a warning (new/strict in PHP 8.x) but still prints `NAN`:
```
echo NAN;
```
stderr: `Warning: unexpected NAN value was coerced to string in Command line code on line 1`
stdout: `NAN`

### Float formatting threshold for `echo` (precision = 14)

PHP switches to scientific notation when the exponent falls outside the range representable in 14 significant digits.

| snippet | exact stdout |
|---|---|
| `echo 0.0001;` | `0.0001` |
| `echo 0.00001;` | `1.0E-5` |
| `echo 100000000000000.0;` (1e14) | `1.0E+14` |
| `echo 1000000000000000.0;` (1e15) | `1.0E+15` |
| `echo 10000000000000000.0;` (1e16) | `1.0E+16` |
| `echo 123456789012345.6;` | `1.2345678901235E+14` |
| `echo 1234567890123456789;` (int literal) | `1234567890123456789` (exact, it is an int) |
| `echo 12345678901234567890;` (overflows int → float) | `1.2345678901235E+19` |

The scientific form is `D.DDDDE±XX`: mantissa always has a decimal point (`1.0E+20`, not `1E+20`), exponent sign is always present, exponent has no leading zero (`E-5`, not `E-05`).

---

## 2. `var_dump` (uses serialize_precision = -1)

Each top-level argument prints on its own line(s), each terminated by `\n`. Nested array elements are indented 2 spaces per level.

| snippet | exact stdout (each line `\n`-terminated) |
|---|---|
| `var_dump(42);` | `int(42)` |
| `var_dump(-42);` | `int(-42)` |
| `var_dump(1.0);` | `float(1)` |
| `var_dump(0.1+0.2);` | `float(0.30000000000000004)` |
| `var_dump(1/3);` | `float(0.3333333333333333)` |
| `var_dump(INF);` | `float(INF)` |
| `var_dump(NAN);` | `float(NAN)` (no warning here) |
| `var_dump(-0.0);` | `float(-0)` |
| `var_dump(true);` | `bool(true)` |
| `var_dump(false);` | `bool(false)` |
| `var_dump(null);` | `NULL` |
| `var_dump("hello");` | `string(5) "hello"` |
| `var_dump("");` | `string(0) ""` |
| `var_dump("héllo");` | `string(6) "héllo"` (byte length, not codepoints — `é` is 2 bytes) |

`var_dump(1, "two", 3.0);` (multiple args) →
```
int(1)
string(3) "two"
float(3)
```

Simple array `var_dump([1,2,3]);` →
```
array(3) {
  [0]=>
  int(1)
  [1]=>
  int(2)
  [2]=>
  int(3)
}
```

Assoc `var_dump(["a"=>1,"b"=>2]);` →
```
array(2) {
  ["a"]=>
  int(1)
  ["b"]=>
  int(2)
}
```

Nested `var_dump([1,["x"=>2,"y"=>[3,4]],"s"=>"str"]);` →
```
array(3) {
  [0]=>
  int(1)
  [1]=>
  array(2) {
    ["x"]=>
    int(2)
    ["y"]=>
    array(2) {
      [0]=>
      int(3)
      [1]=>
      int(4)
    }
  }
  ["s"]=>
  string(3) "str"
}
```
Rules: int keys printed bare `[0]=>`, string keys quoted `["a"]=>`. Key line and value line are separate lines. Indent = 2 spaces × depth. Closing `}` is at the parent's indent.

---

## 3. `print_r` and `var_export`

### print_r

Scalars print as their string form, with no newline:

| snippet | stdout |
|---|---|
| `print_r(42);` | `42` |
| `print_r("hi");` | `hi` |
| `print_r(true);` | `1` |
| `print_r(false);` | `` (empty) |
| `print_r(null);` | `` (empty) |

`print_r([1,2,3]);` →
```
Array
(
    [0] => 1
    [1] => 2
    [2] => 3
)
```
(note: trailing `\n` after the `)`).

`print_r([1,["x"=>2,"y"=>[3,4]],"s"=>"str"]);` →
```
Array
(
    [0] => 1
    [1] => Array
        (
            [x] => 2
            [y] => Array
                (
                    [0] => 3
                    [1] => 4
                )

        )

    [s] => str
)
```
Rules: header `Array\n`, then `(\n`, entries as `    [key] => value`. Base indent is 4 spaces. Nested arrays indent the `(` and children by 8 more spaces relative to the key, and there is a **blank line after each nested `)`**. Keys are printed bare (no quotes) for both int and string keys.

### var_export (uses serialize_precision = -1; valid PHP syntax, no trailing newline)

| snippet | stdout |
|---|---|
| `var_export(42);` | `42` |
| `var_export(1.0);` | `1.0` (float always gets `.0` to stay a float) |
| `var_export(0.1+0.2);` | `0.30000000000000004` |
| `var_export("hi");` | `'hi'` (single-quoted) |
| `var_export(true);` | `true` |
| `var_export(false);` | `false` |
| `var_export(null);` | `NULL` |

`var_export([1,2,3]);` →
```
array (
  0 => 1,
  1 => 2,
  2 => 3,
)
```

`var_export([1,["x"=>2,"y"=>[3,4]],"s"=>"str"]);` →
```
array (
  0 => 1,
  1 => 
  array (
    'x' => 2,
    'y' => 
    array (
      0 => 3,
      1 => 4,
    ),
  ),
  's' => 'str',
)
```
Rules: `array (` on one line. Indent = 2 spaces × depth. Every element line ends with a trailing comma, including the last. A nested array's `array (` goes on its **own line** after `key => ` (note the trailing space after `=>` then newline). Int keys bare, string keys single-quoted. No trailing newline is emitted by `var_export` itself.

`json_encode` for comparison: `json_encode(0.1+0.2)` → `0.30000000000000004`; `json_encode(1.0)` → `1` (json drops the `.0`).

---

## 4. Integer semantics

| constant | value |
|---|---|
| `PHP_INT_MAX` | `9223372036854775807` (2^63−1, 64-bit) |
| `PHP_INT_MIN` | `-9223372036854775808` |
| `PHP_INT_SIZE` | `8` |

**Overflow promotes to float** (never wraps):
- `var_dump(PHP_INT_MAX);` → `int(9223372036854775807)`
- `var_dump(PHP_INT_MAX + 1);` → `float(9.223372036854776E+18)`
- `echo PHP_INT_MAX + 1;` → `9.2233720368548E+18` (echo uses precision 14)

**Division `/`** returns int only when it divides evenly, else float:
- `var_dump(7/2);` → `float(3.5)`
- `var_dump(6/2);` → `int(3)`
- `var_dump(6/3);` → `int(2)`

**`intdiv`** truncates toward zero:
- `intdiv(7,2)` → `int(3)`
- `intdiv(-7,2)` → `int(-3)`

**Modulo `%`** (integer; operands cast to int) — result takes the **sign of the dividend**:
- `7 % 3` → `int(1)`
- `-7 % 3` → `int(-1)`
- `7 % -3` → `int(1)`
- `-7 % -3` → `int(-1)`
- `fmod(-7,3)` → `float(-1)` (float modulo, also dividend sign)

**Power `**`** (right-associative; binds tighter than unary minus):
- `2 ** 10` → `int(1024)`
- `2 ** -1` → `float(0.5)`
- `2 ** 0.5` → `float(1.4142135623730951)`
- `2 ** 63` → `float(9.223372036854776E+18)` (exceeds int range → float)
- `-2 ** 2` → `int(-4)` (parsed as `-(2 ** 2)`)

**Bitwise** (integer):
- `5 & 3` → `int(1)`, `5 | 2` → `int(7)`, `5 ^ 1` → `int(4)`
- `~5` → `int(-6)` (two's complement)
- `1 << 4` → `int(16)`, `256 >> 2` → `int(64)`

**Division/modulo by zero throw** `DivisionByZeroError` (a Throwable, catchable), not a warning:
- `intdiv(1,0)` → `Uncaught DivisionByZeroError: Division by zero`
- `1 % 0` → `Uncaught DivisionByZeroError: Modulo by zero`
- `1 / 0` → `Uncaught DivisionByZeroError: Division by zero`

---

## 5. Type juggling & loose comparison (PHP 8 rules)

**PHP 8 changed number↔string `==`:** when comparing a number to a string, if the string is **not** numeric, the **number is cast to string** (string comparison); only if the string is numeric does PHP compare numerically. (PHP 7 did the opposite and gave `0 == "a"` → true.)

| expression | result |
|---|---|
| `0 == "a"` | `false` (PHP 7 gave true) |
| `0 === "a"` | `false` (different types) |
| `"1" == "01"` | `true` (both numeric strings → numeric compare) |
| `"10" == "1e1"` | `true` (10 == 10.0) |
| `100 == "1e2"` | `true` (string is numeric → 100 == 100) |
| `"abc" == 0` | `false` (string non-numeric → "0" == "abc") |
| `"1abc" == 1` | `false` (leading-numeric but not fully numeric → "1" == "1abc") |
| `null == false` | `true` |
| `null == 0` | `true` |
| `null == ""` | `true` |
| `null == []` | `true` |
| `[] == false` | `true` |
| `"" == null` | `true` |
| `"0" == false` | `true` |
| `"" == false` | `true` |
| `0 == ""` | `false` (PHP 8: int↔non-numeric-string → "0" == "") |
| `0 == null` | `true` |
| `"abc" == "abc"` | `true` |

**Spaceship `<=>`** returns `int(-1|0|1)`:

| expression | result |
|---|---|
| `1 <=> 2` | `-1` |
| `2 <=> 1` | `1` |
| `1 <=> 1` | `0` |
| `"a" <=> "b"` | `-1` |
| `"10" <=> "9"` | `1` (both numeric → numeric) |
| `"10" <=> "9a"` | `-1` ("9a" non-numeric → string compare "10" vs "9a", '1' < '9') |
| `[1,2] <=> [1,2]` | `0` |
| `[1,2,3] <=> [1,2]` | `1` (more elements → greater) |
| `true <=> false` | `1` |
| `1 <=> "1"` | `0` |
| `null <=> false` | `0` |

**Ordering `<` — strings vs numbers:** two numeric strings compare numerically; otherwise byte-wise (ASCII) string comparison.

| expression | result |
|---|---|
| `"abc" < "abd"` | `true` |
| `"Z" < "a"` | `true` (`Z`=0x5A < `a`=0x61) |
| `"10" < "9"` | `false` (numeric: 10 < 9 is false) |
| `10 < 9` | `false` |
| `"apple" < "banana"` | `true` |

---

## 6. String ↔ number coercion & casts

**`(int)` cast** parses a leading optional-sign integer prefix (after leading whitespace), ignoring trailing junk; no hex/binary/octal prefix recognition:

| snippet | result |
|---|---|
| `(int)"123abc"` | `int(123)` |
| `(int)"abc"` | `int(0)` |
| `(int)"12.9"` | `int(12)` (stops at `.`) |
| `(int)12.9` | `int(12)` (truncates toward zero) |
| `(int)-12.9` | `int(-12)` |
| `(int)"  42  "` | `int(42)` (leading whitespace skipped) |
| `(int)"0x1A"` | `int(0)` (hex prefix NOT honored; parses leading `0`) |
| `(int)"0b11"` | `int(0)` |
| `(int)"010"` | `int(10)` (NOT octal in casts; decimal) |

**`(float)` cast** parses a leading float (incl. exponent); non-numeric → `0`; trailing junk ignored **without a warning** (unlike arithmetic):

| snippet | result |
|---|---|
| `(float)"1.5e3"` | `float(1500)` |
| `(float)"abc"` | `float(0)` |
| `(float)"5 apples"` | `float(5)` (no warning on cast) |

**Arithmetic on numeric-ish strings:**

| snippet | result | note |
|---|---|---|
| `"5" + 3` | `int(8)` | |
| `"5.5" + 3` | `float(8.5)` | |
| `"5 apples" + 3` | `int(8)` | **+ Warning:** `A non-numeric value encountered` (leading-numeric string) |
| `"  5  " + 3` | `int(8)` | leading **and trailing** whitespace OK in PHP 8, no warning |
| `"1e2" + 0` | `float(100)` | exponent string is numeric → float |
| `"abc" + 3` | **Fatal** `TypeError: Unsupported operand types: string + int` | fully non-numeric string in arithmetic throws |

So three tiers for `+`: fully numeric string → silent; leading-numeric ("5 apples") → **warning** but uses the leading number; non-numeric ("abc") → **TypeError**.

**`is_numeric`** (PHP 8 allows trailing whitespace; still no hex/binary):

| snippet | result |
|---|---|
| `is_numeric("123")` | `true` |
| `is_numeric("1.5e3")` | `true` |
| `is_numeric(" 123")` | `true` (leading ws) |
| `is_numeric("123 ")` | `true` (**trailing ws allowed since PHP 8.0**) |
| `is_numeric("0x1A")` | `false` (hex strings are NOT numeric) |
| `is_numeric("0b11")` | `false` |
| `is_numeric(".5")` | `true` |
| `is_numeric("5.")` | `true` |

---

## 7. `gettype()` / `get_debug_type()` names and `is_*`

| value | `gettype()` | `get_debug_type()` |
|---|---|---|
| `42` | `integer` | `int` |
| `1.5` | `double` | `float` |
| `"s"` | `string` | `string` |
| `true` | `boolean` | `bool` |
| `null` | `NULL` | `null` |
| `[]` | `array` | `array` |

Note the legacy `gettype` names (`integer`, `double`, `boolean`, `NULL`) differ from the modern `get_debug_type` names (`int`, `float`, `bool`, `null`).

`is_*` results:

| value | is_int | is_float | is_string | is_bool | is_null | is_array | is_numeric | is_scalar |
|---|---|---|---|---|---|---|---|---|
| `42` | true | false | false | false | false | false | true | true |
| `"123"` | false | false | true | false | false | false | **true** | true |
| `1.0` | false | true | — | — | — | — | true | true |
| `null` | false | — | — | false | true | — | — | **false** |

(`is_scalar(null)` is **false**; `is_numeric("123")` is **true** even though it's a string.)

---

## 8. Concatenation `.` (uses precision = 14)

The `.` operator stringifies both sides with the same rules as `echo`.

| snippet | stdout |
|---|---|
| `echo 1 . 2;` | `12` |
| `echo 1.5 . "x";` | `1.5x` |
| `echo "a" . true . "b";` | `a1b` |
| `echo "a" . false . "b";` | `ab` (false → "") |
| `echo "a" . null . "b";` | `ab` (null → "") |
| `echo "n=" . 3.0;` | `n=3` (float 3.0 → "3") |
| `echo "r=" . (0.1+0.2);` | `r=0.3` (precision 14, not the var_dump form) |
| `echo 1e20 . "";` | `1.0E+20` |

---

## Additional casts (reference)

| snippet | result |
|---|---|
| `(string)1.0` | `"1"` |
| `(string)(0.1+0.2)` | `"0.3"` |
| `(string)INF` | `"INF"` |
| `(string)true` | `string(1) "1"` |
| `(string)false` | `string(0) ""` |
| `(string)null` | `string(0) ""` |
| `(bool)"0"` | `false` |
| `(bool)""` | `false` |
| `(bool)"0.0"` | `true` (only exactly "0" and "" are falsy strings) |
| `(bool)"false"` | `true` |
| `(bool)0.0` | `false` |
| `(bool)[]` | `false` |
| `(bool)[0]` | `true` (any non-empty array) |

Falsy values in PHP: `false`, `0`, `0.0`, `-0.0`, `""`, `"0"`, `[]`, `null`. Everything else is truthy (notably `"0.0"`, `"false"`, `"0 "`, and `-0`-as-int which is just `0`).

---

## Golden fixtures

21 `NAME.php` / `NAME.out` pairs live in `tests/golden/`. Each `.out` is the **exact stdout** of `php NAME.php` under PHP 8.5.8 (stderr/warnings excluded so the bytes are path-independent and deterministic). Fixtures avoid fatal-error cases (those are specified narratively above). The pairs:

`echo_int`, `echo_float_formats`, `echo_bool_null`, `var_dump_scalars`, `var_dump_nested`, `print_r_nested`, `var_export_nested`, `int_overflow`, `intdiv_mod`, `power_operator`, `bitwise`, `comparison_php8`, `spaceship`, `string_ordering`, `int_casts`, `float_casts`, `is_numeric_cases`, `numeric_string_arith`, `gettype_names`, `concatenation`, `bool_casts`.
