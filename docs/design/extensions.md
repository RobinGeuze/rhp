# Extensions & native-library reuse

**Status:** design / research. No engine behavior changes here.
**Question:** can `rhp` reuse native C code to get byte-exact PHP behavior with
less reimplementation, and if so, *which* native code and *how*?
**Ground truth:** the reference `php` (8.5.12, PCRE 10.49 2026-09-28; every
version is in `crates/rhp-builtins/src/version.rs`). Written against 8.5.8 / PCRE 10.47.

This doc distinguishes three very different things that all get called
"extensions", gives a verdict on each, then proposes a concrete
**native-backed builtin** architecture that fits the existing
`registry()` / `Host` layering, and finishes with a ranked roadmap, a
`json_decode` recommendation, and the build/portability consequences.

> **Decision (2026-10-24).** The macro-shim prototype of §9 is **abandoned** —
> `crates/rhp-zendshim` has been removed. The chosen strategy is to
> **reimplement each extension in Rust and prove it byte-identical to the real
> `php` binary with a differential-test framework** (`crates/rhp-difftest`,
> documented in §10). Pure-Rust crates are used where a faithful 1:1 exists; a
> C system library (e.g. libmysqlclient, libpq) is permitted only where no 1:1
> Rust alternative exists. The first extension reimplemented this way is `hash`
> (§10).

---

## 0. TL;DR verdicts

| Category | Verdict | One-line reason |
|---|---|---|
| 1. Load compiled Zend extensions (`json.so`, `mbstring.so`) | **Infeasible** | They are ABI clients of the Zend Engine — zval layout, Zend MM, executor hooks. Loading one means *being* the Zend engine. |
| 1b. Compile ext *source* against a macro shim (§9, prototyped) | **Abandoned** | Proven byte-exact for small/numeric functions, but the macro/type surface balloons for objects/globals/HashTable iteration. Superseded by reimplement-in-Rust + differential testing (§10); `crates/rhp-zendshim` removed. |
| 2. Link the C libs PHP builds on (pcre2, ICU, libxml2, OpenSSL, zlib, gmp/mpdecimal, oniguruma) | **Feasible** via `-sys` crates / bindgen | This is where byte-exactness actually lives. Caveat: must match PHP's *same lib + version + build options*, and account for PHP's own wrapping. |
| 3. PHP's own C ext code that is *not* a system lib (`ext/json`, `ext/standard`) | **Feasible but a judgment call per-function** | Can vendor the C, bind a Rust crate + patch quirks, or reimplement. For `json_decode`: **reimplement in Rust against the differential harness** (recommended below). |

The through-line: **rhp reimplements each extension in Rust and proves it
byte-identical to the C `php` binary with a differential harness (§10); it reuses
the same underlying libraries PHP links (pcre2, and, where no 1:1 Rust crate
exists, C system libs) rather than loading or recompiling Zend extensions.**

---

## 1. Category 1 — loading compiled Zend extensions (`.so`): INFEASIBLE

### What a Zend extension actually is

A distro `json.so` / `mbstring.so` is **not** a freestanding C library with a
neutral C ABI. It is compiled against `php-src`'s internal headers and is a
client of the **Zend Engine ABI**. Concretely, a module's functions receive
and manipulate:

- **`zval`** — the 16-byte tagged union (`zend_value` + `u1`/`u2` type-info
  words), with reference-counted `zend_refcounted` payloads (`zend_string`,
  `zend_array` aka HashTable, `zend_object`, `zend_reference`). Every argument
  arrives as a `zval*`, and return values are written through
  `return_value`/`ZVAL_*` macros. The *memory layout* is the contract.
- **`INTERNAL_FUNCTION_PARAMETERS`** — the hidden calling convention
  (`execute_data`, `return_value`), and `ZEND_PARSE_PARAMETERS_*` /
  `zend_parse_parameters` for arg coercion, which itself calls back into the
  engine's type-juggling and error machinery.
- **The Zend Memory Manager** — `emalloc`/`efree`/`erealloc`, a custom
  per-request arena allocator. zvals and `zend_string`s must be allocated and
  freed through it; their refcounting interlocks with the GC.
- **Module lifecycle hooks** — `zend_module_entry` with
  `MINIT`/`MSHUTDOWN`/`RINIT`/`RSHUTDOWN`/`MINFO`, the `ZEND_GET_MODULE`
  symbol, and a `ZEND_MODULE_API_NO` (the ABI stamp, e.g. `20240924`) that the
  engine checks on `dlopen` and *rejects on mismatch*.
- **Executor and INI hooks** — extensions register INI entries, opcode
  handlers, `zend_execute_ex`/`zend_compile` overrides (opcache, xdebug),
  resource destructors, class entries, and stream wrappers — all against engine
  globals (`EG(...)`, `CG(...)`, `PG(...)`).

### Why loading one into rhp is effectively reimplementing Zend

To `dlopen` a stock `json.so` and call into it, rhp would have to present,
byte-for-byte, the Zend engine it was compiled against:

1. **Replicate the exact `zval`/HashTable/`zend_string` memory layout** for the
   target PHP version and build (NTS vs ZTS changes struct shapes; debug builds
   add fields). rhp's `Value` is a Rust enum with `Rc<BString>` strings and
   an `indexmap`-backed array — a *completely different* representation. Every
   call boundary would need a bidirectional, allocation-correct zval marshaller.
2. **Provide the Zend MM, the GC, and all `EG`/`CG`/`PG` globals** the module
   touches, with matching semantics, because the `.so` frees what it allocated
   and expects the engine's refcount/GC invariants.
3. **Match the `ZEND_MODULE_API_NO`** and the entire exported symbol surface
   (`zend_parse_parameters`, `zend_hash_*`, `zend_throw_exception`,
   `zend_register_*`, hundreds of symbols), with identical behavior — the `.so`
   is dynamically linked against these and will resolve them against *our*
   process. Any divergence is UB, not a graceful error.
4. **Pin ABI per PHP patch build**. The layout and symbol set are stable only
   within an API-number window; a user's `json.so` from a different distro
   build can mismatch.

