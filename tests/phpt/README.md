# php-src `.phpt` differential tests

`rhp-difftest phpt` runs php-src's own test suite through both the real `php`
and `rhp`, and compares their output the way `run-tests.php` does. php is the
oracle, not the test's `--EXPECT--`, so `EXPECTF` patterns and platform quirks
need no special handling.

- `php-src.ref` is the php-src commit everything here runs against.
- `known-passing.txt` lists the tests rhp currently matches php on. CI (the
  `phpt` job) fails if one of them stops matching, goes missing, or is skipped
  for any reason other than its own `--SKIPIF--`. This list ratchets: add tests
  as they start passing, and remove a line only for a deliberate reason.

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

# Regenerate the list (after fixing things) from the same three trees CI checks out.
target/release/rhp-difftest phpt --php-src ../php-src \
  --filter tests/lang/ --filter Zend/tests/ --filter ext/standard/tests/ \
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
