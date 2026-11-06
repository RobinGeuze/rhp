# rhp

rhp is an engine for PHP 8.5 code, written in Rust. It parses PHP, compiles it
to a register-based bytecode and runs that on a VM, with the aim of printing
byte for byte what php 8.5 prints (the reference is php 8.5.12). Much of its
runtime is translated from php-src's C, and some php-src C is compiled in as is.
It is a very experimental hobby project: incomplete, and not for production use.

## Crates

The crates are in `crates/`, the test tooling in `tools/`.

- **rhp-values**: PHP's value model (the zval): copy-on-write byte strings and ordered arrays, objects,
  references. Type juggling, float formatting, memory accounting and the operators (`ops`) every crate shares.
- **rhp-bytecode**: the register-based instruction set, as plain data: the compiler emits it and the VM
  runs it, without either depending on the other. Also holds php's stub signatures of the builtins.
- **rhp-compiler**: parses PHP 8.5 with [mago-syntax](https://github.com/carthage-software/mago) and compiles
  the AST to bytecode. Refuses what rhp does not support (property hooks, `|>`, ...) with the file and line.
- **rhp-builtins**: the runtime library: php's functions, constants and classes and about 30 extensions, laid
  out like php-src. Links PCRE2, OpenSSL, libcurl, libxml2, ICU, zlib, bzip2, libzip, GMP and iconv.
- **rhp-csrc**: compiles C that rhp uses unmodified (php-src's crypt() code, timelib, the Argon2 reference
  code) into static archives, with safe Rust wrappers and every C symbol renamed `rhp_*`.
- **rhp-mbfl-sys**: php-src's libmbfl (LGPL-2.1) and php_unicode.c (PHP-3.01), vendored and linked
  statically in a crate of their own. mbstring runs on them, through rhp-builtins' default-on `mbstring-libmbfl` feature.
- **rhp-vm**: runs the bytecode: calls, classes and objects, generators, exceptions, include and eval,
  streams. Also an opcache-style compiled-script cache (no optimizer or JIT) and a request's engine state.
- **rhp-sapi**: the request layer: `run_request` runs one PHP request through a `Sapi` trait that
  the embedding server implements. Contains the CLI SAPI and a php-cgi compatible CGI SAPI.
- **rhp-cli**: the `rhp` binary: php's command line (a script, `-r`, `-d`, `-n`, `-f`, ...) on top
  of rhp-sapi. With `--cgi`, or when the binary is named `php-cgi`, it behaves as php-cgi.
- **rhp-difftest** (`tools/`): differential tests against a real php and php-cgi: php-src's .phpt suite,
  goldens, the fixture apps and extension sweeps. Also generates the tables taken from php-src, and this README's.

## PHP test support

php-src's own tests (every `.phpt` file at the php-8.5.12 tag, less the opcode
dumps in `tests/phpt/ignored.txt`) that rhp passes, meaning its output equals
php's. CI checks every listed test against php 8.5 on each change
(`tests/phpt/known-passing.txt`, see [tests/phpt/README.md](tests/phpt/README.md)).
A test that php skips, or that the harness does not run, counts as not
passing, so the percentages are of all php-src tests.

