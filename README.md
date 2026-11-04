# rhp

A PHP 8.5 execution engine written in Rust — a clean-room reimplementation of
the language, replacing the Zend/C implementation. Parses with
[mago-syntax](https://github.com/carthage-software/mago), compiles to a
register-based bytecode, and runs it on a VM.

> Status: in progress. The language (functions, closures, generators, classes,
> enums, traits, exceptions, references, `include`/`require`, `eval`) and a
> large part of the standard library and common extensions run, verified
> byte-for-byte against real `php` 8.5; Composer runs end to end. A php-cgi
> compatible SAPI (`rhp --cgi`) and an embeddable request layer (`rhp-sapi`)
> exist; `-S` and FastCGI are future work.

## Design

One execution core over one shared, verified semantics layer: an
AST→bytecode compiler (`rhp-compiler`) emits a data-only register ISA
(`rhp-bytecode`) that the **bytecode VM** (`rhp-vm`) runs. Operators
(`rhp_values::ops`) and builtins (`rhp-builtins`) are shared libraries, so
each PHP rule is implemented once.

The VM runs every fixture in `tests/golden/` and must match the output the real
`php` binary recorded for it **byte-for-byte**. `rhp-difftest` also diffs rhp
against a live `php` / `php-cgi` on the php-src `.phpt` corpus, fixture apps and
extension sweeps. [`docs/testing.md`](docs/testing.md) says which kind of test
to write when.

### Crates

Production crates are in `crates/`, the test tooling in `tools/`.

| crate | role |
|---|---|
| `rhp-values` | `Value` (zval), `PhpStr` (byte-string, COW), `PhpArray` (ordered, COW), conversions, float formatting, `Thrown`, the operators (`ops`) |
| `rhp-bytecode` | the register ISA (data only) and php's stub signatures for the builtins |
| `rhp-compiler` | parsing (mago-syntax) and AST → bytecode; refuses what rhp does not implement yet. Knows the builtins only through what the request layer passes in |
| `rhp-builtins` | php's builtin functions, constants and PHP prelude (`*.php`), laid out as php-src (`zend/`, `main/`, `ext/`, `sapi/`; [builtins-layout.md](docs/design/builtins-layout.md)) |
| `rhp-csrc` | php-src's own C (crypt) and frozen C libraries php links (argon2, timelib), vendored and linked statically |
| `rhp-mbfl-sys` | php-src's libmbfl (LGPL-2.1), vendored and linked statically for mbstring |
| `rhp-vm` | the register VM |
| `rhp-sapi` | the request layer: `run_request`, `compile_file`, the `Sapi` trait a server implements, the CLI and CGI SAPIs and their command lines |
| `rhp-cli` | the `rhp` binary: picks php-cli or php-cgi (`--cgi`) and hands over to `rhp-sapi` |
| `rhp-difftest` | in `tools/`: differential harnesses against php / php-cgi (php-src `.phpt`, goldens, fixture apps, extensions) and the table generators |

The compiler and the builtins build in parallel: neither depends on the
other.

```
rhp-values ── rhp-bytecode ─┬─ rhp-compiler ────────────┐
                            └─ rhp-builtins ── rhp-vm ──┴─ rhp-sapi ── rhp-cli
                                 └─ rhp-csrc, rhp-mbfl-sys (C)
```

Full design docs are in [`docs/design/`](docs/design/README.md) — start with
[`bytecode.md`](docs/design/bytecode.md).

### Running web requests

`rhp --cgi` behaves like `php-cgi` (same environment variables, same
`Status:`/header/body output), and is diffed byte-for-byte against it. For
embedding in a web server, `rhp-sapi` is a library: the server implements
the `Sapi` trait (body writer, `send_headers`, flush, log), fills in a
`RequestInfo` (server variables, query string, cookies, body) and calls
`rhp_sapi::run_request` on a worker thread, with one fresh engine per
request, pinned to that thread, and engine values that stay thread-local
(`Rc`, not `Send`). Each request has its own working directory and umask;
rhp never changes the process's, and reads its umask once, when the first
engine starts. See [`docs/design/sapi.md`](docs/design/sapi.md).

## Non-goals / refused by design

`extract`, `compact`, `create_function`, backticks and the `(unset)` cast are
**deliberately not implemented**. Backticks and the cast are refused at compile
time, the functions when called; either way rhp stops with an `RHP_UNSUPPORTED`
message naming the construct and its file and line, and never silently
mis-executes. (`include`/`require` and `eval` are supported.)

## Build & test

```sh
cargo build
cargo test --workspace          # unit tests + php-differential golden suite
cargo run -p rhp-cli -- script.php
```

Requires Rust ≥ 1.99, the C libraries rhp links (on macOS php's own Homebrew
dependencies and the SDK; on Debian/Ubuntu the `-dev` packages that
`.github/actions/native-deps` installs; see
[`docs/design/libraries.md`](docs/design/libraries.md)) and, for
regenerating golden fixtures, a local `php` 8.5.
rhp tracks the latest php 8.5 release (currently 8.5.12, Homebrew build): every
version and build value it reports lives in `crates/rhp-builtins/src/version.rs`
or comes from the library rhp linked, and `tools/capture-ref-build.php` prints
them from a php for comparison.

rhp is licensed under MIT OR Apache-2.0; the code it links or ports from
others is listed in `THIRD-PARTY-NOTICES`.
