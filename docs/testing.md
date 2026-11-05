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
PCRE2, …) are newer than the mac's. Every test and harness finds php
through `rhp_difftest::php_bin` (`php` from `PATH`; `RHP_PHP` overrides it)
and php-cgi through `rhp_difftest::php_cgi` (the `php-cgi` next to that
php, else from `PATH` or `/opt/homebrew/bin`; `RHP_PHP_CGI` overrides it).

## Layout

```
tests/                         data only
  golden/AREA/NAME.php|.inc|.out   goldens (areas below), reference-only.txt
  cli/CASE/                    CLI fixtures: a script, RUN.cmd, RUN.stdout, …
  cgi/NAME.php|.cgi            CGI fixtures
  curl_http/NAME.php|.out      curl fixtures
  phpt/                        known-passing.txt, ignored.txt, php-src.ref
  apps/                        pinned Symfony / Laravel apps
  asan/                        ASan stress scripts, phpts.txt
crates/rhp-cli/tests/          diff.rs (the goldens, in-process), and runs
                               of the rhp binary: cli.rs, cgi.rs,
                               unsupported.rs, the live process tests
                               (intl, memory_limit, phar, ref_build,
                               startup_ini); curl_http.rs
crates/rhp-sapi/tests/         the request layer in-process: embedding,
                               isolation, memory, opcache
crates/rhp-compiler/tests/     prelude_cache.rs: the per-thread prelude cache
crates/rhp-vm/tests/golden.rs  tests/golden/vm, for `cargo test -p rhp-vm`
crates/*/src/**/tests.rs       unit tests, a sibling file per module
tools/rhp-difftest/            the harness: php discovery, spawn, known
                               lists, Diff (lib.rs); golden, phpt, app,
                               lint, gen, sweeps; tests/ = live-php tests
```

The golden areas follow php-src: `lang/` (the Zend language: syntax,
control flow, functions, references, juggling, constants, include,
namespaces, exceptions, errors, generators), `oop/` (classes, traits,
enums, objects, magic methods, autoloading), `stdlib/AREA/` (ext/standard,
by its test areas: `array`, `strings`, `math`, `var`, `file` for files and
streams, `output`, `general`), `ext/NAME/` (one directory per other
extension), `sapi/`, `corpus/` (larger whole programs) and `vm/` (the VM's
own). A fixture's `.inc` files sit next to it.

## Which kind of test

| kind | where | expectation | runs without php |
|---|---|---|---|
| golden | `tests/golden/AREA/NAME.php` + `NAME.out` | committed output of `php -n` | yes |
| CLI fixture | `tests/cli/CASE/` (`main.php`, `RUN.cmd`, `RUN.stdout`, …) | committed stdout, stderr and exit status of `php -n` | yes |
| live-php test | `tools/rhp-difftest/tests`, `crates/rhp-cli/tests` | php on the machine, at test time | skips |
| CGI fixture | `tests/cgi/NAME.php` (+ `NAME.cgi`) | php-cgi on the machine | skips |
| curl fixture | `tests/curl_http/NAME.php` + `NAME.out` | committed output of `php -n` against the test's local server | yes |
| phpt ratchet | `tests/phpt/known-passing.txt` | php on the machine, per php-src `.phpt` | no (CI job) |
| apps | `tests/apps/` | lint list; php-cgi for the request diff | no (CI job) |
| ASan | `tests/asan/` | stress scripts print `ok`; `phpts.txt`, a subset of the ratchet, still matches | no (CI job) |

To choose:

1. A golden, whenever php's stdout says it all and is the same on every
   machine and with every library version php may link.
2. A CLI fixture when the test needs the binary as a process: argv,
   stdin, `-d`/`-r`, stderr, the exit status, `fork`.
3. A CGI fixture for `rhp --cgi`.
4. A live-php test when the output depends on the machine or the library
   versions, or for a sweep over generated inputs.
5. A unit test only for an internal helper or one of rhp's own contracts
   that php has no output for (the embedding API, refusals).