<!-- phpt-table start: written by `rhp-difftest gen readme`; edit only the notes -->
| area | passing | total | % | notes |
|---|---:|---:|---:|---|
| Zend/tests | 4,059 | 5,396 | 75.2% | no Fiber (0/113); lazy objects 215/229 (no typed reference sources); the pipe operator `\|>` is refused |
| tests | 693 | 895 | 77.4% | php-src's tests of the language, functions, classes and output |
| sapi/cli | 21 | 117 | 17.9% | tests that spawn the php binary run rhp; tests of the `-S` server are skipped |
| sapi/cgi | 5 | 24 | 20.8% | `rhp --cgi` against php-cgi |
| ext/standard | 2,891 | 3,880 | 74.5% | no user stream wrappers, stream filters, `mail`, `getimagesize` |
| ext/bcmath | 64 | 170 | 37.6% | the `bc*` functions; no `BcMath\Number` |
| ext/bz2 | 8 | 27 | 29.6% | libbz2 1.0.8, linked statically |
| ext/ctype | 38 | 49 | 77.6% | |
| ext/curl | 153 | 180 | 85.0% | on libcurl |
| ext/date | 684 | 705 | 97.0% | on timelib, compiled in (rhp-csrc) |
| ext/dom | 476 | 901 | 52.8% | the `DOMDocument` API on libxml2; no `Dom\` (HTML5) API |
| ext/filter | 40 | 129 | 31.0% | `filter_var` only |
| ext/gmp | 95 | 99 | 96.0% | on libgmp |
| ext/hash | 61 | 83 | 73.5% | all 60 algorithms; no `mhash`, `hash_hkdf` |
| ext/iconv | 75 | 82 | 91.5% | the system iconv |
| ext/intl | 42 | 511 | 8.2% | `grapheme_*`, `Normalizer`, `idn_*` and the `intl_*` error functions only, on ICU; not reported as loaded |
| ext/json | 80 | 94 | 85.1% | |
| ext/libxml | 17 | 32 | 53.1% | on libxml2 |
| ext/mbstring | 223 | 446 | 50.0% | on php-src's libmbfl (rhp-mbfl-sys) |
| ext/opcache | 702 | 876 | 80.1% | the compiled-script cache only; no optimizer, JIT or file cache |
| ext/openssl | 215 | 236 | 91.1% | on OpenSSL |
| ext/pcntl | 23 | 66 | 34.8% | |
| ext/pcre | 164 | 174 | 94.3% | PCRE2, linked statically |
| ext/phar | 19 | 574 | 3.3% | reads and runs phars (composer.phar); no write API |
| ext/posix | 32 | 61 | 52.5% | |
| ext/random | 17 | 69 | 24.6% | the functions only; no `Random\Randomizer` or engines |
| ext/readline | 0 | 28 | 0.0% | `readline()`, `readline_add_history()`, `readline_clear_history()` only; no line editing; not reported as loaded |
| ext/reflection | 353 | 540 | 65.4% | |
| ext/simplexml | 149 | 165 | 90.3% | on libxml2 |
| ext/sockets | 3 | 119 | 2.5% | `socket_import_stream()` and socket options only; no `socket_create`; not reported as loaded |
| ext/spl | 513 | 796 | 64.4% | |
| ext/tokenizer | 51 | 54 | 94.4% | a port of the Zend scanner |
| ext/xml | 61 | 71 | 85.9% | on libxml2 |
| ext/xmlreader | 57 | 63 | 90.5% | on libxml2 |
| ext/xmlwriter | 50 | 51 | 98.0% | on libxml2 |
| ext/zip | 115 | 125 | 92.0% | `ZipArchive` on libzip |
| ext/zlib | 81 | 161 | 50.3% | system zlib; no incremental deflate/inflate contexts |
| other ext/ | 2 | 3,953 | 0.1% | the 2 passing are ext/sodium tests that need none of its code |
| other sapi/ | 0 | 217 | 0.0% | |
| **total** | **12,332** | **22,219** | **55.5%** | |

Not implemented, so counted under other ext/: calendar, com_dotnet, dba, dl_test, enchant, exif, ffi, fileinfo, ftp, gd, gettext, imap, ldap, mysqli, odbc, pdo, pdo_dblib, pdo_firebird, pdo_mysql, pdo_odbc, pdo_pgsql, pdo_sqlite, pgsql, session, shmop, skeleton, snmp, soap, sodium, sqlite3, sysvmsg, sysvsem, sysvshm, tidy, uri, xsl, zend_test; other sapi/: fpm, phpdbg.
<!-- phpt-table end -->

## Application support

Measured on 2026-11-05 against php 8.5.12; only what was checked is listed.

| application | version | what works |
|---|---|---|
| Composer | 2.11.1 (phar) | Read-only commands (`list`, `about`, `show` except `--platform`, `validate`, `licenses`, `depends`, `why-not`, `outdated`, `audit`, `search`, ...) print what php prints. `update` and `require` write the same `composer.lock`, and `install --no-autoloader` the same `vendor/`. Generating the autoloader fails (rhp's `^` on two strings returns an int), so `install`, `update`, `require` and `dump-autoload` stop at "Generating autoload files". Installed code may still fail: monolog's `Logger` needs `Fiber`. |
| Symfony (skeleton + twig-bundle, `tests/apps/symfony-7.4`) | 7.4.22 | 1,749 of its 1,751 PHP files compile under rhp. `bin/console --version`, `list` and `debug:router` print what php prints; `about` differs in its Intl locale line (`n/a`). Every request of the request diff answers 500 instead. |
| Laravel (laravel/laravel, `tests/apps/laravel-13`) | 13.39.0 | Compiles only: 8,164 of its 8,165 PHP files compile under rhp. It does not boot: the container calls `spl_object_id()` on a closure, which rhp rejects, so every artisan command and request fails. |
| PHPUnit (`tests/apps/tools`) | 13.4.6 | All 29 checked commands print what php prints: `--version`, `--help`, the `--list-*` commands, configuration validation and migration, test runs (passing, failing and skipped tests, data providers, `--filter`, `--group`, `--stop-on-failure`, random order with a seed) and every output format (testdox, TeamCity, debug, JUnit, event log). |
| PHP-CS-Fixer (`tests/apps/tools`) | 3.95.28 | 24 of 25 commands print what php prints: `--version`, `list`, `help`, `describe`, `list-sets`, `list-files`, `check` and `fix` (dry runs with `--diff`, real fixes that write the same files, `--rules`, `--stop-on-violation`, a single path, without cache) in every output format, and runs with 2 parallel workers. The parallel runs write the same files but list them in the order the workers finish, which with rhp's slower workers is sometimes not php's: one of the two differed in the measured run. |
| Psalm (`tests/apps/tools`) | 7.0.3 | 22 of 24 commands print what php prints: `--version`, `--help`, single-threaded analysis with every checked output format but `xml`, `--show-info`, `--stats`, `--find-dead-code`, `--set-baseline` and `--alter` (psalter, with and without `--dry-run`). `--threads=2` stops at `Fiber` (amphp's event loop), and `--output-format=xml` fails because rhp's `property_exists()` doesn't see `DOMDocument`'s properties (`preserveWhiteSpace`, `formatOutput`). |
| PHPStan (`tests/apps/tools`) | 2.3.5 | 3 of 25 commands print what php prints: `--version`, `list` and `help analyse`. Every other command (`analyse`, `diagnose`, `dump-parameters`, `clear-result-cache`, ...) runs out of the default 128M memory limit while PHPStan tokenises files (rhp's `token_get_all()` needs about 2.4 times php's memory). With `--memory-limit=512M` analysis runs but reports an internal error: rhp's `[$a, $b] = $a;` overwrites `$a` before it reads `$b`. |
| Rector (`tests/apps/tools`) | 2.8.4 | 5 of 20 commands print what php prints: `--version`, `list`, `help process` and `list-rules` (text and JSON). Every `process` run fails on each file because `stream_wrapper_restore()` is missing, and parallel runs stop at `Random\Randomizer`, which rhp lacks. |

## Build and test

```sh
cargo build --release
cargo test --workspace           # unit tests, and the golden suite against recorded php output
target/release/rhp script.php
```

With a php on PATH (or `RHP_PHP` naming one), `cargo test` also compares rhp
against it live. Those comparisons expect php 8.5.12 and can fail on another
version.

The build needs Rust 1.99 or later and the C libraries rhp links: on macOS
php's own Homebrew dependencies (pcre2, openssl, curl, icu4c, libzip, gmp) and
the Command Line Tools; on Debian/Ubuntu the `-dev` packages that
[.github/actions/native-deps](.github/actions/native-deps) installs (see
[docs/design/libraries.md](docs/design/libraries.md)). Design notes are in
[docs/design/](docs/design/), starting with
[bytecode.md](docs/design/bytecode.md); the fixture apps are described in
[tests/apps/README.md](tests/apps/README.md).

## Licence

- rhp's own code is MIT OR Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).
- Translated php-src code is also PHP-3.01 or Zend-2.0; other third-party code keeps its licence, notably libmbfl (LGPL-2.1, vendored in `crates/rhp-mbfl-sys`, linked statically) and libgmp (LGPL-3.0, linked dynamically) ([THIRD-PARTY-NOTICES](THIRD-PARTY-NOTICES)).
- This product includes PHP software, freely available from <http://www.php.net/software/>, and the Zend Engine, freely available at http://www.zend.com.
- rhp is not affiliated with or endorsed by The PHP Group or Zend; it prints php's version and credit strings only for compatibility.
- Source only: anyone shipping an rhp binary must include LICENSE-*, THIRD-PARTY-NOTICES, the Rust crates' notices and libmbfl's source.
