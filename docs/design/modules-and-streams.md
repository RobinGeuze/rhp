# Modules & stream wrappers (plan toward running Composer)

**Status:** IMPLEMENTED (Parts A/B/C + M1). `rhp composer.phar --version` now
produces byte-identical stdout to php 8.5.8. See the commit "run composer.phar"
and `crates/rhp-cli/tests/phar.rs`. Resource streams (plan Stage 2) also landed,
since composer's output needs `fwrite` to `php://stdout`. The notes below are the
original plan, kept for reference; broader composer commands (`list`/`about`)
need Reflection next (M2).
**Goal:** (1) turn each extension into a self-contained *module* that can register
functions **and** classes, constants, and **stream wrappers**; (2) add a stream
layer so file I/O and `include`/`require` dispatch by URL scheme; (3) implement
`phar://` + `Phar` on top of it, far enough to **execute `composer.phar`**.
**Ground truth:** the reference `php` (see `crates/rhp-builtins/src/version.rs`; has phar+zlib+zip+bz2). Everything below was
verified against a real `composer.phar` (native format, 831 files, **all stored
uncompressed**, **SHA-512** signed, API 1.1.0) and the current codebase.

---

## 0. TL;DR

- **Milestone M1 — run `composer.phar` (read-only).** Critical path is tiny:
  native-phar parse + SHA-512 verify + a **read-only `phar://` wrapper** feeding
  `include`/`require`/`file_get_contents` + `Phar::mapPhar()`. **No zlib, no zip,
  no tar, no resources** needed — composer's archive is 100% stored and the
  parser already stops at `__halt_compiler` (verified).
- **Milestone M2 — `composer install` of real packages.** Adds `ZipArchive`
  (zip dist), `gzopen`/tar (tgz dist), and resource-backed streams + networking.
- The **module abstraction** and the **stream-wrapper layer** are the two pieces
  of reusable architecture; Phar/zlib/zip are their first consumers.
- Favorable facts already in place: `sha2` is a dep (hash ext); `__halt_compiler`
  works; `rhp-compiler` has no dep on `rhp-builtins` yet but one would be acyclic.

---

## 1. Part A — the module abstraction

### 1.1 What exists (the shortfall)

Today a "module" is a single `register(&mut HashMap<&'static str, Builtin>)` that
inserts **functions only**. Everything else is centralized away from the module:

- Classes live in the compiler's PHP-source `PRELUDE` const (`rhp-compiler`),
  incl. GMP/DateTime. There is **no native-class path**; ext classes are PHP
  source whose methods delegate to hidden `__dt_*`/gmp native builtins.
- Constants are a hand-written `vec![]` in `builtins::constants()`; per-ext
  constants (`GMP_*`, `PASSWORD_*`) are inlined there (only preg/json use a
  chained `*_constants()` fn).
- The builtin registry is a **process-wide `OnceLock`**, built once, shared by
  all `Vm`s.
- No stream/resource concept anywhere.

### 1.2 Proposed: a `ModuleDef` descriptor (in `rhp-builtins`)

One declarative descriptor per module; a central list replaces the scattered
`register`/`constants`/`PRELUDE` wiring.

```rust
// rhp-builtins/src/module.rs
pub struct ModuleDef {
    pub name: &'static str,
    /// Native functions this module provides.
    pub functions: fn(&mut HashMap<&'static str, Builtin>),
    /// (name, value) engine constants.
    pub constants: fn() -> Vec<(&'static str, Value)>,
    /// PHP-source class/interface declarations (compiled into the prelude).
    /// "" when the module declares no classes. Methods may delegate to the
    /// module's own native `__<ext>_*` builtins (the existing GMP/DateTime idiom).
    pub prelude_php: &'static str,
    /// Built-in stream wrappers this module installs at VM start.
    pub wrappers: fn() -> Vec<WrapperReg>, // WrapperReg { scheme, factory }
}

pub fn modules() -> &'static [ModuleDef]; // core *_fns groups + ext/* modules
```

Derived, each from the single `modules()` list:

- `registry()` folds every `functions` in (unchanged `OnceLock`, still process-wide).
- `constants()` chains every `constants`.
- **`module_prelude() -> String`** concatenates every `prelude_php`. The compiler
  pulls this instead of hard-coding ext classes in `PRELUDE`.
- **`wrappers()`** gives the VM the built-in wrappers to register at `Vm::new`.