**Goldens** are the default for language and library behaviour. The runner
(`crates/rhp-cli/tests/diff.rs`) runs every fixture below `tests/golden`
as a CLI request in-process (`rhp_sapi::CaptureSapi`), by its absolute
path and from its directory as php ran it, and requires byte-identical
stdout. Each fixture runs with its directory as the request's working
directory (`Request::cwd`); the test process's own directory and umask
never change, so the fixtures run on several threads at once
(`GOLDEN_THREADS=N`, default: one per CPU; `GOLDEN_THREADS=1` runs them in
order). The few that install signal handlers, which belong to the
process, run alone afterwards (`ALONE` in diff.rs). A fixture's temp
files need names no other fixture uses. `GOLDEN_ONLY=SUBSTR` runs only
the fixtures whose name
(`ext/json/json_flags`, below `tests/golden`, no extension) contains
SUBSTR. `tests/golden/vm` holds the VM's own goldens; rhp-vm's
`tests/golden.rs` runs them instead, so `cargo test -p rhp-vm` and
cargo-mutants on rhp-vm see them. Both runners honour `GOLDEN_ONLY`;
diff.rs fails when it matches no fixture at all, `vm/` included.
Print only the basename of paths (`basename(__FILE__)`), catch exceptions
whose message holds a path (print `get_class($e)` and the message with the
path removed), keep output independent of time, the machine, the
environment and where the fixture sits, and refer to included files by
`__DIR__`.

**Golden policy:** a golden holds only output that is the same with every
library version php may link. The no-php CI job runs the goldens against
Ubuntu's libraries, not the mac's. Output that varies with the library
version (libxml2 error messages, ICU data, curl and OpenSSL error strings)
or with the machine belongs in a live-php test, which
compares with the php linked against the same libraries on the same machine.

