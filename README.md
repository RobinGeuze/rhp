# phrust

A PHP 8.5 execution engine written in Rust — a clean-room reimplementation of
the language, replacing the Zend/C implementation. Parses with
[mago-syntax](https://github.com/carthage-software/mago), compiles to a
register-based bytecode, and runs it on a VM.

> Status: early. The core language (scalars, operators, type juggling, arrays,
> variables, string interpolation, a starter standard library) runs and is
> verified byte-for-byte against real `php` 8.5. Control flow, user functions,
> and OOP are in progress. Extensions and non-CLI SAPIs are future work.

## Design

Two execution tiers over one shared, verified semantics layer:

- **Bytecode VM** (`phrust-vm`) — the primary core: an AST→bytecode compiler
  (`phrust-compiler`) over a data-only register ISA (`phrust-bytecode`).
- **Tree-walking oracle** (`phrust-oracle`) — a small interpreter kept purely as
  a *differential* reference.

Both run every fixture in `tests/golden/` and must match the output of the real
`php` binary **byte-for-byte**. Because all operators and builtins are shared
code (`phrust-semantics`, `phrust-builtins`), a VM≠oracle divergence can only be
a compiler/VM bug — not a semantic one.

### Crates

| crate | role |
|---|---|
| `phrust-values` | `Value` (zval), `PhpStr` (byte-string, COW), `PhpArray` (ordered, COW), conversions, float formatting, `Thrown` |
| `phrust-frontend` | safe scoped wrapper over the mago-syntax arena parser |
| `phrust-semantics` | PHP operator semantics (arithmetic, comparison, concat, …) |
| `phrust-builtins` | native function + constant registry, `var_dump`/`print_r`/… |
| `phrust-bytecode` | the register ISA (data only) |
| `phrust-compiler` | AST → bytecode; owns the structural refusals |
| `phrust-vm` | the register VM |
| `phrust-oracle` | tree-walking differential oracle |
| `phrust-cli` | the CLI SAPI + differential test runner |

Full design docs are in [`docs/design/`](docs/design/) — start with
[`bytecode.md`](docs/design/bytecode.md).

## Non-goals / refused by design

`eval`, `extract`, `compact`, `create_function`, `include`/`require`, backticks,
and the `(unset)` cast are **deliberately not implemented** — they are refused
structurally at compile time, never silently mis-executed.

## Build & test

```sh
cargo build
cargo test --workspace          # unit tests + php-differential golden suite
cargo run -p phrust-cli -- script.php
```

Requires Rust ≥ 1.99 and (for regenerating golden fixtures) a local `php` 8.5.