### 1.3 Where the pieces land (dependency direction)

- Functions / constants / wrappers reference `Value`, `Builtin`, `Host`,
  `StreamWrapper` — all in `rhp-builtins`. Keep the module system there.
- `prelude_php` is just `&'static str`. Give the compiler the class source by
  **adding `rhp-compiler → rhp-builtins`** (acyclic: builtins does not depend on
  the compiler) and calling `rhp_builtins::module_prelude()`. The engine's core
  classes (`Throwable`, `stdClass`, exception tree, …) stay in the compiler's own
  `PRELUDE`; only *ext* classes move into their modules' `prelude_php`.
  - *Alternative if we want to avoid the dep:* keep ext class source in the
    compiler and have modules contribute only functions/constants/wrappers. Less
    tidy (classes split from their module) but zero new deps. **Recommendation:
    add the dep** — it makes a module genuinely self-contained, which is the ask.

### 1.4 Migration

Mechanical, no behavior change: wrap each existing `*_fns`/`ext/*` as a
`ModuleDef`; move `GMP_*`/`PASSWORD_*` constants next to their module; move
GMP/DateTime class source into their modules' `prelude_php`. Done incrementally
behind the unchanged `registry()`/`constants()` facades, so nothing observable
changes — guarded by the full differential suite staying green.

---

## 2. Part B — the stream layer

### 2.1 Scheme model

A path either names a wrapper via a `scheme://` prefix (`scheme` =
`[A-Za-z][A-Za-z0-9+.\-]*`) or, with no scheme, uses the **default `file://`
wrapper**. One helper:

```rust
fn split_scheme(path: &[u8]) -> (&[u8] /*scheme, "file" default*/, &[u8] /*rest*/);
```

### 2.2 `StreamWrapper` trait (native wrappers)

Staged so M1 needs only the path-based half (no resources):

```rust
pub trait StreamWrapper: std::any::Any {
    // --- Stage 1: path-based (enough for include/require + file_get_contents + stat)
    fn exists(&self, url: &[u8]) -> bool;
    fn get_contents(&self, url: &[u8]) -> Option<Vec<u8>>;      // whole-file read
    fn stat(&self, url: &[u8]) -> Option<StatInfo>;            // is_file/is_dir/size/mtime
    fn put_contents(&self, url: &[u8], data: &[u8]) -> Option<usize> { None } // writable only
    // --- Stage 2: resource-backed (fopen family) — deferred
    // fn fopen(&self, url, mode) -> Option<Box<dyn StreamHandle>>;
    // --- Stage 2+: directories — deferred
    // fn opendir(&self, url) -> Option<Box<dyn DirHandle>>;
    fn as_any(&self) -> &dyn std::any::Any;  // for wrapper-specific ops (e.g. Phar::mapPhar)
}
```

### 2.3 Per-VM registry + `Host` additions

The active wrapper set is **runtime-mutable** (`stream_wrapper_register`), so it
lives on the `Vm`, not in the static registry:

```rust
// on Vm
wrappers: HashMap<Vec<u8>, Rc<dyn StreamWrapper>>, // seeded at new() from modules::wrappers() + file://
```

Builtins reach it through **new `Host` methods** (keeps builtins VM-agnostic, as
the current `Host` design intends):

```rust
fn stream_exists(&self, url: &[u8]) -> bool;
fn stream_get_contents(&self, url: &[u8]) -> Option<Vec<u8>>;
fn stream_stat(&self, url: &[u8]) -> Option<StatInfo>;
fn stream_put_contents(&mut self, url: &[u8], data: &[u8]) -> Option<usize>;
fn stream_wrapper(&self, scheme: &[u8]) -> Option<Rc<dyn StreamWrapper>>; // for Any-downcast ops
fn running_main_path(&self) -> Option<Vec<u8>>; // for Phar::mapPhar (the executing script)
// Stage 3 (userspace wrappers): stream_wrapper_register(scheme, class), _unregister, get_wrappers
```

### 2.4 Routing existing I/O through the layer

- **`file_fns`**: `file_exists`/`is_file`/`is_dir`/`file_get_contents`/
  `file_put_contents` call `host.stream_*` instead of `std::fs` directly. `file://`
  wrapper keeps today's `std::fs` behavior, so non-phar paths are unchanged.
  (Also bump `file_get_contents` max_args so it can take the `$context` arg later.)
