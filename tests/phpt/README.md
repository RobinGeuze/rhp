# php-src `.phpt` differential tests

`rhp-difftest phpt` runs php-src's own test suite through both the real `php`
and `rhp`, and compares their output the way `run-tests.php` does. php is the
oracle, not the test's `--EXPECT--`, so `EXPECTF` patterns and platform quirks
need no special handling.

- `php-src.ref` is the php-src commit everything here runs against.
- `known-passing.txt` lists the tests rhp currently matches php on. CI (the
  `phpt` job) fails if one of them stops matching, goes missing, or is skipped
  for any reason other than its own `--SKIPIF--`. This list ratchets: add tests
  as they start passing, and remove a line only for a deliberate reason. CI
  and the nightly check out `Zend/tests`, `tests`, `ext/*/tests`,
  `ext/*/*/tests` (`ext/intl/uchar/tests`) and `sapi/*/tests`: every php-src
  test.
- `ignored.txt` lists tests that never run (see [the ignore list](#the-ignore-list)).
- `totals.txt` counts php-src's tests per directory, for the PHP test support
  table in the top-level README.md. `rhp-difftest gen readme` rewrites that
  table from `known-passing.txt`, `ignored.txt` and `totals.txt`; run it in
  the change that edits them (`cargo test` fails until you do). When
  `php-src.ref` changes, run `rhp-difftest gen readme --php-src ../php-src`
  so `totals.txt` is recounted too (CI's `generated tables` job checks it).

How it compares, mirroring `run-tests.php`:

- stdout and stderr are merged (`2>&1`), CRLF is normalised, and the result is
  trimmed.
- Some php output is unspecified and can change from run to run (a raw
  `memcmp()` magnitude from `strncmp()`, ids, sizes). php-src marks such
  output with `--EXPECTF--` or `--EXPECTREGEX--`. When the two engines'
  outputs differ but both satisfy the test's own pattern, translated exactly
  like `run-tests.php` and matched with PCRE, the test passes. The summary
  says how many matches came about this way.
- php gets run-tests' INI overrides and environment (offline,
  non-interactive). Performance-sensitive tests (`SKIP_PERF_SENSITIVE`) are
  skipped; slow tests run, as with run-tests' default (`--skip-slow` sets
  `SKIP_SLOW_TESTS`, like run-tests' `-x`). Both
  engines run with `LC_ALL=C`, so the locale (libc's, and ICU's default
  locale behind intl) is the same on every machine; run-tests itself passes
  the caller's environment through.
- `--INI--` is read as run-tests' `settings2array` reads it: `name=value`
  lines, both sides trimmed, the last value of a name kept.
- A test's `--INI--` setting that leaves php's effective value as run-tests'
  defaults have it (`ini_get()`, asked of the php under test once per
  setting) is a no-op. A directive php doesn't know (`ini_get()` is false)
  only becomes a cfg entry (`get_cfg_var()`): both engines get it. A
  setting that changes a known directive's value runs only if rhp applies
  the directive (`RHP_INI` in `tools/rhp-difftest/src/phpt.rs`, e.g.
  `pcre.jit`, `error_reporting`, and every `opcache.*`); both engines then get
  the setting. `--rhp-ini <directive>` runs the tests a directive blocks, to
  measure it before adding it.
- `opcache.enable_cli=1` tests run with opcache on in both engines (rhp has
  the compiled-script cache, no optimizer and no JIT). Tests that dump JIT
  code (`opcache.jit_debug`) or use the file cache are skipped with an
  `opcache:` reason. Tests that dump opcodes are on the ignore list; one
  that isn't (new in a php-src bump) is skipped as `opcode dump not on
  tests/phpt/ignored.txt` until it is added.
- Each worker has its own copy of the test trees. After every engine run, the
  runner executes `--CLEAN--` and restores the test's directory, so php and rhp
  (and every mutant) start from the same files.
- Tests that spawn "the php under test" spawn the engine they run on:
  `TEST_PHP_EXECUTABLE` (and `_ESCAPED`) is php for php's run and rhp for
  rhp's, `PHP_BINARY` is each engine's own binary, and
  `TEST_PHP_CGI_EXECUTABLE` is php-cgi for php and a shim running the CGI
  candidate (`rhp --cgi`) for rhp. `TEST_PHP_EXTRA_ARGS` is run-tests'
  ` -n` plus the `-d` settings. Tests that start php's CLI server (`-S`,
  which rhp lacks) are skipped, except where the server is only the HTTP
  peer of the client code under test (ext/curl, and ext/standard's `http`,
  `streams` and `url` tests): both engines get `TEST_PHP_EXECUTABLE=php`
  there.
- `--CONFLICTS--` (or a `CONFLICTS` file in the test's directory) is
  scheduled as run-tests schedules it: two tests sharing a key never run at
  once, and tests that conflict with `all` run one by one after the rest.
- `--CAPTURE_STDIO--` captures only the streams it names; the others are a
  pty, as a terminal would be under run-tests. Output is stdout and stderr
  merged when both are captured, else the captured one.
- `--FILEEOF--` is the script minus its trailing newlines. A flaky test
  (run-tests' `is_flaky()`: `--FLAKY--`, a timing function, ...) or one
  whose output looks like a transient failure is run once more when it
  doesn't match, like run-tests' retry.
- `--write-known` re-runs every match with 1.1s between the two engines, and
  leaves out any test that then diverges. That keeps output depending on the
  current time out of the list.

```sh
cargo build --release -p rhp-cli -p rhp-difftest
git -C ../php-src checkout "$(grep -v '^#' tests/phpt/php-src.ref)"

# Explore: classify and cluster every failure, biggest gaps first.
target/release/rhp-difftest phpt --php-src ../php-src --filter Zend/tests

# Ratchet: what CI runs, split across three runners (--shard 1/3, 2/3, 3/3:
# every third test).
target/release/rhp-difftest phpt --php-src ../php-src --check-known tests/phpt/known-passing.txt

# Regenerate the list (after fixing things) from the whole corpus, as the
# nightly does. --write-known keeps only tests that ran and matched, so a
# --filter here drops listed tests, and so do skips: CGI tests without a
# php-cgi, version-printing tests when php's version isn't the one rhp reports.
target/release/rhp-difftest phpt --php-src ../php-src \
  --write-known tests/phpt/known-passing.txt

# Mutation: run 8 single-token mutants of every passing test through both
# engines, and write minimised, self-contained repros of the divergences.
target/release/rhp-difftest phpt --php-src ../php-src \
  --check-known tests/phpt/known-passing.txt \
  --mutate 8 --emit-fixtures /tmp/found --minimize
```

An emitted repro is a `NAME.php` / `NAME.out` pair in the `tests/golden` format,
with php's output as the expectation. Once rhp is fixed, move the pair into
the right `tests/golden/` subdirectory so it stays covered.

## The ignore list

`ignored.txt` lists tests whose expected output rhp can never print. One test
per line: its path, then whitespace and the reason. Blank lines and `#` lines
are skipped; keep it C-sorted. The list is compiled into `rhp-difftest`, so
rebuild it after an edit.

- Ignored tests never run. The summary counts them apart from skips, and
  `--json` gives them the status `ignored`.
- `--write-known` never writes one.
- `--check-known` fails, before running anything, if `known-passing.txt`
  lists an ignored test or an ignored test is missing from `--php-src`.

A test belongs here only if no change to rhp could make it match. Today that
is the 61 tests that set `opcache.opt_debug_level`: they print Zend's
optimizer listing. A missing feature (the JIT, the file cache, an extension)
is a skip or a failure, not an ignore. When an ignored test also prints
script output and rhp matches php on it, that part is kept as a golden
(`tests/golden/lang/opt_*.php`). Twenty of the 61 print script output; 14 are
goldens. The others, with what blocks them:

- `ext/opcache/tests/ct_eval_frameless_002.phpt`: the "Undefined
  variable" warning.
- `ext/opcache/tests/dump_property_hooks.phpt`: property hooks.
- `ext/opcache/tests/pipe_optimizations.phpt`: piping into a first-class
  callable (`5 |> f(...)`).
- `ext/opcache/tests/nodiscard_001.phpt`: the `(void)` cast; the
  `zend_test_nodiscard()` part needs zend_test.
- `ext/opcache/tests/preload_optimizer.phpt` (`opcache.preload`) and
  `ext/dl_test/tests/frameless_temporary_opcache.phpt` (dl_test.so): not
  portable to a golden.

## Hunting bugs in php itself

Mutants also exercise php in ways its own tests don't, so php can crash on
them. `--php-findings DIR` records every run in which **php itself** went
wrong: it was killed by a signal, printed an AddressSanitizer or UBSan report,
or reported `zend_mm_heap corrupted`. (A timeout doesn't count, because a
mutant can loop forever legitimately.) Findings are grouped by bug: the ASan
error type plus the first stack frame in php's sources, or the UBSan source
line and message. Stack overflows form one group, because php's recursion limit
is well known and where it trips varies from run to run. For each group, up to
three repros are written, minimised under `--minimize` (with a time budget) so
that the *same* bug still reproduces: `NAME.php` plus php's output in
`NAME.txt`.

`--mutate-all` mutates every test, not only the ones rhp already matches. For
tests rhp doesn't match yet, the mutants run through php only.

The nightly `php-bugs` job runs this against a php built with ASan and UBSan
from the pinned php-src commit (`.github/scripts/build-php-sanitizers.sh`:
php-src's own CI flags, minus UBSan's noisy `object-size` check), with
run-tests' `--asan` environment. It needs `llvm-symbolizer` on PATH, or every
ASan finding lands in one unnamed group:

```sh
.github/scripts/build-php-sanitizers.sh /path/to/php-src-copy "$PWD/php-san"   # Linux, clang
USE_ZEND_ALLOC=0 USE_TRACKED_ALLOC=1 ZEND_DONT_UNLOAD_MODULES=1 SKIP_ASAN=1 \
ASAN_OPTIONS=detect_leaks=0 UBSAN_OPTIONS=print_stacktrace=1 RHP_PHP="$PWD/php-san/bin/php" \
target/release/rhp-difftest phpt --php-src /path/to/php-src-copy --filter Zend/tests/ \
  --mutate 4 --mutate-all --minimize --timeout 30 --php-findings /tmp/php-bugs
```

Before reporting a finding upstream, check it against php-src's current
branch. It may already be fixed.
