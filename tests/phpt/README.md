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
  and the nightly check out `Zend/tests`, `tests`, `ext/*/tests` and
  `sapi/*/tests`: every php-src test except the 5 in `ext/intl/uchar/tests`
  (none pass yet), which the list must not contain.

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
  non-interactive). Slow and timing-sensitive tests are skipped too.
- A test whose `--INI--` changes a directive runs only if rhp applies that
  directive (`RHP_INI` in `crates/rhp-difftest/src/phpt.rs`, e.g. `pcre.jit`);
  both engines then get the setting.
- Each worker has its own copy of the test trees. After every engine run, the
  runner executes `--CLEAN--` and restores the test's directory, so php and rhp
  (and every mutant) start from the same files.
- Tests that spawn "the php under test" (`TEST_PHP_EXECUTABLE`, `PHP_BINARY`)
  are skipped, because rhp's CLI doesn't accept php's flags.
- `--write-known` re-runs every match with 1.1s between the two engines, and
  leaves out any test that then diverges. That keeps output depending on the
  current time out of the list.

```sh
cargo build --release -p rhp-cli -p rhp-difftest
git -C ../php-src checkout "$(grep -v '^#' tests/phpt/php-src.ref)"

# Explore: classify and cluster every failure, biggest gaps first.
target/release/rhp-difftest phpt --php-src ../php-src --filter Zend/tests

# Ratchet: what CI runs.
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
`tests/golden/` so it stays covered.

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
