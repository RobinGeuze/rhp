# Placeholders & partial implementations

**Status:** engineering backlog (as of 2026-10-25).
**Purpose:** catalogue every place where rhp currently fakes, stubs, approximates,
or outright no-ops a piece of PHP behaviour, with a concrete plan to finish each.
**Ground truth:** the reference `php` (the differential oracle; see `crates/rhp-builtins/src/version.rs`). Every item below
was confirmed by reading the cited `file:line` in `crates/` and, where relevant,
by comparing against php's documented behaviour. The `vm/lib.rs:N` citations
predate the 2026-11 split of `rhp-vm` into modules (bytecode.md §3): search
for the named function instead. The goal that orders the roadmap
is the north star — run `composer.phar` end to end (`history/modules-and-streams.md`) and
then real frameworks.

Nothing here is a *bug* in the sense of "wrong where it claims to be right": each
shortcut was taken deliberately to get `composer.phar --version` byte-identical
(M1, shipped) without paying for machinery no bootstrap path exercises. This doc
is the list of IOUs that machinery left behind, so they can be paid down in
priority order rather than rediscovered one framework crash at a time.

---

## 0. TL;DR

| # | Placeholder | Where | Breaks | Effort | Priority |
|---|---|---|---|---|---|
| 1 | `ini_get`/`ini_set` — tiny curated table, set is a no-op | `php_fns.rs:118`,`:135` | per-request config, `ini_set('memory_limit',…)` round-trip | med | med |
| 2 | `set_error_handler`/`set_exception_handler`/`restore_*` — stored-nowhere no-ops | `php_fns.rs:140`,`:146` | framework error pipelines, `trigger_error` routing | high | high |
| 3 | `trigger_error` — returns true, emits nothing | `php_fns.rs:150` | user warnings/deprecations, logging | med | med |
| 4 | `error_reporting` — always returns `32767` | `php_fns.rs:154` | reads/writes of the mask; no effect either way | low | low |
| 5 | ~~`register_shutdown_function` — accepted, never invoked~~ **done**: run by `Vm::run_main` (then destructors, then `ob_*` flush) | — | shutdown hooks, fatal-error handlers, flush-on-exit | — | — |
| 6 | `memory_get_usage`/`_peak_usage` — fixed `2097152` | `php_fns.rs:167` | memory probes, `ini` memory-limit logic | low | low |
| 7 | `gc_*`, `set_time_limit` — no-ops | `php_fns.rs:445` | none observable (cosmetic) | low | low |
| 8 | `setlocale` — always `"C"` | `php_fns.rs:187` | locale-dependent formatting | low | low |
| 9 | `php_uname` — hardcoded Darwin/arm64 strings | `php_fns.rs:81` | platform probes on non-macOS hosts | low | low |
| 10 | `extension_loaded('reflection')` → true, **no Reflection class exists** | `php_fns.rs:42` | `class_exists`/Reflection-gated code paths lie | med | high |
| 11 | `PHP_BINARY` constant — hardcoded `/usr/bin/php` | `builtins/lib.rs:402` | re-exec / proc-spawn of "self" | low | med |
| 12 | `mt_rand`/`rand` — CSPRNG, `mt_srand` ignored | `php_fns.rs:268`,`:458` | reproducible seeded sequences | med | med |
| 13 | `putenv` — no-op true | `php_fns.rs:113` | child-process env, `getenv` round-trip | low | med |
| 14 | ~~`fseek`/`ftell` → 0 (no-op)~~ **RESOLVED** — real positions on plain files (`LocalFile`) and `php://memory` | `fs_fns.rs`, `streams.rs` | — | — | — |
| 15 | `stream_set_blocking`/`_timeout`/buffer knobs — no-ops | `stream_fns.rs:124` | none observable (cosmetic) | low | low |
| 16 | ~~`stream_isatty`/`posix_isatty` → false~~ **done**: real `isatty(2)` on the stream's fd (`proc_fns.rs`, `ext/posix`) | — | colour/TTY detection (composer output) | — | — |
| 17 | ~~`stream_get_meta_data` — fixed minimal array~~ **RESOLVED** — built from the live stream (wrapper/stream type, mode, uri, eof, seekable, unread_bytes) | `fs_fns.rs` | `wrapper_data` still absent | — | — |
| 18 | ~~stdin reads return empty~~ **done**: `STDIN` reads a buffered dup of fd 0; `php://stdin` opens its own | — | interactive input, piped stdin | — | — |
| 19 | ~~`is_readable`/`is_writable`/`is_executable`~~ **RESOLVED** — `access(2)` for plain files, mode bits vs uid/gid for other wrappers (phar) | `fs_fns.rs` | — | — | — |
| 20 | ~~`fileperms` — fixed~~ **RESOLVED** — real `st_mode` via the stream layer (phar: php's `phar_dostat` modes) | `fs_fns.rs` | — | — | — |
| 21 | ~~`chmod`/`chdir`/`clearstatcache`~~ **RESOLVED** — real `chmod`/`chdir`; `clearstatcache` is a no-op because rhp has no stat cache (php's single-entry stat cache staleness is not modelled) | `file_fns.rs` | — | — | — |
| 22 | ~~`touch` — does not set mtime~~ **RESOLVED** — sets mtime/atime (php's defaulting rules) | `file_fns.rs` | — | — | — |
| 23 | ~~Phar MD5/SHA-1/OpenSSL signatures — accepted without verification~~ **resolved**: all signature types verified (OpenSSL against `<phar>.pubkey`) | `ext/phar/mod.rs` `verify_signature` | — | — | — |
| 24 | Phar bzip2 entries unsupported | `ext/phar/mod.rs:124` | bz2-compressed phars | low | low |
| 25 | Phar is **read-only**; `createDefaultStub`→`""`, `canCompress`→false | `ext/phar/mod.rs:426`,`:429` | phar writing/creation; self-update | high | low |
| 26 | Phar tar/zip formats unsupported | `ext/phar/mod.rs` (native-only) | tar/zip phars | med | low |
| 27 | ~~zlib compressors not byte-identical to php~~ — **done**: C zlib with php's deflateInit2 params (`ext/zlib/zraw.rs`) | — | — | — | — |
| 28 | ~~`gzopen` family + `compress.zlib://` wrapper missing~~ — **done** (whole-buffer streams; gzopen's level/strategy mode letters are ignored on write) | `ext/zlib/mod.rs` | — | low | low |
| 29 | `spl_autoload_unregister` — no-op true, never removes | `ext/spl/mod.rs:39` | dynamic autoloader swapping | med | low |
| 30 | `Closure::bindTo`/`Closure::call` instance methods missing | `compiler/prelude.rs` | rebinding via instance form; scoped invoke | med | high |
| 31 | var_dump object ids — monotonic, no handle recycling | `vm/classes.rs:321` | byte-exact `var_dump`/`spl_object_id` vs php | high | low |
| 32 | No `E_WARNING`/`E_NOTICE`/`E_DEPRECATED` diagnostics ever emitted (builtins already report them through `Host::emit_diagnostic` — iconv, openssl, mbstring so far — which every host still discards) | engine-wide | warnings-as-errors, deprecation logs, `@`-suppression semantics | high | med |
| 33 | `declare(strict_types=1)` — no-op; type hints unenforced | `compiler/lib.rs:1977` | type coercion/TypeError on hinted params — **in progress** | high | high |
| 34 | Named args to functions whose defaults don't fold — refused | `compiler/lib.rs:3284` | named-arg calls skipping non-constant-default params | med | med |
| 35 | Arithmetic on a leading-numeric string (`"1abc" + 5`, `"8k" % 3`) — no "A non-numeric value encountered" warning | VM arithmetic | warning output, error handlers that throw on it | low | med |
| 36 | Builtin `int`/`int\|float` parameters accept leading-numeric strings (`intdiv("9z", 2)`) instead of throwing `TypeError` | builtin argument parsing | TypeError parity at builtin calls | med | med |
| 37 | Named arguments to builtins — refused (`RHP_UNSUPPORTED`) | `vm/lib.rs` (call setup) | `str_replace(search: …)` and every other named builtin call | med | high |
| 38 | `metaphone()` — not implemented | — | callers get "Call to undefined function" | low | low |

---

## 1. Runtime / env functions (`crates/rhp-builtins/src/php_fns.rs`)

This file's own header is candid: the handler setters are "stored, not invoked —
this engine emits no diagnostics", plus "a handful of no-op-ish runtime knobs"
(`php_fns.rs:1-5`). The individual shortcuts:

### 1.1 `ini_get` / `ini_set`
- **Now:** `ini_get` answers from a 9-entry hardcoded match (`php_fns.rs:118-132`)
  and returns `false` for anything else. `ini_set` is a pure no-op that returns
  `false` (`php_fns.rs:135-138`).
- **OK because:** composer's bootstrap only reads a handful of settings
  (`memory_limit`, `precision`, `date.timezone`, …) and never depends on a write
  taking effect. The values returned are the php-CLI defaults.
- **Full impl:** a per-request INI store on the `Vm` (a `HashMap<Vec<u8>, Vec<u8>>`
  seeded with php's defaults), reached via two new defaulted `Host` methods
  (`ini_get`/`ini_set`, mirroring the `define_const`/`get_const` pattern in
  `builtins/lib.rs`). `ini_set` returns the previous value and records the new one;
  `ini_get` reads the store. A few settings have *behavioural* coupling
  (`precision`/`serialize_precision` feed float formatting, `memory_limit` feeds
  item #6) — those must read the store rather than constants once it exists.
- **Effort/priority:** med / med. Cheap mechanically; the value is that framework
  config (`ini_set('memory_limit', '512M')`, error-display toggles) stops silently
  vanishing.

### 1.2 Error / exception handler registration
- **Now:** `set_error_handler`, `set_exception_handler` return `Null` ("no previous
  handler") and discard their argument; `restore_*` return `true`
  (`php_fns.rs:140-148`).
- **OK because:** the engine emits no diagnostics (§7), so an error handler would
  never fire, and uncaught exceptions are handled by the VM's own top-level path.
- **Full impl:** store the handler callables on the `Vm` (new `Host` hooks
  `set_error_handler`/`set_exception_handler` returning the previous value). This
  is a prerequisite for, and co-dependent with, #3 (`trigger_error`) and the §7
  diagnostics work: an emitted `E_*` must look up the current error handler and
  invoke it with `(errno, errstr, errfile, errline)`; an uncaught `Throwable` must
  route through the exception handler before the VM's default. Without diagnostics
  this is half a feature, so land it alongside §7.
- **Effort/priority:** high / high. It is on the critical path for real frameworks
  (every framework installs handlers at boot), but only pays off once diagnostics
  exist.

### 1.3 `trigger_error`
- **Now:** returns `true`, emits nothing (`php_fns.rs:150-152`).
- **OK because:** differential tests run php with `error_reporting=0`
  (`rhp-difftest/src/lib.rs:57`), so php's own `trigger_error` output is suppressed
  too — parity holds by both sides being silent.
- **Full impl:** once #1.2 + §7 land, `trigger_error(msg, level)` becomes "emit a
  user diagnostic of `level`", i.e. route to the registered error handler or the
  default formatter. Needs the error-handler store and the diagnostic sink.
- **Effort/priority:** med / med (gated behind §7).

### 1.4 `error_reporting`, `error_get_last`
- **Now:** `error_reporting` ignores its arg and always returns `32767` (=`E_ALL`)
  (`php_fns.rs:154-156`); `error_get_last` always returns `Null`
  (`php_fns.rs:158-160`).
- **OK because:** nothing consumes the mask (no diagnostics to filter), and no
  error is ever recorded.
- **Full impl:** store the reporting mask in the INI store (#1.1); `error_get_last`
  reads the last diagnostic recorded by §7. Trivial once those exist.
- **Effort/priority:** low / low.

### 1.5 `register_shutdown_function` — registered, **never invoked** — RESOLVED
- **Resolved:** callbacks are stored on the `Vm` and run by
  `Vm::run_main` after `{main}` returns, exits or throws: shutdown functions in order (an `exit`/uncaught throwable stops the
  rest), then destructors (globals holding the sole reference, last first, then
  every remaining object in creation order), then all output buffers are flushed.
  Objects with a `__destruct` are tracked; one the program releases is destructed
  at the next instruction boundary (frame return, overwritten/unset variable, end
  of an expression statement via `ClearTemps`). Gap: an object freed while only a
  non-tracked container held it is destructed late (at the next check) rather than
  exactly at release, and cycles are destructed at shutdown (php's GC may run
  earlier).

The original notes:
- **Now:** accepts and discards; returns `Null` (`php_fns.rs:162-165`). The comment
  admits "Stored by php and run at shutdown; not yet invoked here" — and it is not
  even stored.
- **OK because:** composer's `--version` path registers no shutdown function whose
  absence changes stdout.
- **Full impl:** store the callables (+bound args) on the `Vm`; invoke them in
  registration order after `{main}` returns and before the process exits, *and* on
  a fatal error (that is their main use in frameworks: convert a fatal into a
  rendered error page / flush buffers). Requires a `Host` hook to register and a
  VM teardown hook to drain the list. Interacts with output buffering
  (`ob_*`) if/when a shutdown fn flushes.
- **Effort/priority:** high / high. Many frameworks (and composer plugins) rely on
  shutdown handlers; silent non-invocation is a correctness cliff.

### 1.6 `memory_get_usage` / `memory_get_peak_usage`
- **Now:** both return the constant `2_097_152` (2 MiB) (`php_fns.rs:167-169`,
  registered at `:443-444`).
- **OK because:** the number is plausible and nothing asserts on it.
- **Full impl:** rhp's allocator is Rust's, not a zend arena, so a *faithful* byte
  count is not meaningful. Options: (a) keep a coarse running estimate (bytes in
  live `Value`s is impractical), or (b) report process RSS via a platform call.
  Neither will match php's zend-MM accounting, so this stays approximate by nature
  — document it as "best-effort, not php-identical" rather than chase parity.
- **Effort/priority:** low / low.

### 1.7 `gc_enable`/`gc_disable`/`gc_collect_cycles`, `set_time_limit`
- **Now:** no-ops (`true`/`true`/`0`/`true`) (`php_fns.rs:445-448`).
- **OK because:** rhp uses `Rc` + the planned cycle story is separate; there is no
  settable time limit.
- **Full impl:** `gc_collect_cycles` could return a real freed-count if/when a
  cycle collector exists; otherwise these are permanently cosmetic. Leave as-is.
- **Effort/priority:** low / low (likely never).

### 1.8 `setlocale`
- **Now:** always returns `"C"`, models nothing (`php_fns.rs:187-190`).
- **OK because:** rhp's number/string formatting is locale-independent (C locale),
  which matches php's default.
- **Full impl:** only needed if locale-sensitive functions (`strcoll`,
  locale-aware `printf`, `strftime`) are implemented; defer until then.
- **Effort/priority:** low / low.

### 1.9 `php_uname`
- **Now:** returns hardcoded Darwin/`localhost`/`24.0.0`/`arm64` strings regardless
  of host (`php_fns.rs:81-92`).
- **OK because:** the oracle in CI is this macOS host, so the strings match there.
- **Full impl:** query the real host via `std::env::consts` + a `uname`-style
  platform call (libc `uname` behind cfg, or read `/proc`/sysctl). Low value until
  rhp runs on varied hosts under test.
- **Effort/priority:** low / low.

### 1.10 `extension_loaded` — curated list that **over-claims `reflection`**
- **Now:** `LOADED_EXTS` (`php_fns.rs:26-43`) lists 16 extensions and
  `extension_loaded`/`get_loaded_extensions`/`phpversion` answer from it. It
  includes `"reflection"` — but **no `ReflectionClass`/`ReflectionMethod`/… exists**
  anywhere (confirmed: zero `class Reflection*` in the tree).
- **OK because:** nothing on composer's `--version` path instantiates a Reflection
  object; it only checks presence.
- **Breaks:** any framework that does `extension_loaded('reflection')` (or
  `class_exists('ReflectionClass')` — which is a different check and returns false,
  so the two disagree) and then *uses* reflection will get a true answer followed
  by a fatal "class not found". This is the most dangerous entry in the file
  because it lies in the direction of "yes, go ahead".
- **Full impl:** two parts. (a) Near-term honesty: remove `"reflection"` from
  `LOADED_EXTS` until the classes exist, so probes fail cleanly. (b) The real fix
  is the Reflection extension itself (a separate, large work item — `modules.md`
  M2 names Reflection as the next composer milestone). The list should be derived
  from actually-registered modules, not hand-maintained, once the `ModuleDef`
  system (`history/modules-and-streams.md` §1) is in place.
- **Effort/priority:** (a) low / high — delete one line to stop lying.
  (b) high / high — Reflection is the M2 blocker for `composer list`/`about`.

### 1.11 `PHP_BINARY` constant
- **Now:** hardcoded `Value::str("/usr/bin/php")` (`builtins/lib.rs:402`);
  `PHP_BINDIR` similarly `"/usr/bin"` (`:408`).
- **OK because:** no bootstrap path re-execs `PHP_BINARY`.
- **Breaks:** code that spawns "the same php" (composer sometimes re-invokes
  `PHP_BINARY` for sub-commands / scripts) would launch the system php, not rhp.
- **Full impl:** set `PHP_BINARY` to the rhp executable's own path
  (`std::env::current_exe()`), threaded in from the CLI at VM construction (the CLI
  already knows its argv[0]). `PHP_BINDIR` = its parent dir.
- **Effort/priority:** low / med.

### 1.12 `mt_rand` / `rand` / `mt_srand` — CSPRNG, **not php's Mersenne Twister**
- **Now:** `mt_rand`/`rand` draw from the OS CSPRNG via `getrandom`
  (`php_fns.rs:225-276`); `mt_srand`/`srand` are no-ops (`fn_noop_null`,
  registered `:458-459`). So a seed is accepted and ignored.
- **OK because:** the outputs are uniform and in-range, and nothing on the
  bootstrap path asserts a *specific* seeded sequence.
- **Breaks:** `mt_srand($seed)` followed by `mt_rand()` must reproduce php's exact
  sequence (tests, fixtures, deterministic shuffles, cache keys built from seeded
  RNG). rhp's values are unrelated to php's for any given seed.
- **Full impl:** implement php's actual algorithm — the MT19937 generator with
  php's specific seeding (`php_mt_reseed`), the `RAND_RANGE`/`mt_rand`
  range-reduction, and php 8's default "MT_RAND_MT19937" mode — as a seedable
  generator stored on the `Vm` (state must persist across calls). Keep
  `random_int`/`random_bytes` on the CSPRNG (correct as-is). This is a
  self-contained, differentially-testable unit (seed × count sweep vs php), a good
  fit for `rhp-difftest`.
- **Effort/priority:** med / med. Not needed for composer, but a common framework
  /test-suite expectation.

### 1.13 `putenv`
- **Now:** no-op returning `true` (`php_fns.rs:113-116`); `getenv` reads the real
  process env (`:94-111`).
- **OK because:** composer reads env (`getenv`) but does not depend on `putenv`
  mutations being visible.
- **Breaks:** `putenv('X=1'); getenv('X')` round-trip, and env passed to spawned
  children. Because `getenv` reads the real env, a `putenv` that doesn't mutate it
  makes the pair inconsistent.
- **Full impl:** maintain a per-VM env overlay that both `putenv` and `getenv`
  consult (ahead of `std::env`), rather than mutating the real process env (which
  is global/unsafe). Simple map on the `Vm`.
- **Effort/priority:** low / med.

### 1.14 `metaphone()` — not implemented
- **Now:** "Call to undefined function metaphone()". `tests/golden/corpus/strings.php`
  leaves it out; add it back with the function.

---

## 2. Stream & filesystem

### 2.1 `fseek` / `ftell` — RESOLVED
- `fopen` on a plain local path now opens a real descriptor
  (`streams::LocalFile`, `StreamState::File`) with php's plain-files semantics:
  read buffer, logical position, eof set only when `read(2)` returns 0, greedy
  `fread`, `fgets($h, $len)`. `fseek`/`ftell`/`rewind`/`ftruncate`/`fstat`/
  `flock` work on it; `php://memory` seeks/truncates in its buffer. Non-local
  wrappers (`phar://`) still use the buffered `Mem`/`FileWrite` states.

### 2.2 `stream_set_blocking` / `_timeout` / write-buffer knobs
- **Now:** no-ops (`stream_fns.rs:124-127`), explicitly labelled "accepted so
  framework output code runs".
- **OK because:** rhp's streams are synchronous in-memory/file; blocking and
  buffering have no observable effect.
- **Full impl:** only meaningful once real sockets/pipes exist (networking is out
  of scope for composer per `history/modules-and-streams.md`). Leave as accept-and-ignore.
- **Effort/priority:** low / low.

### 2.3 `stream_isatty` / `posix_isatty` / `sapi_windows_vt100_support` → false
- **Status:** `stream_isatty` and `posix_isatty` are now real (`isatty(2)` on the
  fd behind STDIN/STDOUT/STDERR or a `proc_open` pipe; false for memory/file
  buffers). `sapi_windows_vt100_support` stays false (non-Windows). The notes
  below are the original analysis.
- **Was:** all `fn_false` (`stream_fns.rs:131-133`).
- **OK because:** under the test SAPI output is captured, so "not a tty" is the
  safe answer and matches piped php.
- **Breaks:** interactive composer runs decide colour/progress output from
  `stream_isatty(STDOUT)`; always-false forces no-colour mode. Fine for captured
  parity, wrong for a real terminal.
- **Full impl:** when the stream maps to the process stdout/stderr, call
  `libc::isatty`/`IsTerminal` on the real fd; keep false for in-memory streams.
  Needs the stream→fd association to exist (stdout/stderr already have dedicated
  `StreamState`s).
- **Effort/priority:** low / med.

### 2.4 `stream_get_meta_data` — RESOLVED
- Built from the live stream via `Host::stream_meta` (php's key order:
  timed_out, blocked, eof, wrapper_type, stream_type, mode, unread_bytes,
  seekable, uri). `wrapper_data` (http) is still not reported.

### 2.5 stdin reads return empty (`crates/rhp-vm/src/lib.rs:275-304`) — RESOLVED
- **Resolved:** `STDIN` reads go through a lazily created `FdStream` over a dup of
  fd 0 (php's buffering: one 8 KiB fill per read); `fopen('php://stdin')` and
  `file_get_contents('php://stdin')` open their own dup, as php does.

The original notes:
- **Now:** `stream_read` only handles `StreamState::Mem`; every other state
  (including `Stdin`) hits the `_ => Some(Vec::new())` arm (`vm/lib.rs:284`), and
  `stream_gets` returns `None` for stdin (`:302`). So `fgets(STDIN)` / `fread(STDIN)`
  yield nothing.
- **OK because:** the bootstrap and the diff corpus never read stdin.
- **Breaks:** any interactive prompt or piped-input program (`php script < input`,
  composer's rare interactive prompts).
- **Full impl:** back `StreamState::Stdin` with a real `std::io::stdin()` reader
  (buffered), implementing `stream_read(len)` and `stream_gets` (line read) and
  `stream_eof` against it. Keep it lazy so non-reading programs pay nothing.
- **Effort/priority:** med / med.

### 2.6 `is_readable` / `is_writable` / `is_executable` — RESOLVED
- Plain files: `access(2)` (php's own route). Other wrappers: stat mode bits
  against uid/gid as in `php_stat` (so `phar://` entries are readable, never
  writable).

### 2.7 `fileperms` — RESOLVED
- Real `st_mode` through the stream layer; `stat`/`lstat`/`fstat` return php's
  full 26-element array, and `fileinode`/`fileowner`/`filegroup`/`file[amc]time`
  read the same stat.

### 2.8 `chmod` / `chdir` / `clearstatcache` — RESOLVED (cache semantics aside)
- `chmod`/`chdir`/`umask` are real (process-global cwd, like php CLI).
  `clearstatcache` is a no-op: rhp keeps no stat cache, so a php script that
  observes a *stale* cached stat (e.g. `filesize` after an external change
  without `clearstatcache`) sees fresh data under rhp. Known, deliberate gap.

### 2.9 `touch` — RESOLVED
- Sets mtime/atime (`utimes`), creating the file when missing.

### 2.10 Diagnostics from filesystem functions
- php's warnings/notices (`opendir(...): Failed to open directory`, `fread():
  Read of N bytes failed`, `tempnam(): file created in the system's temporary
  directory`, `readdir()` null-handle deprecation, …) are not emitted — the
  engine has no diagnostics sink yet (see §7.1). Return values match.
- Closed resources still report their original type (`stream`) instead of
  php's `Unknown` / `resource (closed)`.

---

## 3. Phar (`crates/rhp-builtins/src/ext/phar/mod.rs`)

The module is deliberately "READ-ONLY, native format only"
(`ext/phar/mod.rs:10-12`). It is exactly enough to parse, verify, and execute
`composer.phar` (M1, shipped). The remaining gaps:

### 3.1 ~~MD5 / SHA-1 / OpenSSL signatures accepted without verification~~ (resolved)
- **Now:** `verify_signature` checks every native signature type php writes:
  MD5 / SHA-1 / SHA-256 / SHA-512 byte-for-byte, and OpenSSL (`0x10` SHA-1,
  `0x11` SHA-256, `0x12` SHA-512) by RSA/ECDSA-verifying the archive body against
  the `PUBLIC KEY` PEM in `<phar>.pubkey` (`ext/openssl/pkey.rs`
  `verify_with_pem`), with php's exact `PharException` messages for a missing
  key ("openssl public key could not be read"), an unreadable key ("openssl
  signature could not be processed") and a mismatch ("broken openssl
  signature"). Covered by `tests/golden/ext/phar/phar_openssl_signature.php`.

### 3.2 bzip2 entries unsupported
- **Now:** a `COMPRESSED_BZ2` entry returns `None` from `read`
  (`ext/phar/mod.rs:123-124`), i.e. the entry is unreadable.
- **OK because:** composer stores all entries uncompressed; gz (raw DEFLATE) is
  handled (`:118-122`).
- **Breaks:** bz2-compressed phars.
- **Full impl:** add a pure-Rust bzip2 decoder (`bzip2-rs`, or `bzip2` crate with a
  pure backend per `history/modules-and-streams.md` §5) and decompress in the `BZ2` arm.
- **Effort/priority:** low / low.

### 3.3 Read-only: no write/create; `createDefaultStub`→`""`, `canCompress`→false
- **Now:** the `Phar` prelude class exposes only read methods; `canCompress`
  returns `false`, `createDefaultStub` returns `""`, `interceptFileFuncs` is empty
  (`ext/phar/mod.rs:426-429`). There is no manifest *writer*.
- **OK because:** running a phar never writes one. Composer's own phar creation
  (`box`/self-build) is not a thing rhp needs to *run composer*.
- **Breaks:** any program that builds or modifies a phar (`new Phar(...)`,
  `$phar->addFile(...)`, `setStub`, compression). Composer self-update writes a new
  phar — so this blocks `composer self-update` specifically.
- **Full impl:** a native-format manifest *writer* (emit stub + manifest + data +
  recomputed signature), a writable `Phar` object model (per-VM open-archive state
  keyed by handle, as the zip plan in `history/modules-and-streams.md` §4 describes), and
  `createDefaultStub` producing php's canonical stub source. Large; gated behind
  `phar.readonly` semantics.
- **Effort/priority:** high / low (not needed for `install`; only self-update).

### 3.4 tar / zip phar formats unsupported
- **Now:** the parser is native-format only (`PharArchive::parse` scans for
  `__HALT_COMPILER` and reads the native manifest).
- **OK because:** composer.phar is native format.
- **Breaks:** tar-based and zip-based phars (some tools ship these).
- **Full impl:** detect the format and dispatch to a tar reader (`tar` crate) or a
  zip reader (`zip` crate) — the same crates the M2 install path needs for
  tgz/zip dist (`history/modules-and-streams.md` §4-5), so this rides along with that work.
- **Effort/priority:** med / low.

---

## 4. zlib / compression (`crates/rhp-builtins/src/ext/zlib/mod.rs`)

### 4.1 Compressors emit valid-but-not-byte-identical output
- **Now:** `gzdeflate`/`gzcompress`/`gzencode` use `flate2`'s pure-Rust
  `miniz_oxide` backend; the module header states compressors are "valid but not
  byte-identical" vs php's zlib and are proven by round-trip, not byte comparison
  (`ext/zlib/mod.rs:8-11`). Decompression *is* byte-exact.
- **OK because:** DEFLATE output is non-canonical; any valid stream round-trips.
  rhp's own consumers (phar inflate) only ever *decompress*.
- **Breaks:** code that hashes/compares compressed bytes, or expects php's exact
  gzip output (e.g. an HTTP `Content-Encoding` fixture, a checksum of a `.gz`).
- **Full impl:** to get php's exact bytes, link `flate2`'s zlib/zlib-ng backend
  (the actual zlib php uses) behind an opt-in feature — exactly the "native backend
  behind a Cargo feature" pattern in `extensions.md` §4. Keep miniz_oxide as the
  pure-Rust default. Accept that byte-parity requires matching php's zlib version.
- **Effort/priority:** med / low.

### 4.2 `gzopen` family + `compress.zlib://` wrapper — DONE
- **Status:** implemented: `gzopen`/`gzread`/`gzwrite`/`gzgets`/`gzeof`/`gzclose`/… are
  the stream functions over a `compress.zlib://` `StreamWrapper` (transparent read of
  plain files, concatenated members; writes match php's zlib stream bytes). Remaining
  gap: a level digit in `gzopen`'s mode (`wb9`) is ignored. `compress.bzip2://` +
  `bzopen` likewise (`ext/bz2`). The notes below are historical.
- **Now:** only the six string↔string functions exist (`ext/zlib/mod.rs:112-128`);
  the header defers `gzopen`/`gzread`/… and the `compress.zlib://` wrapper to "a
  later (resource) stage" (`:13-14`).
- **OK because:** composer's `--version` path needs none of them.
- **Breaks:** streamed gzip read/write, and — relevant to M2 — reading `.tar.gz`
  dist archives via a gzip stream wrapper during `composer install`.
- **Full impl:** now that `Value::Resource` exists (`rhp-values/src/lib.rs:50`),
  implement `gzopen`→a `StreamState` wrapping a `GzDecoder`/`GzEncoder`, with
  `gzread`/`gzwrite`/`gzgets`/`gzeof`/`gzclose` over the existing stream `Host`
  methods, plus a `compress.zlib://` `StreamWrapper` (de/compressing on read/write).
- **Effort/priority:** med / med (part of the M2 install path).

---

## 5. SPL / autoload (`crates/rhp-builtins/src/ext/spl/mod.rs`)

### 5.1 `spl_autoload_unregister` — no-op true, never unregisters
- **Now:** returns `true` without removing anything
  (`ext/spl/mod.rs:37-41`); the comment names the blocker: "callable identity
  comparison missing".
- **OK because:** composer registers its autoloader and never unregisters it;
  `spl_autoload_register`/`_functions`/`_call` all work (`:19-57`).
- **Breaks:** code that swaps autoloaders (test frameworks installing/removing a
  mock autoloader, composer's own `ClassLoader::unregister()`), which would leave
  stale autoloaders active.
- **Full impl:** implement PHP's callable-equality so a registered callable can be
  matched and removed from the `Vm`'s autoloader list. The hard part is identity
  for all callable forms — a closure (by `Rc` identity), `[obj, 'method']` (object
  identity + method name), `[class, 'method']` / `"Class::method"` (name equality),
  a plain `"function"` string. Add a `Host` method `unregister_autoloader(cb)` that
  applies these rules against the stored list. The same equality logic is reusable
  for `in_array`/`array_search` over callables.
- **Effort/priority:** med / low.

---

## 6. Closures (`crates/rhp-compiler/src/prelude.rs`, `crates/rhp-vm/src/lib.rs`)

### 6.1 `Closure::bindTo` / `Closure::call` instance methods missing
- **Now:** the prelude `Closure` class has only the two **static** methods
  `fromCallable` and `bind` (the `PRELUDE` source in `compiler/prelude.rs`).
  The underlying rebinding machinery exists and works — `__closure_bind`
  (`php_fns.rs:216`) calls `host.closure_bind` (`vm/lib.rs:355`), and the VM honours
  a rebound `$this`/scope when invoking (`vm/lib.rs:2273-2279`). Only the
  *instance-method spellings* are absent.
- **OK because:** composer uses `Closure::bind(...)` (static) and
  `Closure::fromCallable(...)`, both present.
- **Breaks:** `$closure->bindTo($newThis, $scope)` and `$closure->call($newThis,
  ...$args)` — both extremely common in framework code (container bindings, scoped
  callbacks). Currently a "method not found" fatal.
- **Full impl:** mostly additive. `bindTo` delegates to the existing
  `__closure_bind` with `$this` as the first arg — add
  `public function bindTo($newThis, $newScope = "static") { return __closure_bind($this, $newThis, $newScope); }`
  to the prelude class. `call` is `bindTo` + immediate invoke: bind, then call the
  bound closure with the remaining args — needs a tiny native helper
  (`__closure_call($closure, $newThis, $args)`) because the invoke-after-bind must
  happen in one step with the new scope. Both reuse `closure_bind`/the closure
  call path already in the VM; no new runtime mechanism.
- **Effort/priority:** med / high. Small, self-contained, and unblocks a lot of
  real framework code — good near-term win.

---

## 7. Diagnostics & error handling (engine-wide)

### 7.1 No `E_WARNING` / `E_NOTICE` / `E_DEPRECATED` diagnostics are ever emitted
- **Now:** the engine raises *exceptions* (`TypeError`, `ValueError`, …) but never
  emits a non-fatal diagnostic. Places that in php would warn instead silently
  succeed or return a sentinel — e.g. array-to-string conversion returns `"Array"`
  with a code comment "PHP also emits an E_WARNING" (`rhp-values/src/conv.rs:71`),
  undefined-index/variable reads yield null without a notice, etc. The `E_*`
  constants exist (`builtins/lib.rs:416-433`) purely as values.
- **How it's worked around:** the differential harness runs php with
  `error_reporting=0` (`rhp-difftest/src/lib.rs:57`, and the CLI diff tests of
  the time), so php writes no
  diagnostics to stdout either — parity is achieved by *both sides being silent*.
  php sends diagnostics to stderr, which the harness ignores.
- **Breaks:** (a) code that sets `error_reporting(E_ALL)` and treats warnings as
  fatal (many frameworks in dev/test), (b) deprecation logging, (c) the `@`
  error-suppression operator's semantics (there is nothing to suppress), (d) any
  output that *includes* a diagnostic when `display_errors=1`.
- **Full impl:** a diagnostic sink on the `Vm` — an `emit_diagnostic(errno, msg,
  file, line)` path that (1) records it as `error_get_last`, (2) consults the
  current `error_reporting` mask (#1.4) and registered error handler (#1.2), and
  (3) if unhandled and displayed, writes php's exact `Warning: … in … on line N`
  format. Then thread `emit_diagnostic` calls into every site that php warns at —
  this is the large, spread-out part (array-to-string, undefined index/var/prop,
  division warnings, type-coercion notices, hundreds of call sites). Best done
  incrementally, site by site, each pinned by a differential fixture run with
  `error_reporting=E_ALL` + `display_errors=stderr` compared on stderr.
- **Effort/priority:** high / med. Co-dependent with #1.2–#1.4. Not required for
  composer (which runs clean), but a prerequisite for faithful framework dev-mode.

### 7.2 Arithmetic on leading-numeric strings — no warning
- **Now:** `"1abc" + 5` and `"8k" % 3` give php's values (6, 2) but no
  `Warning: A non-numeric value encountered`, which php 8.5 emits for an
  operand with trailing data. Found folding tests/corpus into goldens
  (2026-11).
- **Workaround:** `tests/golden/corpus/typejuggle.php` silences that line
  with `@`; drop the `@` when this is fixed.
- **Full impl:** warn where the arithmetic operators convert a string
  operand, as php's `_zendi_try_convert_scalar_to_number` and
  `zendi_try_get_long` (for `%`, `<<`, `>>`, `|`, `&`, `^`) do in
  `Zend/zend_operators.c`.

---

## 8. Type system

### 8.1 `declare(strict_types=1)` is a no-op; type hints unenforced — **IN PROGRESS**
- **Now:** the compiler treats all `declare(...)` directives — including
  `strict_types=1` — as no-ops and just compiles the body
  (`compiler/lib.rs:1977-1988`). Parameter and return *types are parsed and stored*
  (`rhp-bytecode/src/lib.rs:279`, `:403`) and the proto carries a `strict_types`
  flag (`:405-407`, defaulting false at `:433`), but nothing reads them to coerce
  or to throw `TypeError`. So a function typed `(int $x)` accepts a string
  unchanged.
- **Status:** **actively being implemented right now** — the open modification in
  this working tree is `crates/rhp-bytecode/src/lib.rs` (the proto/type plumbing),
  which is the data side of exactly this feature.
- **OK because (until now):** the diff corpus avoided relying on coercion/TypeError
  at typed boundaries.
- **Breaks:** PHP's scalar type juggling at call boundaries — in weak mode, `"5"`
  passed to `(int $x)` coerces to `5` (with a deprecation for non-numeric); in
  strict mode, a type mismatch throws `TypeError`. Both are observable and
  common.
- **Full impl (the in-progress plan):** at the call boundary (`bind_params`,
  `vm/lib.rs:3170`) and at `return`, check each value against the declared type;
  in **weak** mode coerce per php's scalar rules (int↔float↔string↔bool, with the
  int→string and numeric-string rules), in **strict** mode (proto `strict_types`
  true) accept only exact types (plus the int→float widening php allows) and else
  throw `TypeError` with php's exact message. The `strict_types` flag must be set
  from the `declare` directive per-file and propagated to each proto.
- **Effort/priority:** high / high. On the critical path for real frameworks;
  already underway.

### 8.2 Named arguments to functions whose defaults don't fold — refused
- **Now:** for a call with named args into a *known* user function, the compiler
  reorders them into positional slots and fills gaps with the parameter's folded
  constant default (`compiler/lib.rs:3270-3297`). If a skipped parameter's default
  is **not a foldable constant** (e.g. `= new Foo()`, `= self::BAR`, `= FUNC()`),
  `specs[i].default` is `None` and the call errors with "Missing required argument"
  (`:3284-3293`) — even though php would supply the default. Named args into
  variadics, into unknown/dynamic callees, and with spread are refused outright
  (`:3145`, `:3289`, `:3365`, `:3823-3854`). The same non-constant-default gap
  exists at the positional-default site in the VM (`ParamDefault::Init(_) => {}`,
  `vm/lib.rs:3189`).
- **OK because:** composer's named-arg usage lands on foldable defaults, and the
  common case works.
- **Breaks:** calling with named args while skipping a parameter that has a
  non-constant default — increasingly common as constructors adopt named args with
  rich defaults.
- **Full impl:** represent non-constant defaults as a *default-init thunk* the
  callee evaluates in its own scope when the slot is unfilled (this is what php
  does — defaults are evaluated lazily at call time, not folded). Then the
  named-arg reordering can leave the slot "unprovided" and let the callee's prologue
  run the thunk; and `ParamDefault::Init` in `bind_params` stops being a no-op.
  This also fixes positional calls that omit a non-constant-default trailing arg.
  Touches the compiler (emit default-init thunks), the proto (store them), and the
  VM prologue (run them for unfilled params).
- **Effort/priority:** med / med.

### 8.3 Builtin int/number parameters take leading-numeric strings
- **Now:** a leading-numeric string passed to a builtin's `int` or
  `int|float` parameter is truncated to its number: `intdiv("9z", 2)` is 4,
  `str_repeat("a", "3x")` is `"aaa"`, `abs("4q")` is 4. php throws
  `TypeError: intdiv(): Argument #1 ($num1) must be of type int, string
  given`. User functions with `int $x` already throw in both.
- **Full impl:** the builtin parameter parser should accept only numeric
  strings (leading and trailing whitespace allowed) for these types, as
  `zend_parse_arg_long_weak` does, and raise php's `TypeError` otherwise.

### 8.4 Named arguments to builtins — refused
- **Now:** a call with named arguments to a builtin function stops with
  `RHP_UNSUPPORTED: named arguments to a builtin function is not supported
  (deferred)` (`vm/lib.rs`, both call paths).
- **Workaround:** `tests/golden/corpus/namedargs.php` keeps only named
  arguments to user functions; restore the `str_replace(search: …)` lines
  when this lands.
- **Full impl:** map names to positions with the builtin's arginfo (the
  generated signatures already hold parameter names) and fill skipped
  optional parameters with their defaults.

---

## 9. VM / object model

### 9.1 var_dump object ids — monotonic counter, no handle recycling
- **Now:** object ids come from a monotonically increasing `next_object_id`
  (`vm/classes.rs:321-322`, `:481-482`; `vm/lib.rs:51`, `:589`, `:1795`, `:2377`)
  and are printed by `var_dump` as `object(Class)#N` (`rhp-builtins/src/dump.rs:69`).
  php instead *recycles* freed object handles, so a short-lived object's id is
  reused by the next allocation. Also, closures always dump as `object(Closure)#0`
  with a hardcoded `#0` (`dump.rs:88`).
- **OK because:** the difftest suite deliberately never `var_dump`s objects for id
  comparison (`rhp-difftest/src/sweeps.rs:143`, `:163` note this explicitly for
  gmp), so the divergence is unobserved in CI.
- **Breaks:** any `var_dump`/`var_export`/`print_r`-of-object output compared to
  php byte-for-byte once more than one object is alive, and `spl_object_id`/
  `spl_object_hash` parity. php's recycling means its ids are smaller and reused;
  rhp's only grow.
- **Full impl:** reproduce php's handle allocator — a free-list of released object
  handle slots that new objects reuse, so ids match php's recycling order. This is
  invasive (object lifetime → handle release must be tracked, which `Rc` drop
  timing does not naturally mirror php's refcount-exact frees) and may never be
  perfectly byte-identical without matching php's exact alloc/free ordering. Treat
  as "parity only where object lifetimes are simple"; keep object ids out of the
  byte-exact contract otherwise. Fix the closure `#0` to use a real id as a cheap
  first step.
- **Effort/priority:** high / low. High cost, low payoff (tests already avoid it);
  do the closure-`#0` tidy-up opportunistically, defer the recycling allocator.

### 9.2 STDIN/STDOUT/STDERR constants — stale placeholder ints, shadowed at runtime
- **Now:** `builtins/lib.rs:436-441` defines `STDIN`/`STDOUT`/`STDERR` as
  placeholder ints `0`/`1`/`2` with a comment "this engine has no resource Value".
  That comment is now **stale**: `Value::Resource` exists, and `Vm::run` seeds the
  three constants with *real* stream resources at startup
  (`vm/lib.rs:718-724`), shadowing the int placeholders. So the runtime behaviour
  is correct; only the builtin-table defaults and their comment are leftover.
- **OK / already effectively fixed:** the real resources win because the VM inserts
  them into `self.constants` after the builtin constants are loaded.
- **Full impl:** delete the three int placeholders (and the stale comment) from
  `builtins/lib.rs` so the only definition is the VM's resource-backed one, and
  confirm nothing reads the ints before `run()` seeds them. Pure cleanup.
- **Effort/priority:** low / low.

---

## 10. Prioritized roadmap

Ordered by the north star: first what unblocks **`composer install`** of real
packages, then **general framework compatibility**, then long-tail parity.

### For `composer install` (M2 critical path)
1. **Reflection extension** (#1.10b) — named as the M2 blocker in
   `history/modules-and-streams.md`; also stop `extension_loaded('reflection')` lying today
   (#1.10a, one-line). *High effort, highest priority.*
2. **`register_shutdown_function` actually runs** (#1.5) — composer plugins and
   error paths depend on it. *High / high.*
3. **`strict_types` + type enforcement** (#8.1) — already in progress; real
   packages declare it and rely on coercion/TypeError. *High / high.*
4. **`chdir` + permission-accurate `is_writable`/`is_readable`** (#2.8, #2.6) —
   composer probes writability and changes directories during install.
   *Med / med.*
5. **`gzopen` + `compress.zlib://`** (#4.2) and **tar/zip phar + ZipArchive**
   (#3.4, and `history/modules-and-streams.md` §4) — needed to unpack tgz/zip dist archives.
   *Med / med.* (Networking to *fetch* them is out of scope per the modules doc.)

### For general framework compatibility
6. **`Closure::bindTo` / `Closure::call`** (#6.1) — small, additive, unblocks a lot.
   *Med / high — do early, it's cheap.*
7. **Error-handler store + diagnostics sink** (#1.2, #1.3, #1.4, #7.1) — frameworks
   install handlers at boot and run in `E_ALL` dev mode. Large and co-dependent;
   stage it (store handlers → emit the highest-value warnings → broaden).
   *High / med.*
8. **`ini_get`/`ini_set` real store** (#1.1), **`putenv` overlay** (#1.13),
   **`PHP_BINARY`=self** (#1.11) — config/env correctness. *Low–med / med.*
9. **Seekable streams** (`fseek`/`ftell`, #2.1), **`stream_get_meta_data`** (#2.4),
   **stdin reads** (#2.5), **`stream_isatty` real fd** (#2.3) — stream fidelity.
   *Med / med.*
10. **php's Mersenne Twister** (#1.12) — deterministic seeded RNG for test suites.
    *Med / med.*
11. **Named-arg non-constant defaults** (#8.2) — default-init thunks. *Med / med.*
12. **`touch` mtime** (#2.9), **`fileperms`/`chmod` real bits** (#2.7, #2.8) —
    cache/permission round-trips. *Low / med.*

### Long-tail parity (do when a concrete need forces it)
13. Phar MD5/SHA-1 verify (#3.1 cheap), OpenSSL-signed phars, phar writing /
    self-update (#3.3), bzip2 entries (#3.2) — *low priority*.
14. zlib byte-identical compressors (native backend, #4.1) — *low*.
15. `spl_autoload_unregister` callable identity (#5.1) — *low*.
16. var_dump object-id recycling (#9.1), STDIO-constant cleanup (#9.2),
    `php_uname` real host (#1.9), `memory_get_usage` best-effort (#1.6),
    `setlocale`/`gc_*` (#1.7, #1.8) — *low; some are "never" unless a test needs
    them*.

The through-line: items 1–5 are gated by the composer-install goal and the
`history/modules-and-streams.md` M2 plan; items 6–12 are what turns "runs composer" into
"runs a framework"; everything in the long tail is parity polish the differential
harness already lets rhp defer safely (by both sides staying silent, or by keeping
the diverging surface — object ids, compressed bytes — out of the byte-exact
contract).