- **`include`/`require`**: the single disk-read hook is
  `Vm::resolve_include_path` (vm `lib.rs`). Change it to: if the path has a
  non-`file` scheme → `wrapper.get_contents(url)` (phar entries are absolute
  within the archive, no candidate search); else keep the current
  candidate-resolution (relative to including file, then CWD) over the `file://`
  wrapper. Returns `(canonical_key, source_bytes)` as today.

### 2.5 Resources — deferred to Stage 2

M1 needs **no user-visible resource**: `require 'phar://…'` and
`file_get_contents('phar://…')` are internal/string-valued. Stage 2 adds
`Value::Resource(Rc<RefCell<ResourceData>>)` (a new `Value` variant — note the
blast radius across `conv`/`fmt`/`dump`/`ops`/oracle) plus `fopen`/`fread`/
`fwrite`/`fclose`/`gzopen`, giving `var_dump` its `resource(N) of type (stream)`.
Flagged now so the trait is shaped for it, but **out of scope for M1**.

---

## 3. Part C — Phar (the M1 payload)

### 3.1 Native format (verified against composer.phar)

`[stub] [manifest] [file data] [signature]`, little-endian except the 2-byte API
version (3 big-endian nibbles).

- **Stub**: PHP source ending `__HALT_COMPILER(); ?>` + newline; manifest begins
  right after. Scan for `__HALT_COMPILER` (our parser already stops there).
- **Global manifest**: `len(4)` · `num_files(4)` · `api_ver(2)` · `flags(4)` ·
  `alias_len(4)` · `alias` · `global_meta_len(4)` · `global_meta`
  (`serialize()`). File data starts at `offset(num_files field) + manifest_len`.
- **Per-file entry** ×N: `name_len(4)` · `name` · `usize(4)` · `mtime(4)` ·
  `csize(4)` · `crc32(4)` · `flags(4)` · `meta_len(4)` · `meta`. Data blocks
  follow in order, `csize` bytes each, no inter-block padding.
- **Per-file flags**: perms `0x1FF`; `0x1000` = zlib **raw DEFLATE**; `0x2000` =
  bzip2; mask `0x3000`; none set = stored.
- **Signature trailer** (global flag `0x10000`): `[hash][type(4,LE)][GBMB]`.
  Types: MD5 `1`/16, SHA1 `2`/20, SHA256 `3`/32, SHA512 `4`/64, OpenSSL `0x10`.
  Hash is over `file[0 .. size-(hashlen+8)]`. composer = SHA-512 (verified
  byte-for-byte). PHP's `phar.require_hash=On` ⇒ verify or refuse.

### 3.2 `phar://` wrapper

A `PharWrapper { archives: RefCell<HashMap<alias_or_path, Rc<Phar>>> }` in the VM
registry under scheme `phar`. `phar://<archive>/<entry>`: resolve `<archive>` by
registered alias (from `mapPhar`) or by the filesystem path up to the deepest
`.phar`; look `<entry>` up in the manifest; return stored bytes (raw-inflate if
`0x1000`, via `flate2` — composer needs none). Implements Stage-1
`get_contents`/`exists`/`stat`.

### 3.3 `Phar` class + `mapPhar`

Follow the established idiom: `Phar` is a **PHP-source prelude class** in the phar
module's `prelude_php`, whose static methods delegate to native `__phar_*`
builtins. For M1 only one method matters:

```php
final class Phar /* extends ... */ {
    const TAR=1, ZIP=2, GZ=0x1000, BZ2=0x2000, MD5=1, SHA1=2, SHA256=3, SHA512=4; // ints so source parses
    public static function mapPhar($alias = null, $dataoffset = 0) { return __phar_mapphar($alias); }
    public static function running($retphar = true) { return __phar_running($retphar); } // self-update only
    public static function createDefaultStub(...) { /* deferred */ }
}
```

`__phar_mapphar($alias)`: via `host.running_main_path()` read the executing
script, parse+verify it, and register the archive under `$alias` in the
`PharWrapper` (reached by `host.stream_wrapper(b"phar")`→`as_any().downcast`).
Then the stub's `require 'phar://composer.phar/bin/composer'` resolves through the
wrapper. `Phar::running()` is used only by composer's self-update — stub it.

### 3.4 CLI

`rhp composer.phar` already works mechanically: the CLI reads the file, the parser
runs the stub and **stops at `__halt_compiler`** (verified), execution hits
`mapPhar` then the `require`. The CLI must expose the main script's path/bytes to
the VM for `running_main_path()` (it already canonicalizes + reads the script).