That is not "loading an extension" — it is shipping a binary-compatible
reimplementation of the Zend Engine's runtime and ABI, which is the entire
thing rhp set out *not* to be (register VM + owned `Value`s, PHP strings as
`Rc<BString>`, no Zend internals). It also directly conflicts with rhp's
"no `eval`, structural refusals, everything owned/`'static`" posture, and would
re-import exactly the C-level memory-safety surface the project is replacing.

**Verdict: do not attempt.** There is no partial win here: a Zend extension is
all-or-nothing against the engine ABI. (The only theoretical carve-out — a
pure, self-contained leaf function that never allocates a zval or touches a
global — does not exist in practice among the real extensions anyone wants.)

> Note on FFI subagents / "out-of-process PHP": one could shell out to a real
> `php` to service specific calls, but that is adopting PHP, not reusing an
> extension in rhp, and it reintroduces a PHP runtime dependency the project
> is eliminating. Out of scope.

---

## 2. Category 2 — linking the C libraries PHP builds on: FEASIBLE

This is the productive path. Many PHP functions are thin wrappers over a
*general-purpose* C library with a **stable, neutral C ABI** that is already
exposed to Rust via maintained `-sys` crates (or is trivially bindgen-able):

| PHP surface | Underlying C lib | Rust binding |
|---|---|---|
| `preg_*` | **PCRE2** (`libpcre2-8`) | `pcre2-sys` / `pcre2` crate |
| `intl`, parts of `mbstring` (casing, collation, normalization), `IntlDateFormatter` | **ICU** (`libicui18n`/`libicuuc`) | `icu`/`rust_icu_*` (bindgen over system ICU), or ICU4X (pure Rust, *different data*) |
| `mb_ereg*` | **Oniguruma** (`libonig`) | `onig` crate |
| `dom`, `simplexml`, `xml` | **libxml2** | `libxml`/bindgen |
| `openssl`, `hash` (some), `sodium` | **OpenSSL**, **libsodium** | `openssl`/`openssl-sys`, `libsodium-sys` |
| `zlib`, `gzencode`, `gzip` streams | **zlib** | `libz-sys`/`flate2` (zlib backend) |
| `bcmath` | **libmpdec** (mpdecimal) | bindgen (no mature crate) |
| `gmp` | **GMP** | `rug`/`gmp-mpfr-sys` |

### Why it is feasible

These libraries expose plain C functions over plain C types (`pcre2_compile`,
`u_strToUpper`, `BN_*`, `mpd_*`). There is no engine ABI, no zval, no shared
allocator requirement at the call boundary — rhp converts `&[u8]` ↔ the
lib's input/output buffers at the FFI edge and keeps its own `Value`
representation everywhere else. The existing `registry()` + `NativeFn`
signature already isolates each builtin, so a native backend is a drop-in body
swap for one function.

### The compatibility caveat (this is the whole game)

Byte-exactness with PHP requires matching **the same library, the same major
version, and the same build options** PHP used — *and* reproducing PHP's own
wrapper logic around it:

- **Same engine, not a sibling.** `fancy-regex` (what `preg_*` uses today, see
  `crates/rhp-builtins/src/preg_fns.rs`) is a *different* regex engine from
  PCRE2. Its header comment already enumerates the divergences it cannot avoid:
  possessive quantifiers, recursion/subroutines (`(?R)`, `(?1)`, `\g<1>`),
  `\G`, `(?'name'…)`, and `\d\w\s` being Unicode-aware vs PCRE's ASCII-default.
  Those are *semantic* mismatches no amount of wrapping fixes — only PCRE2
  itself matches PCRE2.
- **Version matters.** PCRE2 (10.49 in the reference php) and ICU releases change
  matchable Unicode property data, collation tables, and casing between
  versions. ICU is the worst offender: output depends on the **CLDR/ICU data
  version**, so even "the same ICU API" gives different collation keys across
  ICU majors. Matching PHP means matching PHP's ICU *data*, not just its API.
- **PHP wraps/patches behavior.** PHP does not call these libs raw:
  - PCRE: PHP parses the `/pat/flags` delimiter syntax, maps modifier letters
    to PCRE2 option bits, sets its own `pcre.backtrack_limit` /
    `pcre.recursion_limit` / `pcre.jit`, and does its own UTF-8 validation and
    `$matches` array shaping. All of that is PHP code wrapping PCRE2, and must
    be reproduced in rhp regardless of the backend.
  - mbstring: PHP has its own encoding tables and `mbstring.*` INI handling on
    top of whatever lib does the casing.
  - bcmath: `libmpdec` gives arbitrary-precision decimals, but PHP's scale
    handling, rounding mode, and string formatting are PHP's.
  So "link the C lib" removes the *algorithm* reimplementation, not the
  *PHP-wrapper* reimplementation. The wrapper is where the differential harness
  earns its keep.

**Verdict: adopt selectively, as opt-in native backends, with the differential
harness as the acceptance gate.** The pcre2 swap is the canonical first case.

---

## 3. Category 3 — PHP's own C ext code (`ext/json`, `ext/standard`): per-function judgment

`ext/json` is the instructive case: there is **no system `libjson`**. PHP's JSON
is bespoke C in `php-src/ext/json/` — a hand-written recursive-descent parser
(`json_parser.y` → `json_parser.tab.c`), a scanner (`json_scanner.re`), and an
encoder — none of it a shared library. So "link the lib PHP uses" does not
apply; the only options are:

- **(a) Vendor/bind php-src's `ext/json` C.** Pulls the exact parser, so
  byte-exactness is free. But: it is written against zval/HashTable/zend_string
  and the Zend MM (same ABI problem as Category 1, in miniature) — you'd write
  a zval↔`Value` shim for just this subtree, carry a C toolchain + a chunk of
  GPL-adjacent… (actually PHP license) C, and still have to translate its
  depth/flags/error machinery to rhp's. Lots of C surface for one function
  family.
