# Testing rhp

rhp is right when it prints what php prints. Every check below compares rhp
with php, either with output php recorded earlier or with a php running next
to it.

## The reference php

The reference is php 8.5.12, the Homebrew build on macOS (aarch64). Its
version and build values live in `crates/rhp-builtins/src/version.rs`;
`tools/capture-ref-build.php` prints them, and
`crates/rhp-cli/tests/ref_build.rs` checks that rhp reports the same and links
the same library versions as the php on the machine. CI uses setup-php's
8.5.12 on Ubuntu, whose libraries (libxml2, ICU, curl, OpenSSL, libzip,
PCRE2, …) are newer than the mac's. Live-php tests use `php` from `PATH`
(`RHP_PHP` overrides it for `rhp-difftest` and `ref_build`) and `php-cgi`
from `PATH` or `/opt/homebrew/bin` (`PHP_CGI` overrides it).

## Which kind of test

| kind | where | expectation | runs without php |
|---|---|---|---|
| golden | `tests/golden/NAME.php` + `NAME.out` | committed output of `php -n` | yes |
| live-php test | `crates/rhp-cli/tests`, `crates/rhp-difftest/tests` | php on the machine, at test time | skips |
| CGI fixture | `tests/cgi/NAME.php` (+ `NAME.cgi`) | php-cgi on the machine | skips |
| curl fixture | `tests/curl_http/NAME.php` + `NAME.out` | committed output of `php -n` against the test's local server | yes |
| phpt ratchet | `tests/phpt/known-passing.txt` | php on the machine, per php-src `.phpt` | no (CI job) |
| apps | `tests/apps/` | lint list; php-cgi for the request diff | no (CI job) |

**Goldens** are the default for language and library behaviour. The runner
(`crates/rhp-cli/tests/diff.rs`) runs every fixture on the VM in-process under
its real path and requires byte-identical output. Print only the basename of
paths (`basename(__FILE__)`), catch exceptions whose message holds a path,
and keep output independent of time, the machine and the environment.

**Golden policy:** a golden holds only output that is the same with every
library version php may link. The no-php CI job runs the goldens against
Ubuntu's libraries, not the mac's. Output that varies with the library
version (libxml2 error messages, ICU data, curl and OpenSSL error strings)
or with the machine belongs in a live-php test, which
compares with the php linked against the same libraries on the same machine.

**Live-php tests** are Rust tests that run a PHP snippet through php and
rhp and compare. Use them for version-sensitive output, for behaviour that
needs the `rhp` binary (argv, stdin, exit status, stderr interleaving, ini
files, `fork`), and for sweeps over generated inputs (`rhp-difftest`'s hash,
bcmath, ctype, … sweeps). Each one returns early when `php_available()` is
false.

Unit tests (inline `#[cfg(test)]`, `crates/rhp-vm/tests`,
`crates/rhp-sapi/tests`) hold literal expectations for rhp's own contracts:
VM snippets, the embedding API, request isolation, the compiled-script cache.

**CGI fixtures** cover `rhp --cgi`: `crates/rhp-cli/tests/cgi.rs` runs each
`tests/cgi/*.php` under rhp and php-cgi with the request a `NAME.cgi` file
describes (directives are documented at the top of `cgi.rs`) and compares
stdout, stderr and the exit status.

**The phpt ratchet** runs php-src's own tests through php and rhp; every
test in `tests/phpt/known-passing.txt` must keep matching. See
[`tests/phpt/README.md`](../tests/phpt/README.md). Add tests that start
passing because of rhp's own behaviour, not ones that pass only because the
local php lacks an extension CI's php has (or the reverse). Tests whose
output rhp can never print are on `tests/phpt/ignored.txt` and never run.

**Bytecode snapshots:** there are none, and there should be none until rhp
has a real optimizer. php-src's opcode-dump tests are on the phpt ignore
list. Where rhp matches php on the script output they print after the
dump, that output is kept as goldens (`tests/golden/opt_*`; the pending
ones are listed in the phpt README). Once rhp optimizes, add a developer
`--dump-bytecode` and per-pass snapshot tests modelled on php-src's
`ext/opcache/tests/opt`.

**Apps** (`tests/apps/README.md`) are pinned Symfony and Laravel projects:
a compile-only lint ratchet and a request diff of `rhp --cgi` against
php-cgi.

## Regenerating expectations

- A golden: `cd tests/golden && php -n NAME.php > NAME.out`, with the
  reference php. Never regenerate a golden just to make a failing run pass:
  a changed `.out` is a behaviour change to explain.
- A curl fixture: `cargo test -p rhp-cli --test curl_http -- --ignored regenerate`.
- The phpt list: `rhp-difftest phpt --write-known` (see the phpt README),
  then keep the list C-sorted with its header lines first; remove an entry
  only with a stated reason.
- An app's lint list: `python3 tests/apps/apps.py lint APP --write-known`.
- `rhp-difftest phpt --mutate N --emit-fixtures DIR --minimize` writes
  minimised golden pairs for the divergences it finds; move a pair into
  `tests/golden/` once rhp is fixed.

Generated tables have their own `--check`: `rhp-difftest gen signatures`,
`gen html-tables`, `gen parser` (see `crates/rhp-difftest/src/gen/`).

## Running

```sh
cargo test --workspace --all-targets          # goldens, unit and live-php tests
RHP_PHP=php cargo test --workspace --all-targets --no-fail-fast
cargo build --release -p rhp-cli -p rhp-difftest
target/release/rhp-difftest phpt --php-src ../php-src \
  --php-cgi "$(command -v php-cgi)" --check-known tests/phpt/known-passing.txt
```