---

## 4. Part D — zlib & zip modules

- **zlib (do now; cheap, requested):** a `zlib` module with the six pure
  string↔string functions via `flate2` (pure-Rust `miniz_oxide` backend):
  `gzdeflate`/`gzinflate` (raw DEFLATE), `gzcompress`/`gzuncompress` (zlib
  RFC1950), `gzencode`/`gzdecode` (gzip RFC1952), plus `zlib_encode`/`_decode`
  and `ZLIB_*` constants. Not on composer's bootstrap path but self-contained and
  a good first exercise of a module owning its own constants. `gzopen` family and
  `compress.zlib://` wrapper are resource-based ⇒ Stage 2.
- **zip (defer to M2):** a `zip` module; `ZipArchive` as a prelude class
  delegating to `__zip_*` builtins over the pure-Rust `zip` crate
  (`default-features=false, features=["deflate-flate2"]`). Open archives held in
  per-VM state keyed by an int handle (same pattern as phar archives), until real
  resources land. Composer only needs it to *install* zip-dist packages; a subset
  (`open`/`getFromName`/`statIndex`/`extractTo`/`addFile`/`close`) suffices.
  `class_exists('ZipArchive')` gating means composer still *runs* without it.

---

## 5. Crates to add

| Crate | Version | Features | When |
|---|---|---|---|
| `flate2` | `1.1` | default (`rust_backend`/miniz_oxide, pure Rust) | zlib module (now); compressed-phar (M1 opt) |
| `zip` | `9` | `default-features=false`, `["deflate-flate2"]` | M2 (installs) |
| `tar` | `0.4` | default (pure Rust) | M2 (tgz dist) |
| `bzip2` | `0.6` | default (pure-Rust backend) | later (bz2 phar/dist) |

`sha2` already present (phar signature). Avoid `zip`/`flate2` C-backed features.

---

## 6. Order of work & testing

1. **Module abstraction** (`ModuleDef` + `modules()`; migrate existing ext/fns;
   compiler pulls `module_prelude()`). Pure refactor — suite stays green.
2. **Stream layer** (scheme split, `StreamWrapper`, per-VM registry, `Host`
   stream methods, `file://` wrapper, route `file_fns` + `resolve_include_path`).
   Differential test: file I/O + include over plain paths unchanged.
3. **Phar M1** (native parser + SHA-512, `phar://` read wrapper, `Phar`
   prelude class + `__phar_mapphar`, `running_main_path`). **Acceptance: `rhp
   composer.phar --version` / `list` matches `php composer.phar …` byte-for-byte.**
4. **zlib module** (6 fns + constants) — independent, any time.
5. **M2**: resources (`Value::Resource`, fopen/fread/gzopen), `ZipArchive`, tar.

**Test strategy:** behaviour that one self-contained script can show
(namespaces, streams, types) belongs in a **golden fixture**:
`tests/golden/NAME.php` plus php's output as `NAME.out`, which the VM test
holds byte for byte without needing php. Use a **CLI differential test** (real
`rhp` vs `php`, like `tests/include.rs` / `tests/phar.rs`) only when a case
needs more than one script's stdout: several files in a temp dir, an
argv/stdin/exit-code contract, or a phar built in-test with
`php -d phar.readonly=0`; gate on a real `composer.phar` when present (skip
when absent / no `php`). (This section originally sent namespace and stream
tests to the CLI suite because goldens also ran through the tree-walking
oracle, which lacked both; the oracle has since been removed.)

---

## 7. Risks / open questions

- **`Value::Resource`** is a cross-cutting addition (every `Value` match). M1
  avoids it; M2 must budget for the blast radius (or model resources as objects
  of a prelude class — less faithful to `var_dump`/`get_resource_type`).
- **`serialize()` metadata**: composer has none (global + per-file meta absent),
  so M1 can ignore it; general phars need a `serialize`/`unserialize` reader.
- **Wrapper-specific ops via `Any` downcast** (`mapPhar` reaching `PharWrapper`)
  is pragmatic but slightly unidiomatic; acceptable and contained.
- **`phar.require_hash`**: implement SHA-512 verify for M1 (composer is signed);
  surface the other hash types for general phars.
- **Userspace stream wrappers** (`stream_wrapper_register` with a PHP class) are
  Stage 3 — not needed for composer; the native-wrapper trait is the foundation.