- **(b) Use a Rust JSON crate (`serde_json`) and patch PHP's quirks.** Low
  effort to start, but a stock crate misses many PHP-specific behaviors (listed
  in §5) and some are *hard* to retrofit onto serde (big-int-as-string, PHP's
  array-key normalization, PHP's exact error codes/messages, depth counting).
- **(c) Reimplement in Rust against the differential harness.** A focused
  recursive-descent decoder over `&[u8]` that targets php's observable behavior
  directly. More up-front code than (b), but *every* quirk is expressible and
  each is pinned by a golden test. This matches how `json_encode`,
  `serialize`/`unserialize`, and the `preg_*` family were already built here.

**Verdict: (c) for `json_decode`** (full recommendation + quirk list + test plan
in §5). `ext/standard` functions are already being reimplemented one-by-one in
`rhp-builtins` and should continue that way; reach for Category-2 linking
only where a function *is* a thin wrapper over a shared lib (regex, intl, gmp).

---

## 4. The "native-backed builtin" architecture

Goal: let a single builtin delegate to a `-sys` crate **without** (a) forcing
that C dependency on every build, (b) changing the `registry()` / `Host`
contract, or (c) weakening the differential guarantee. The current layering
makes this clean — `rhp-builtins` depends only on `rhp-values` +
`rhp-semantics`, and each builtin is a `NativeFn = fn(&mut dyn Host,
&[Value]) -> Result<Value, Thrown>` looked up by name in `registry()`
(`crates/rhp-builtins/src/lib.rs:78`, `:101`).

### 4.1 Feature-gating (pure-Rust build stays the default)

Add Cargo features to `rhp-builtins`, off by default, one per native backend:

```toml
# crates/rhp-builtins/Cargo.toml
[features]
default = []
native-pcre2 = ["dep:pcre2"]
native-icu   = ["dep:rust_icu_ucol", "dep:rust_icu_ustring"]
native-gmp   = ["dep:rug"]
# umbrella for distro-parity builds:
native-all   = ["native-pcre2", "native-icu", "native-gmp"]

[dependencies]
fancy-regex = "0.13"          # pure-Rust fallback, always present
pcre2       = { version = "0.2", optional = true }
rug         = { version = "1", optional = true }
```

The crate always compiles with zero native libs (today's behavior). A parity
build opts in with `--features native-all`. `rhp-vm`, `rhp-sapi` and
`rhp-cli` forward the feature (`rhp-vm/native-pcre2 =
["rhp-builtins/native-pcre2"]`) so a single top-level flag selects the build.

### 4.2 One builtin, two backends, one public body

Keep the *builtin function* identical; switch only its engine via `cfg`. Using
`preg_match` as the worked example (today it calls `compile()` →
`fancy_regex::Regex` in `preg_fns.rs:170`):

```rust
// preg_fns.rs — a backend module boundary, selected at compile time.
#[cfg(not(feature = "native-pcre2"))]
#[path = "regex_backend/fancy.rs"]
mod regex_backend;
#[cfg(feature = "native-pcre2")]
#[path = "regex_backend/pcre2.rs"]
mod regex_backend;

// Both backends expose the SAME small internal API, so the PHP-wrapper code
// (delimiter parsing, flag mapping, $matches shaping, PREG_* handling) is
// written ONCE and is backend-agnostic:
//
//   fn compile(body: &[u8], opts: PcreOpts) -> Result<CompiledRe, CompileErr>;
//   impl CompiledRe {
//       fn find_at(&self, subj: &[u8], start: usize) -> Option<MatchSet>;
//   }
//
// MatchSet reports byte offsets + group spans in terms of the ORIGINAL bytes,
// so the array-building code never knows which engine produced it.
```

Key design rules this encodes:

- **The PHP wrapper lives above the backend split.** Delimiter parsing, modifier
  → option mapping, UTF-8 validation, `$matches`/`PREG_OFFSET_CAPTURE` shaping,
  and `preg_last_error` stay in shared code. Only the "compile a body + find"
  primitive is swapped. (This is the §2 lesson: linking the lib removes the
  algorithm, not the wrapper.)
- **FFI stays at the leaf.** The `pcre2.rs` backend is the *only* file that
  touches the `-sys` crate; it converts `&[u8]` ↔ PCRE2 buffers and returns
  plain Rust structs. No FFI type escapes into `Value` or the `Host` boundary.
- **`Host` is untouched.** Native backends need no new `Host` methods — they
  operate on owned `Value`s exactly like every other builtin. (If a native lib
  ever needs an engine service, e.g. INI like `pcre.backtrack_limit`, that
  would be a *new, defaulted* `Host` method, mirroring how `get_const` /
  `call_callable` were added in `lib.rs:61`/`:66` — never a breaking change.)
- **Byte-string discipline survives.** rhp strings are bytes
  (`Rc<BString>`). The backend API is `&[u8]`-in / byte-offsets-out, so we drop
  the `from_utf8_lossy` offset-shifting caveat that the fancy-regex backend
  documents (`preg_fns.rs:17-25`) — PCRE2 matches bytes natively. This is itself
  a correctness win, not only a feature win.

### 4.3 Keeping byte-exactness tested (the differential harness is the gate)

The harness in `crates/rhp-cli/tests/diff.rs` holds the VM strictly to
committed `.out` files captured from the reference php, over `tests/golden/*.php`. Make
the native backend subordinate to it:

1. **Same fixtures, both backends.** Golden `.out`s are captured from php once.
   Run the suite twice in CI: `cargo test` (pure-Rust) and
   `cargo test --features native-all`. The native build must pass the *same*
   `.out`s — that is the definition of "byte-exact with PHP".
2. **A backend-parity gate.** For functions with both backends, add a test that
   asserts fancy-vs-pcre2 agree on a corpus (and both agree with php). Where
   they *cannot* agree (the fancy-regex limitations list), the fixture is a
   "native-only" case: gate it `#[cfg(feature = "native-pcre2")]` so the
   pure-Rust build legitimately skips it rather than failing.
3. **Capture provenance.** Record, next to the goldens, the exact lib versions
   PHP used (PCRE, ICU, JSON — from `PCRE_VERSION`,
   `INTL_ICU_VERSION`; now `crates/rhp-builtins/src/version.rs`, checked by
   `tools/capture-ref-build.php`). A native build that links a *different* version is
   expected to diverge on version-sensitive fixtures; the harness turns that
   into a visible failure instead of silent drift.
4. **Fuzz the wrapper, not the lib.** Differentially fuzz `rhp preg_match`
   vs `php -r` on random patterns/subjects; the lib is PHP's own, so surviving
   diffs are wrapper bugs.

This gives the property the project wants: the pure-Rust build is always
correct-or-skips, and the native build is correct-or-fails — both measured
against real php output, never against each other alone.

---

## 5. `json_decode` — concrete recommendation

`json_decode` is **not implemented today** (only `json_encode`, in
`crates/rhp-builtins/src/misc_fns.rs:133`). So this is a greenfield decision.

**Recommendation: option (c) — reimplement in Rust** as a byte-oriented
recursive-descent decoder in `rhp-builtins`, no external crate, pinned by
the differential harness. Rationale: it is a few hundred lines, every PHP quirk
below is directly expressible, it keeps the pure-Rust / no-C-toolchain default,
and it matches how `json_encode`/`serialize`/`preg_*` were built. `serde_json`
is rejected because retrofitting PHP's big-int/array-key/error semantics onto it
is more fragile than a purpose-built decoder; vendoring `ext/json` C is rejected
for the zval-shim cost (§3a).

### PHP quirks a naive Rust crate would miss (all verified against php 8.5.8)

| Quirk | PHP 8.5.8 behavior (observed) |
|---|---|
| **assoc flag** | `$assoc=true` (or `JSON_OBJECT_AS_ARRAY` flag) → objects become arrays; default → `stdClass` with dynamic props. `json_decode('{"a":1}')` → `object(stdClass)`; with `true` → `array`. |
| **object key normalization** | When assoc, string keys that look like canonical ints are normalized to int keys: `json_decode('{"0":"a","1":"b"}', true)` → `[0=>"a", 1=>"b"]` (int keys, like all PHP arrays). A JSON-crate map keeps them as strings. |
| **big ints → float** | Integers that overflow `i64` become `float` by default: `12345678901234567890` → `float(1.23…E+19)`; `9223372036854775808` (INT_MAX+1) → float. `PHP_INT_MAX` itself stays `int`. |
| **`JSON_BIGINT_AS_STRING`** | Same overflow input → `string(20) "12345678901234567890"` instead of float. |
| **int vs float distinction** | `1` → `int(1)`; `1.0` → `float(1)`; `1e3` → `float(1000)`; `-0` → `float(-0)`; `-0.0` → `float(-0)`. A crate that yields one numeric type, or parses `-0` as int, diverges. |
| **depth limit (default 512)** | Counts *nesting*, and the top container counts: `json_decode('[1]', true, 1)` → `NULL` + error 1 "Maximum stack depth exceeded"; `json_decode('1', true, 1)` is fine (scalar). `json_decode('[[[1]]]', true, 2)` → error. Off-by-one vs most crates. |
| **trailing data** | `json_decode('1 2')` → `NULL`, error 4 "Syntax error". Leading/trailing *whitespace* is OK; trailing *tokens* are not. |
| **strict grammar** | No trailing commas (`[1,]` → NULL/syntax), no leading zeros (`01` → NULL/syntax), no comments, no single quotes, no bare/unquoted keys. Stricter than JSON5-ish crates. |
| **bare scalars** | Top-level `true`/`null`/`"x"`/`42` all decode (RFC 7159). Note `null` input is indistinguishable from failure by return value → **must** check `json_last_error()`. |
| **duplicate keys** | Last wins: `{"a":1,"a":2}` → `a=2`. (serde_json default also last-wins, but confirm.) |
| **UTF-8 validation** | Invalid UTF-8 in the *input* → `NULL`, error 5 "Malformed UTF-8 characters…". `"\xff\xfe"` fails. PHP validates the whole input is UTF-8. |
| **`\uXXXX` / surrogates** | Decodes `\u` escapes incl. surrogate pairs to UTF-8 bytes; lone/invalid surrogates are errors (code 5/6 depending). |
| **empty string** | `json_decode('')` → `NULL`, error 4 "Syntax error" (not error-free null). |
| **error codes** | `json_last_error()` / `json_last_error_msg()` return a specific code: `0` none, `1` max depth, `3` ctrl char, `4` syntax, `5` UTF-8, `6` UTF-16, `10` invalid property name. These are observable API. |
| **`JSON_THROW_ON_ERROR`** | Throws `JsonException` instead of returning `NULL` + setting last-error. Needs the exception prelude hookup. |
| **`json_last_error` is global state** | Set per-call, read later. Needs a place to live (see below). |

### Where `json_last_error` state lives

`json_last_error`/`_msg` read state set by the previous `json_*` call. rhp
builtins are stateless `fn`s, so this is the one piece that needs a home.
Cleanest fit: a **new defaulted `Host` method pair**
(`set_json_last_error(code)` / `json_last_error() -> i64`), mirroring the
existing `define_const`/`get_const` pattern (`lib.rs:55-63`). The VM stores a
single `i64` in request state. `JSON_THROW_ON_ERROR`
returns `Err(Thrown::Object(...))` built from the prelude's `JsonException`.

### Test plan

1. **Golden fixtures** under `tests/golden/json_decode_*.php`, each `.out`
   captured from php 8.5.8, covering every row above (one quirk per fixture, so
   failures are legible). Seed set = the probe script used to verify this table.
2. **Round-trip fixtures**: `json_decode(json_encode($x))` structural identity
   for the already-tested `json_encode` corpus.
3. **assoc × flags matrix**: each input × `{assoc true/false}` ×
   `{0, BIGINT_AS_STRING, OBJECT_AS_ARRAY, THROW_ON_ERROR}`.
4. **Error-path fixtures**: every malformed input asserts both the return
   (`NULL`/throw) *and* `json_last_error()` + `json_last_error_msg()`.
5. **Differential fuzz**: random JSON (and random bytes) vs `php -r
   'echo json_encode([json_decode($s, true), json_last_error()]);'`.

---

## 6. Ranked roadmap: native-backed vs stay-pure-Rust

Ranked by **(benefit of native backing) ÷ (cost + portability hit)**. "Benefit"
= how much byte-exactness or reimplementation is bought; "cost" = C toolchain,
version-pinning fragility, cross-compile pain.

### Tier 1 — native backing clearly wins (do these)

1. **Regex — PCRE2** (`pcre2` crate). *Highest value.* It is the one area where a
   pure-Rust engine (`fancy-regex`) is a *different regex language* and can
   never be byte-exact (possessive quantifiers, recursion, `\G`, ASCII `\d`,
   JIT backtrack limits). PCRE2 *is* what PHP runs. The §4.2 worked example is
   built for exactly this. **Caveat:** pin to PHP's PCRE2 version (`version.rs`);
   bundle it for reproducibility (§7). Keep fancy-regex as the no-C fallback for
   the common pattern subset.

2. **intl / collation / normalization — ICU** (`rust_icu_*` over system ICU).
   High value *because it is effectively impossible to match otherwise* —
   collation keys, locale casing, `Normalizer`, `IntlDateFormatter`,
   `MessageFormatter` all depend on ICU's data tables. **Heaviest caveat:**
   output is tied to the **ICU data version** PHP was built with, so even "ICU
   the API" diverges across majors. ICU4X (pure Rust) is tempting for the no-C
   build but ships *different* CLDR data → not byte-exact. Recommendation: ICU
   is opt-in only; without it, refuse `intl` (structural refusal, consistent
   with the project's posture) rather than approximate it.

### Tier 2 — native backing worth it for arbitrary-precision correctness

3. **gmp / bcmath — GMP (`rug`) / mpdecimal (bindgen).** `gmp` is unbounded
   integers — a pure-Rust bignum (`num-bigint`/`rug`'s Rust mode) can be
   byte-exact since the *numbers* are exact; the lib choice is about speed, not
   correctness, so this is lower-urgency. `bcmath` needs decimal semantics
   (scale, rounding) — `libmpdec` gives the arithmetic but PHP's scale/format
   wrapper is the real work; a careful Rust decimal impl can also be exact.
   **Verdict:** can stay pure-Rust for correctness; adopt `rug`/mpdecimal only
   if performance matters. Lower priority than Tier 1.

4. **Hashing / crypto — prefer pure-Rust (`sha2`, `md-5`, `ring`), OpenSSL only
   where required.** Hash *outputs* are standardized → pure-Rust crates are
   byte-exact and avoid the OpenSSL build dependency. Reach for `openssl` only
   for `openssl_*` (TLS, X.509, EVP ciphers) where reimplementation is
   unreasonable. So: hashing = stay pure-Rust (high value, no native need);
   `ext/openssl` = native OpenSSL, opt-in, if/when attempted.

### Tier 3 — stay pure-Rust (native backing buys little)

5. **json** (`json_decode`/`encode`) — **stay pure-Rust** (§5). No system lib
   exists; PHP's own C is zval-bound; a focused Rust decoder is byte-exact and
   keeps the default build C-free.

6. **date / time — stay pure-Rust** (`chrono` + `chrono-tz`, or hand-rolled).
   `DateTime`/`date()` is proprietary PHP logic over the IANA tz database, not a
   C lib. `chrono-tz` bundles the tz data (pin its release near PHP's). Native
   ICU would *change* behavior (ICU calendar), not match `date()`. No native win.

7. **mbstring core (encoding conversion, `mb_substr`, `mb_strlen`) —
   mostly pure-Rust.** The encoding tables are the work; `encoding_rs` covers the
   common ones. Only the *Unicode-property* pieces (`mb_convert_case` title-case,
   full casing) really want ICU — route those to the Tier-1 ICU backend when
   enabled, refuse/approximate otherwise. `mb_ereg*` specifically wants
   **Oniguruma** (`onig`), since it *is* oniguruma in PHP — treat like regex
   (opt-in native), but it is niche → low priority.

8. **libxml2 (dom/simplexml) — defer.** Large surface, and `quick-xml`/`roxmltree`
   are *different* parsers (error recovery, entity handling differ from
   libxml2). If strict parity is needed, bind libxml2 (feasible, Category 2);
   otherwise defer the whole DOM family.

9. **zlib — pure-Rust `flate2` (miniz) or `flate2` with zlib backend.** Deflate
   output can differ *byte-for-byte* across zlib/miniz at identical compression
   levels. If `gzencode` byte-parity is required, use `flate2`'s zlib-ng/zlib
   backend (native); otherwise pure-Rust is fine for decompression and for
   round-trip-only uses. Low priority.

**Summary ordering:** PCRE2 (now) → ICU (when `intl` is on the roadmap) →
leave json/date/hash/mbstring-core pure-Rust → gmp/bcmath/openssl/libxml2/zlib
opt-in only when a concrete parity requirement forces it.

---

## 7. Build & portability implications

Every Category-2 adoption adds a C dependency, so the architecture makes them
**opt-in** precisely to protect the default developer and CI experience.

- **Pure-Rust default stays toolchain-free.** `cargo build` with no features
  needs no C compiler, no system libs — today's state (only `fancy-regex`).
  This is the contract for `default = []` in §4.1 and must not regress.
- **C toolchain for native features.** `--features native-pcre2` etc. require a
  C compiler and, for system-lib `-sys` crates, the dev headers (`pkg-config`
  finds them). This is the cost that keeps these gated.
- **Bundled vs system libs — bundle for byte-exactness.** A *system* PCRE2/ICU
  is whatever the build host ships, which defeats version-pinning (§2 caveat).
  Prefer `-sys` crates in **vendored/bundled mode** (`pcre2-sys` builds a pinned
  PCRE2 from source; `icu`/`rug` can vendor) so the binary carries the *exact*
  version matched to the goldens. Trade-off: longer builds, larger binary. For
  ICU specifically, bundling the **data** is the hard part (tens of MB) — this
  is a real reason ICU stays opt-in.
- **Cross-compilation.** Pure-Rust cross-compiles trivially. Each native lib
  adds a cross-toolchain + target build of that C lib (ICU cross-builds are
  notoriously painful). Vendored `-sys` crates help (they build the lib in the
  target's cc), but this is the strongest practical argument for keeping native
  backends optional and the pure-Rust path first-class.
- **Licensing.** PCRE2 (BSD), ICU (ICU/Unicode license), zlib (zlib) are
  permissive and compatible with rhp's `MIT OR Apache-2.0`. GMP is
  **LGPL/GPL dual** — `rug`'s GMP backend pulls LGPL; relevant if rhp ever
  ships static binaries. Vendoring php-src's `ext/json` C would carry the PHP
  license. Note each in the opt-in docs.
- **Future Cranelift JIT.** Native backends are a *non-issue* for the planned
  JIT. The JIT lowers rhp's own ISA (opcodes that delegate to
  `ops`/`conv`/`registry()`); a builtin call remains a call into the registry
  regardless of whether that builtin's body is pure-Rust or an FFI leaf. The JIT
  never needs to inline through FFI — it emits a call to the same `NativeFn`
  pointer the VM uses. The one rule: **keep FFI at the leaf** (§4.2) so the
  JIT-visible surface is pure-Rust calling conventions, and the C library is an
  ordinary linked dependency the JIT'd code calls like any other function. This
  is another reason FFI types must never leak into `Value` or the ISA.

---

## 8. Recommended next steps (actionable)

1. **Implement `json_decode` in pure Rust** (§5) — it is a live gap, greenfield,
   high-value, and needs no new dependency. Add the `Host` last-error hook and
   the golden/fuzz suite. *(Do first; unblocks real programs.)*
2. **Introduce the feature-gate scaffolding** (§4.1) with `native-pcre2` as the
   first flag, and refactor `preg_fns.rs` to the backend-split shape (§4.2) with
   `fancy-regex` as `default` and a `pcre2` backend behind the flag. Run the
   golden suite under both in CI.
3. **Add provenance capture** (PCRE/ICU/JSON versions) next to the goldens and a
   native-vs-pure parity test, so divergence is a visible CI failure.
4. **Defer ICU/intl** until there is a concrete `intl` requirement; when it
   comes, adopt it strictly opt-in with bundled data, and *refuse* intl in the
   pure-Rust build rather than approximate.
5. **Keep date, hashing, mbstring-core, and json pure-Rust**; adopt
   gmp/bcmath/openssl/libxml2/zlib natives only against a specific parity need.

---

## 9. Category 1b — compile ext *source* against a macro shim (PROTOTYPED)

§1 ruled out loading a compiled `.so` (it is an ABI client of the Zend engine).
This section is a different idea, now **prototyped and measured**: do *not* load
the `.so` — recompile the extension's `.c` sources against **our own
redefinitions** of the Zend API macros/types, bridged to rhp's `Value` model.
The lever is that ext `.c` almost never touches `zval`/HashTable/`zend_string`
*fields*; it goes through *macros* (`PHP_FUNCTION`, `ZEND_PARSE_PARAMETERS_*`,
`Z_PARAM_*`, `ZVAL_*`, `RETURN_*`, `add_assoc_*`/`add_next_index_*`) and a few
inline helpers. If we supply those macros, the ext's real algorithm compiles
against our definitions and runs against rhp.

Prototype crate: **`crates/rhp-zendshim/`** (feature `zendshim`, off by
default). It is the only crate that carries a C toolchain dependency, and nothing
depends on it unless the feature is turned on, so the pure-Rust default build is
byte-for-byte unaffected (verified: `cargo build`/`cargo test` unchanged, 186
tests green).

### 9.1 What was built (byte-exact evidence)

**Stage 1 — mechanism proven (`csrc/demo_ext.c`).** A hand-written function using
the *real* Zend macro spellings:

```c
PHP_FUNCTION(demo_sum_and_label) {
    zend_long a, b; char *label; size_t label_len;
    ZEND_PARSE_PARAMETERS_START(3, 3)
        Z_PARAM_LONG(a) Z_PARAM_LONG(b) Z_PARAM_STRING(label, label_len)
    ZEND_PARSE_PARAMETERS_END();
    array_init(return_value);
    add_assoc_long(return_value, "sum", a + b);
    add_assoc_string(return_value, "label", label);
    add_next_index_long(return_value, a);
}
```

Compiled against `include/zend_shim.h`, exposed as builtin `__zendshim_demo`,
`var_dump`'d from a `.php` script, and the returned array (`['sum'=>5,
'label'=>'hello', 0=>2]`, including key normalization + insertion order + the
`[]`-append landing at key 0) is **byte-identical** to the equivalent php 8.5.8.
This proves params-in, array building, strings, and value-out all cross the shim
correctly.

**Stage 2 — real php-src source (`csrc/php_bin2hex.c`).** The `hexconvtab` table,
`php_bin2hex()`, and `PHP_FUNCTION(bin2hex)` are copied **verbatim** from
`php-8.5.8 ext/standard/string.c` (lines 55, 62-77, 158-171); the only change is
`#include "zend_shim.h"` instead of php's headers. Exposed as `__zendshim_bin2hex`
and **byte-identical** to php for `"abc"`, `""`, `"Hello, World!"`, and binary
input `"\x00\x01\xff\xfe"` → `"0001fffe"` (binary-safe). The macros/helpers it
pulled in: `ZEND_SET_ALIGNED`, `zend_string`, `zend_string_safe_alloc`,
`ZSTR_VAL`/`ZSTR_LEN`, `ZEND_PARSE_PARAMETERS_*`, `Z_PARAM_STR`, `PHP_FUNCTION`,
`RETURN_STR` — all provided by the shim, none requiring the real engine.

### 9.2 The macro/type surface implemented in `zend_shim.h`

- **Types:** `zend_long`, `zend_ulong`, `zend_bool`; `zend_string` (flexible-array
  byte string, NUL-terminated, accessed only via `ZSTR_VAL`/`ZSTR_LEN`); `zval`
  (`{ zend_value value; uint8_t u1_type; }`) with the real `IS_*` tag numbers;
  `zend_execute_data` (`{ argc; zval *argv; }`); `INTERNAL_FUNCTION_PARAMETERS` /
  `return_value`.
- **Dispatch:** `PHP_FUNCTION`/`ZEND_FUNCTION`/`ZEND_NAMED_FUNCTION` →
  `void zif_<name>(zend_execute_data*, zval*)`.
- **ZPP:** `ZEND_PARSE_PARAMETERS_START/_END`, `Z_PARAM_LONG/DOUBLE/BOOL/STRING/
  STR/OPTIONAL`. (Z_PARAM macros expand to brace blocks, not `do/while`, so they
  juxtapose with no separators exactly as ext source writes them.)
- **Accessors/setters:** `Z_TYPE_P`, `Z_LVAL_P`, `Z_DVAL_P`, `Z_STR*_P`,
  `Z_ARRVAL_P`; `ZVAL_LONG/DOUBLE/NULL/TRUE/FALSE/BOOL/STR/STRING/STRINGL`;
  `RETVAL_*`/`RETURN_*`.
- **Arrays:** `array_init`, `add_assoc_long/double/bool/null/string/stringl/zval`,
  `add_index_long`, `add_next_index_long/double/string/stringl/zval`.
- **Memory:** `emalloc/ecalloc/erealloc/efree/estrdup/estrndup/safe_emalloc` →
  a tracked-malloc per-call arena; `zend_string_init/alloc/safe_alloc`, and
  `zend_string_release*`/`_free` as arena no-ops.
- **Exceptions:** `zend_throw_exception`/`zend_value_error` → a thread-local
  pending error the Rust boundary converts to `Thrown`.

### 9.3 Build mechanics

- `crates/rhp-zendshim/build.rs` uses the **`cc` crate** (already in the tree
  via `pcre2-sys`) to compile `csrc/*.c` with `-I include` into a static archive
  linked into the Rust crate. Stage 2's vendored file is detected at build time
  and gated by a `--cfg zendshim_bin2hex` (declared with `rustc-check-cfg`, so
  zero warnings).
- **HashTable decision: option B (callback into Rust).** `array_init` calls
  `rs_array_new()` which pushes a `PhpArray` into a thread-local registry and
  returns a 1-based `u32` handle stored in the `zval`. Every `add_assoc_*`/
  `add_next_index_*` forwards the handle to a Rust callback (`rs_array_set_*`/
  `rs_array_push_*`) that mutates that `PhpArray`. This **reuses rhp's exact
  key normalization, insertion ordering, and next-index logic** instead of
  reimplementing an ordered map + canonical-int-string rule in C. Nested arrays
  are moved out of the child slot into the parent at the boundary.
- **The bridge** (`src/lib.rs`) per call: resets the arena + registry, marshals
  `&[Value]` → shim `zval` argv (strings via the C allocator), invokes `zif_*`
  with a synthetic `execute_data`, checks the thread-local exception, then
  marshals `return_value` back to an owned `Value` and frees the arena. All state
  is thread-local.
- **Feature gating:** `rhp-builtins/zendshim` → `rhp-zendshim`; forwarded
  up through `rhp-vm`, `rhp-sapi` and `rhp-cli`. Builtins register as
  `__zendshim_demo` / `__zendshim_bin2hex` (prefixed, so they never shadow the
  pure-Rust builtins and can be diffed against them).

### 9.4 Stage 3 — `json_decode`: SCOPED, not completed

Attempting `ext/json` mapped the surface precisely (php-8.5.8 `ext/json`,
pre-generated `json_scanner.c` + `json_parser.tab.c`, no re2c/bison needed):

- **Feasible and would be byte-exact for free:** the **scanner**
  (`json_scanner.re`) is nearly self-contained — it needs `ZVAL_*`, `ZEND_STRTOL`,
  `zend_strtod`, `zend_string_alloc`, `ZSTR_VAL`, `memcpy`. Compiling it (plus
  `Zend/zend_strtod.c`, David Gay's dtoa, which has no engine deps) buys
  **exact PHP number parsing** — the int-vs-float boundary, `INT_MAX+1`→float,
  `JSON_BIGINT_AS_STRING`, and `-0.0` handling that §5 lists as the hard quirks —
  *without reimplementing any of it*. The **parser** (bison) is a token state
  machine that calls a methods table.
- **Feasible with the handle model: the `assoc=true` path.** The default parser
  callbacks (`json_parser.y`) are `array_init` + `zend_hash_next_index_insert` +
  `zend_symtable_update` — all expressible over our array handle (add
  `rs_array_*_zval` + a `zend_hash`/`zend_symtable` shim over the handle).
- **The real blocker: default (object) mode.** With `assoc=false`, `object_init`
  mints a **`stdClass`** and `zend_symtable_update`/property writes populate it.
  The current bridge is `Host`-less and cannot mint an object with the engine's
  `stdClass` class id. Completing json_decode therefore requires threading
  `&mut dyn Host` through the bridge and adding `Host` hooks to create a
  `stdClass` + set properties — a change that touches the engine, not just
  `Value`.
- **Other surface:** `JSON_G(error_code)` module globals (thread-local shim),
  `php_json_exception_ce` + `zend_throw_exception` for `JSON_THROW_ON_ERROR`
  (→ `Thrown`), `php_json_get_error_msg` (static table, compiles as-is), and
  `php.h`/`php_json*.h` wrapper headers. `json_arginfo.h`/module registration is
  avoidable by calling `php_json_decode_ex` directly.

**Stage 3 verdict:** the shim path *would* deliver byte-exact json_decode and is
the cheapest route to exact number parsing, but a correct, committed-quality
implementation is a multi-file effort (scanner + parser + `zend_strtod` + a
HashTable/object bridge + globals) that also requires the engine-touching object
hook. It was out of scope for a single session held to byte-exactness at every
step, so it is **documented, not shipped**. For json specifically, §5's pure-Rust
recommendation still stands as the near-term call; the shim becomes attractive
only if *several* number-heavy ext functions are wanted at once (then
`zend_strtod` + the scanner amortize).

### 9.5 Hazards found

- **Memory/refcount model.** The shim maps `emalloc`→tracked malloc and
  `efree`/`zend_string_release`→no-ops, freeing everything in one per-call arena
  sweep. This is correct for leaf functions whose allocations live only during
  the call (demo, bin2hex, the json scanner). It is **wrong for any ext that
  caches across calls, stores into persistent/`pemalloc` memory, or relies on
  refcount side effects** (COW separation, `Z_TRY_ADDREF`). `zshim_realloc`
  copies into a fresh block and leaves the old one for the sweep (bounded
  over-read); fine for prototype scope, not for a realloc-heavy loop.
- **HashTable ordering / keys.** Delegating to `PhpArray` is a feature, not a
  risk — it *is* php's ordering. The risk is the inverse: any ext that reads a
  HashTable it was handed (iteration, `zend_hash_find`) needs a far larger
  `zend_hash_*` shim over the handle than the write-only set used so far.
- **Exceptions.** Modeled as a thread-local flag + message → `Thrown`. Real ZPP
  throws precise `TypeError`/`ValueError` with argument-numbered messages; the
  shim's are approximate, so an ext whose *observable behavior includes the exact
  exception message* would diverge until those are reproduced.
- **Thread-local globals (`JSON_G`, `PG`, `EG`, `CG`).** Any ext reading engine
  globals or its own `ZEND_DECLARE_MODULE_GLOBALS` needs each shimmed as
  thread-local state and seeded with php's defaults — easy to under-populate.
- **Generated-code dependencies.** Using release tarball `.tab.c`/`scanner.c`
  avoids bundling bison/re2c, but pins the prototype to a php version's generated
  output; a from-`.y`/`.re` build would re-add those tools.
- **ABI / struct layout.** Because we recompile, layout is *ours* — the Rust
  mirror structs (`Zval`, `ZendExecuteData`) just have to match `zend_shim.h`,
  not real php. This sidesteps §1's fatal ABI problem entirely. The cost moves
  from "match php's binary layout" to "cover php's macro surface."
- **Portability.** Same as any Category-2 native backend: the feature needs a C
  toolchain; the pure-Rust default does not. No system libs are required (the C
  is self-contained), so this is lighter than linking pcre2/ICU.

### 9.6 Verdict: is the macro-shim viable as rhp's extension strategy?

**As a general strategy: no. As a targeted tool: situationally yes.**

- **vs (a) linking system C libs (§2):** different problem. System-lib linking
  wins where php is a thin wrapper over a *stable neutral C ABI* (pcre2, ICU,
  gmp) — no macro surface to reproduce, just `&[u8]` at the edge. The macro shim
  is for php's *own* C that is **not** a library (`ext/json`, `ext/standard`),
  where linking does not apply. They are complementary, not competing.
- **vs (b) pure-Rust reimplementation (§3c):** the shim's genuine edge is
  **inheriting proprietary numeric/parsing exactness for free** — `zend_strtod`,
  the json int/float boundary, `php_bin2hex`'s byte handling — the very things a
  Rust reimpl must chase against the differential harness. Its cost is a C
  toolchain, an open-ended macro surface that grows with each new ext, a memory
  model that is only safe for leaf/arena-shaped functions, and engine-touching
  hooks the moment objects/globals appear (Stage 3). For a *small, self-contained,
  numeric* function the shim is a fast, exact win; for anything that allocates
  across calls, manipulates objects, iterates HashTables, or reads globals, the
  surface balloons and pure-Rust (pinned by the harness) stays the more robust,
  toolchain-free, memory-safe default — consistent with the rest of this doc.

> **Superseded (2026-10-24).** The shim is no longer pursued; `crates/rhp-zendshim`
> has been removed. See §10 for the strategy that replaced it.

---

## 10. The chosen approach: reimplement-in-Rust + differential testing

The adopted extension strategy is deliberately simple and hard to fool:

1. **Reimplement the extension in Rust** — pure-Rust crates where a faithful 1:1
   exists, hand-rolled otherwise; a C system library (e.g. `libmysqlclient`,
   `libpq`) only where no 1:1 Rust alternative exists.
2. **Prove it against the C engine.** Run the *same* PHP source through the real
   `php` binary (the canonical C implementation, our oracle) and through rhp's
   VM, and require identical stdout — not against a hand-committed `.out`, but
   against php live, over a broad, generated, seed-reproducible corpus.

### 10.1 The framework — `crates/rhp-difftest`

A small crate with a library, a binary, and integration tests:

- `run_php(src)` feeds the source to `php -n` on stdin (no temp files, fixed UTC
  timezone) and captures stdout; `run_rhp(src)` compiles and runs it on the VM
  (catching engine panics as diffs). `diff_source(src)` runs both and classifies
  the result `Match | Mismatch | PhpErr | PhrustErr`.
- A dependency-free seeded `Rng` and `default_byte_corpus()` generate the inputs
  where bugs hide: the empty string, every block-boundary length
  (55/56/63/64/65/119/120/127/128…), human-readable vectors, and random binary
  strings. A failing run reproduces exactly from its seed.
- `sweeps.rs` turns an extension's functions into per-feature diffs (today: one
  `hash()` sweep per algorithm from php's own `hash_algos()`).
- The `rhp-difftest` binary prints a per-feature MATCH/MISMATCH verdict
  (`rhp-difftest hash [--seed N] [--count N] [--algo NAME] [--strict]`); it is
  the dev loop. `tests/hash.rs` is the CI gate — it skips cleanly when no `php`
  is on PATH, so the workspace still tests without PHP installed.

This is reusable for any future extension: generate inputs → `diff_source` →
classify.

### 10.2 First extension: `hash`

`hash` is ideal to prove the framework — pure computation, ~60 algorithms,
airtight to diff. All of `hash()`, `hash_hmac()`, `hash_pbkdf2()`,
`hash_equals()`, `hash_algos()`, `hash_hmac_algos()` are byte-identical to
php 8.5.8 over the sweep.

- **Pure-Rust crates (1:1):** RustCrypto `md2`, `md4`, `sha2` (sha224/256/384/512
  and the sha512/224, sha512/256 truncations), `sha3`, `ripemd`,
  `whirlpool`, `tiger` (3-pass), `gost94` (gost + gost-crypto); `xxhash-rust`
  (xxh32/64/3/128). `md5`/`sha1`/`sha256`/`crc32b` stay on rhp's existing
  hand-rolled primitives.
- **Hand-rolled (no 1:1 crate):** adler32; the CRC variants; FNV-1/1a (32/64);
  joaat; MurmurHash3 (x86_32, x86_128, x64_128); and the table-heavy
  `hash_tiger.rs` (4-pass tiger), `hash_snefru.rs` (Snefru-256),
  `hash_haval.rs` (HAVAL 3/4/5-pass × 128–256-bit).
- **Quirks the differential sweep surfaced** (each invisible to spec reading, each
  caught instantly by diffing php): PHP emits the non-reflected `crc32`
  **little-endian** while `crc32b`/`crc32c` are big-endian; `snefru` and
  `snefru256` are the *same* 256-bit digest; murmur/xxh/fnv/joaat outputs are
  big-endian; the HMAC block sizes for the sha3 family follow the keccak rates
  (144/136/104/72).

**Recommendation:** keep `rhp-zendshim` as an **opt-in, feature-gated
capability** for the narrow case it fits (small self-contained ext functions,
especially numeric ones where `zend_strtod`/scanner exactness is the prize), hold
it to the differential harness like every native backend, and keep pure-Rust the
default. Do **not** make it the primary extension mechanism.
