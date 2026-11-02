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

Two execution tiers over one shared, verified semantics layer:

- **Bytecode VM** (`rhp-vm`) — the primary core: an AST→bytecode compiler
  (`rhp-compiler`) over a data-only register ISA (`rhp-bytecode`).
- **Tree-walking oracle** (`rhp-oracle`) — a small interpreter kept purely as
  a *differential* reference.

Both run every fixture in `tests/golden/` and must match the output of the real
`php` binary **byte-for-byte**. Because all operators and builtins are shared
code (`rhp-semantics`, `rhp-builtins`), a VM≠oracle divergence can only be
a compiler/VM bug — not a semantic one.

### Crates

| crate | role |
|---|---|
| `rhp-values` | `Value` (zval), `PhpStr` (byte-string, COW), `PhpArray` (ordered, COW), conversions, float formatting, `Thrown` |
| `rhp-frontend` | safe scoped wrapper over the mago-syntax arena parser |
| `rhp-semantics` | PHP operator semantics (arithmetic, comparison, concat, …) |
| `rhp-builtins` | native function + constant registry, `var_dump`/`print_r`/… |
| `rhp-bytecode` | the register ISA (data only) |
| `rhp-compiler` | AST → bytecode; owns the structural refusals |
| `rhp-vm` | the register VM |
| `rhp-oracle` | tree-walking differential oracle |
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

`eval`, `extract`, `compact`, `create_function`, `include`/`require`, backticks,
and the `(unset)` cast are **deliberately not implemented** — they are refused
structurally at compile time, never silently mis-executed.

## Build & test

```sh
cargo build
cargo test --workspace          # unit tests + php-differential golden suite
cargo run -p rhp-cli -- script.php
```

Requires Rust ≥ 1.99 and (for regenerating golden fixtures) a local `php` 8.5.