**CLI fixtures** cover what needs the `rhp` binary as a process: argv and
`getopt()`, php-cli's own option parsing and its errors, a script read
from stdin, `-r` and startup `-d` values, stderr, the exit status,
uncaught errors and shutdown order, `pcntl_fork`. Each
`tests/cli/CASE/` directory holds a script and one or more runs: `RUN.cmd`
(the command line, stdin and environment, in the CGI fixtures' directive
format), `RUN.stdout`, `RUN.stderr` and `RUN.status` (php's, with the
case's path replaced by `{DIR}`); a single run is named `expected`.
`crates/rhp-cli/tests/cli.rs` documents the format, runs every run under
target's `rhp` and compares all three. It needs no php; `-- --check`
also runs php and compares it with the committed files.

**Live-php tests** are Rust tests that run a PHP snippet through php and
rhp and compare. Use them for version-sensitive output, for comparisons
that depend on the machine (memory_limit sizes, the linked library
versions, phar archives php builds), and for sweeps over generated inputs
(`rhp-difftest`'s hash, bcmath, ctype, … sweeps; the startup ini sweep).
Each one returns early when `rhp_difftest::php_available()` is false.

Unit tests (`crates/*/src/**/tests.rs`, `tools/rhp-difftest/src/**/tests.rs`,
`crates/rhp-sapi/tests`, `crates/rhp-cli/tests/unsupported.rs`) hold
literal expectations for rhp's own contracts: the embedding API, request
isolation, the compiled-script cache, refusals, internal helpers,
diagnostics and error branches. A module's unit tests live in a sibling
file, never inline: the module declares `#[cfg(test)] mod tests;` and the
tests go in `foo/tests.rs` for `foo.rs`, or in `tests.rs` beside a
`mod.rs` or `lib.rs`. rhp-builtins' tests call builtins with
`test_support::TestHost`, which keeps the output and diagnostics and runs
callbacks through a Rust closure; don't add another `Host` for a test.
A case a golden, a sweep or a live-php test already covers needs no unit
test.

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
dump, that output is kept as goldens (`tests/golden/lang/opt_*`; the pending
ones are listed in the phpt README). Once rhp optimizes, add a developer
`--dump-bytecode` and per-pass snapshot tests modelled on php-src's
`ext/opcache/tests/opt`.

**ASan**: CI's `asan` job builds rhp with AddressSanitizer
(`-Zsanitizer=address` on a dated nightly pinned in ci.yml) and runs
`tests/asan/*.php` and the php-src tests in `tests/asan/phpts.txt`, the
code that frees libxml2 trees. Bump the nightly on purpose. Add a
script or a test there when you change how DOM, SimpleXML or XMLReader
own nodes.

**Apps** (`tests/apps/README.md`) are pinned Symfony and Laravel projects:
a compile-only lint ratchet and a request diff of `rhp --cgi` against
php-cgi.

## Regenerating expectations

- A golden: `rhp-difftest golden --write --filter NAME` with the reference
  php (see below); a new fixture needs no `.out` first. Never regenerate a
  golden just to make a failing run pass: a changed `.out` is a behaviour
  change to explain.
- A CLI fixture: `cargo test -p rhp-cli --test cli -- --write` with the
  reference php (`CLI_ONLY=CASE/RUN` narrows it to matching runs). A new
  run needs its `RUN.stdout` to exist first (an empty file will do).
- A curl fixture: `cargo test -p rhp-cli --test curl_http -- --ignored regenerate`.
- The phpt list: `rhp-difftest phpt --write-known` (see the phpt README),
  then keep the list C-sorted with its header lines first; remove an entry
  only with a stated reason.
- An app's lint list: `python3 tests/apps/apps.py lint APP --write-known`.
- `rhp-difftest phpt --mutate N --emit-fixtures DIR --minimize` writes
  minimised golden pairs for the divergences it finds; move a pair into
  its `tests/golden/` area once rhp is fixed.

## Goldens and the reference php

A golden's `.out` is the stdout of the reference php for the fixture, run
the canonical way:

- `php -n /absolute/path/to/NAME.php` (`RHP_PHP` names the php),
- with the fixture's directory as the working directory,
- with the environment inherited, except `TMPDIR`: a fresh empty
  directory, so a `cargo test` running the goldens at the same time can't
  touch the files a fixture writes there;
- with stdin empty;
- only stdout is recorded, not stderr or the exit status.

`rhp-difftest golden --check` re-runs php that way on every fixture
(subdirectories included) and compares with the committed `.out`;
`--write` rewrites them, and only on the reference build (php 8.5.12 on
macOS); `--filter SUBSTR` narrows either to matching names. Run `--check`
against the reference build after changing a fixture and at every php
version bump.

Some goldens print facts of the reference build: its configuration, and
the versions and features of the libraries it links.
`tests/golden/reference-only.txt` lists them, with the reason for each.
`golden --check --portable` skips them; CI's php job runs it against its
php 8.5.12 (Ubuntu, setup-php's libraries). The list came from running
every golden under Ubuntu's php 8.5 with CI's library versions, not from
CI itself: if CI's step finds another fixture, list it with its reason.
A new golden whose output depends on the build belongs in a live-php test
instead (the golden policy above); list it only when that can't be
avoided.

Generated tables have their own `--check`: `rhp-difftest gen signatures`,
`gen html-tables`, `gen parser`, `gen intl-consts`, `gen curl-consts`,
`gen zip-consts`, `gen readme` (see `tools/rhp-difftest/src/gen/`).
`gen readme` writes README.md's PHP test table from `tests/phpt`; `cargo
test` checks it, and `gen readme --php-src` also recounts
`tests/phpt/totals.txt`.

## CI

`.github/workflows/ci.yml`, on pull requests to main:

| job | runs |
|---|---|
| rustfmt, clippy | `cargo fmt --check`, `cargo clippy -D warnings` |
| test (no php) | the whole `cargo test --workspace --all-targets`, with php removed: goldens, CLI and curl fixtures, unit tests; live-php tests skip |
| php 8.5 (live) | only what compares with php: `php -l` on every prelude file (`crates/rhp-builtins/src/**/*.php`), rhp-difftest's tests, rhp-cli's tests except the goldens (the live tests, the CGI fixtures against the real php-cgi), `cli -- --check`, `golden --check --portable` |
| release build | `cargo build --release -p rhp-cli -p rhp-difftest` once, as an artifact for the jobs below (with the licence files) |
| phpt ratchet K/3 | the ratchet in three shards (`phpt --shard K/3`) |
| generated tables | every `gen … --check` |
| app NAME | per pinned app: smoke test, lint ratchet, request diff |
| asan | its own AddressSanitizer build: `tests/asan` |

## Running

```sh
cargo test --workspace --all-targets          # goldens, unit and live-php tests
RHP_PHP=php cargo test --workspace --all-targets --no-fail-fast
GOLDEN_ONLY=ext/json cargo test -p rhp-cli --test diff  # some goldens
CLI_ONLY=getopt cargo test -p rhp-cli --test cli        # some CLI runs
CGI_ONLY=NAME cargo test -p rhp-cli --test cgi          # one CGI fixture
cargo build --release -p rhp-cli -p rhp-difftest
target/release/rhp-difftest golden --check     # the .out files are still php's
cargo test -p rhp-cli --test cli -- --check     # so are the CLI fixtures' files
target/release/rhp-difftest phpt --php-src ../php-src \
  --php-cgi "$(command -v php-cgi)" --check-known tests/phpt/known-passing.txt
```
