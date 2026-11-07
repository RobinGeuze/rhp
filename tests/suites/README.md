# Framework test suites

The frameworks' own PHPUnit suites, run under php and under rhp and compared
per test: a regression net of thousands of tests that exercise the code the
fixture apps (tests/apps) run. `suites.py` drives them from `suites.toml`;
the nightly workflow `suites-nightly.yml` runs every suite.

| suite | what runs | version | parts |
|-------|-----------|---------|-------|
| `laravel-13` | laravel/framework's `tests/` (clone at the tag, its own dev deps) | 13.39.0 | one per `tests/<dir>` |
| `symfony-7.4` | the `Tests/` of 20 Symfony components, installed from source | 7.4.x as the `symfony-7.4` fixture locks them | one per component |
| `wordpress-7.1` | wordpress-develop's PHPUnit suite on SQLite (PHPUnit 9.6) | 7.1.4 | one per `--group` |

## What is committed

Per suite, in `tests/suites/<suite>/`:

- `composer.lock`: every dependency pinned. For `laravel-13` it is resolved
  from the clone's own `composer.json` (dev deps included; the framework
  allows Symfony 7.4 or 8, and resolves to 8.1). `symfony-7.4` and
  `wordpress-7.1` have their own `composer.json`: the components with
  `preferred-install: source`, so each `vendor/symfony/<component>/` keeps
  its `Tests/` and `phpunit.xml.dist` (dist archives leave them out); and
  for WordPress only PHPUnit 9.6 and the polyfills (it replaces the clone's
  `composer.json`, whose other dev deps are linters).
- `wp-tests-config.php` (WordPress): the test configuration, rhp's own. The
  database is the SQLite Database Integration drop-in, the same plugin
  build as the fixture app, in `var/database/` (emptied before each part).
  `WP_PHP_BINARY`, which the bootstrap runs `includes/install.php` with, is
  the engine under test.
- `known-passing.txt`: the tests that pass under php and rhp, one
  `Class::test` per line (data sets included: `Class::test with data set #0`).
- `flaky.txt` (when needed): tests whose outcome under php varies between
  runs; every comparison leaves them out.

Nothing of the frameworks is committed: the clones, `vendor/` and the
downloads live in `tests/suites/work/` (`$RHP_SUITES_DIR`), git-ignored.

## Running

```sh
cargo build --release -p rhp-cli
python3 tests/suites/suites.py setup laravel-13       # clone, composer install, setup
python3 tests/suites/suites.py run laravel-13 --engine php --out /tmp/suites
python3 tests/suites/suites.py run laravel-13 --engine rhp --out /tmp/suites
python3 tests/suites/suites.py compare laravel-13 --ref /tmp/suites/laravel-13/php \
    --cand /tmp/suites/laravel-13/rhp --check-known
python3 tests/suites/suites.py run laravel-13 --engine rhp --out /tmp/suites --part Support   # one part
```

- Each part is one PHPUnit process, run with a fixed environment (`HOME` and
  `TMPDIR` under the output dir, `TZ=UTC`, `LANG=LC_ALL=C`, `COLUMNS=120`,
  `NO_COLOR=1`, `TERM=dumb`), the part's timeout, and
  `--log-teamcity`/`--log-junit`. Outcomes are read from the TeamCity log,
  which PHPUnit writes as tests finish, so when the engine dies only the
  rest of that part is lost; the test that was running counts as `crash`.
- Outcomes: `pass`, `fail` (failure or error), `skip` (skipped or
  incomplete), `crash`. A test run by two parts (WordPress groups overlap)
  keeps the worse outcome.
- `compare`: a test passes when it passed in every `--ref` run and in the
  `--cand` run. `--check-known` exits 1 when a listed test that php passes
  doesn't pass under rhp; a listed test php doesn't pass either is reported
  only. The report groups the tests php passes and rhp doesn't by the first
  line of rhp's message (or the part's crash/timeout): the roadmap.
- `--write-known` rewrites the list (keep each list in step with what rhp
  passes, like the phpt ratchet: drop a line only with a stated reason).
  With several `--ref` runs, `--write-flaky` adds the tests whose outcome
  varied to `flaky.txt`.
- Self-check: three php runs (`--engine php --label php`, `php2`, `php3`)
  compared with `--ref php --ref php2 --cand php3` must leave no unstable
  test outside `flaky.txt`. The nightly's `php_runs` input does the same.

## The nightly

Per suite: setup (the work dir is cached on the manifest and lock), a php
run, an rhp run, then `compare --check-known`. It fails on regressions; the
job summary lists the counts, the newly passing tests and the failure
clusters, and the artifact has every log plus `passing.txt`, the list to
commit. The lists started empty; commit them from the first green nightly
(or a local run).

PR CI runs only the runner's own tests (`test_suites.py`).

## Adding a suite or a part

Add a `[[suite]]` (see the comments in `suites.toml`) or a part, resolve the
lock with `composer update --no-install --no-scripts --no-plugins` (in the
clone for a suite that uses its own `composer.json`, then copy the lock
here), and run `python3 -m unittest discover -s tests/suites -p 'test_*.py'`.
