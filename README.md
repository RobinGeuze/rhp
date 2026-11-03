# rhp

A PHP 8.5 execution engine written in Rust — a clean-room reimplementation of
the language, replacing the Zend/C implementation. Parses with
[mago-syntax](https://github.com/carthage-software/mago), compiles to a
register-based bytecode, and runs it on a VM.

> Status: early. The core language (scalars, operators, type juggling, arrays,
> variables, string interpolation, a starter standard library) runs and is
> verified byte-for-byte against real `php` 8.5. Control flow, user functions,
> and OOP are in progress. A php-cgi compatible SAPI (`rhp --cgi`) and an
> embeddable request layer (`rhp-sapi`) exist; `-S` and FastCGI are future work.

## Design

One execution core over one shared, verified semantics layer: an
AST→bytecode compiler (`rhp-compiler`) emits a data-only register ISA
(`rhp-bytecode`) that the **bytecode VM** (`rhp-vm`) runs. Operators
(`rhp-semantics`) and builtins (`rhp-builtins`) are shared libraries, so each
PHP rule is implemented once.

The VM runs every fixture in `tests/golden/` and must match the output the real
`php` binary recorded for it **byte-for-byte**. `rhp-difftest` also diffs rhp
against a live `php` / `php-cgi` on the php-src `.phpt` corpus, fixture apps and
extension sweeps.

### Crates

| crate | role |
|---|---|
| `rhp-values` | `Value` (zval), `PhpStr` (byte-string, COW), `PhpArray` (ordered, COW), conversions, float formatting, `Thrown` |
| `rhp-frontend` | safe scoped wrapper over the mago-syntax arena parser |
| `rhp-semantics` | PHP operator semantics (arithmetic, comparison, concat, …) |
| `rhp-builtins` | native function + constant registry, `var_dump`/`print_r`/… |
| `rhp-csrc` | php-src's own C (crypt) and frozen C libraries php links (argon2), vendored and linked statically |
| `rhp-bytecode` | the register ISA (data only) |
| `rhp-compiler` | AST → bytecode; owns the structural refusals |
| `rhp-vm` | the register VM |
| `rhp-sapi` | the request layer: `run_request`, the `Sapi` trait a server implements, the CLI and CGI SAPIs |
| `rhp-cli` | the `rhp` command: argv front end over `rhp-sapi` (`rhp script.php`, `-r`, `--cgi`) |
| `rhp-difftest` | differential harnesses against php / php-cgi (php-src `.phpt`, fixture apps, extensions) |

Full design docs are in [`docs/design/`](docs/design/) — start with
[`bytecode.md`](docs/design/bytecode.md).

### Running web requests

`rhp --cgi` behaves like `php-cgi` (same environment variables, same
`Status:`/header/body output), and is diffed byte-for-byte against it. For
embedding in a web server, `rhp-sapi` is a library: the server implements
the `Sapi` trait (body writer, `send_headers`, flush, log), fills in a
`RequestInfo` (server variables, query string, cookies, body) and calls
`rhp_sapi::run_request` on a worker thread, with one fresh engine per
request and engine values that stay thread-local (`Rc`, not `Send`). See
[`docs/design/sapi.md`](docs/design/sapi.md).

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

Requires Rust ≥ 1.99 and (for regenerating golden fixtures) a local `php` 8.5.
rhp tracks the latest php 8.5 release (currently 8.5.12, Homebrew build): every
version and build value it reports lives in `crates/rhp-builtins/src/version.rs`,
and `tools/capture-ref-build.php` prints them from a php for comparison.
