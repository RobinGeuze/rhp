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
  out like php-src. Links PCRE2, OpenSSL, libcurl, libxml2, ICU, zlib, bzip2, libzip, GMP, libsodium, libintl and iconv, and opens libgd on first use.
- **rhp-csrc**: compiles C that rhp uses unmodified (php-src's crypt() code, timelib, the calendar conversions, libmagic with its
  magic database, the built-in web server's HTTP parser, lexbor, the Argon2 reference code) into static archives, with safe Rust wrappers and every C
  symbol renamed `rhp_*`.
- **rhp-mbfl-sys**: php-src's libmbfl (LGPL-2.1) and php_unicode.c (PHP-3.01), vendored and linked
  statically in a crate of their own. mbstring runs on them, through rhp-builtins' default-on `mbstring-libmbfl` feature.
- **rhp-vm**: runs the bytecode: calls, classes and objects, generators, exceptions, include and eval,
  streams. Also an opcache-style compiled-script cache (no optimizer or JIT) and a request's engine state.
- **rhp-sapi**: the request layer: `run_request` runs one PHP request through a `Sapi` trait that
  the embedding server implements. Contains the CLI SAPI, the built-in web server (`rhp -S`) and a php-cgi compatible CGI SAPI.
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
| Zend/tests | 5,272 | 5,396 | 97.7% | no Fiber (0/113); lazy objects 215/229 (no typed reference sources); the pipe operator `\|>` is refused |
| tests | 854 | 895 | 95.4% | php-src's tests of the language, functions, classes and output |
| sapi/cli | 88 | 117 | 75.2% | tests that spawn the php binary run rhp, the `-S` server tests included |
| sapi/cgi | 16 | 24 | 66.7% | `rhp --cgi` against php-cgi |
| ext/standard | 3,416 | 3,880 | 88.0% | user stream wrappers carry no filters |
| ext/bcmath | 64 | 170 | 37.6% | the `bc*` functions; no `BcMath\Number` |
| ext/bz2 | 16 | 27 | 59.3% | libbz2 1.0.8, linked statically |
| ext/calendar | 48 | 54 | 88.9% | php-src's calendar conversions, compiled in (rhp-csrc); the 6 others skip (32-bit) |
| ext/ctype | 38 | 49 | 77.6% | |
| ext/curl | 157 | 180 | 87.2% | on libcurl |
| ext/date | 690 | 705 | 97.9% | on timelib, compiled in (rhp-csrc) |
| ext/dom | 882 | 901 | 97.9% | the `DOMDocument` API and the `Dom\` API on libxml2, HTML5 parsing and CSS selectors on lexbor (rhp-csrc) |
| ext/fileinfo | 60 | 64 | 93.8% | php's bundled libmagic and magic database, compiled in (rhp-csrc); the 4 others skip |
| ext/filter | 125 | 129 | 96.9% | all functions and filters; `uri_parser_class` knows only `parse_url` (no ext/uri) |
| ext/gd | 278 | 336 | 82.7% | on libgd; php skips the others (bundled libgd, GD formats, 32-bit) |
| ext/gettext | 19 | 19 | 100.0% | on libintl (glibc's on Linux) |
| ext/gmp | 96 | 99 | 97.0% | on libgmp |
| ext/hash | 66 | 83 | 79.5% | all 60 algorithms; no `mhash`, `hash_hkdf` |
| ext/iconv | 80 | 82 | 97.6% | the system iconv |
| ext/intl | 464 | 511 | 90.8% | on ICU: every class and function, reported as loaded |
| ext/json | 94 | 94 | 100.0% | |
| ext/libxml | 21 | 32 | 65.6% | on libxml2 |
| ext/mbstring | 236 | 446 | 52.9% | on php-src's libmbfl (rhp-mbfl-sys) |
| ext/opcache | 815 | 876 | 93.0% | the compiled-script cache only; no optimizer, JIT or file cache |
| ext/openssl | 215 | 236 | 91.1% | on OpenSSL |
| ext/pcntl | 24 | 66 | 36.4% | |
| ext/pcre | 167 | 174 | 96.0% | PCRE2, linked statically |
| ext/pdo | 128 | 132 | 97.0% | PDO core; the driver tests run against pdo_sqlite through its `--REDIRECTTEST--` |
| ext/pdo_sqlite | 82 | 89 | 92.1% | `Pdo\Sqlite` on libsqlite3, linked statically |
| ext/phar | 19 | 574 | 3.3% | reads and runs phars (composer.phar); no write API |
| ext/posix | 32 | 61 | 52.5% | |
| ext/random | 68 | 69 | 98.6% | the functions, `Random\Randomizer` and the engines |
| ext/readline | 16 | 28 | 57.1% | on libedit: the functions, history, the callback interface and the interactive shell (`php -a`) |
| ext/reflection | 480 | 540 | 88.9% | |
| ext/session | 250 | 269 | 92.9% | |
| ext/simplexml | 162 | 165 | 98.2% | on libxml2 |
| ext/sockets | 58 | 119 | 48.7% | the socket calls, `AddressInfo` and the constants; no `socket_sendmsg`/`socket_recvmsg`, no multicast options |
| ext/sodium | 31 | 31 | 100.0% | on libsodium, linked statically |
| ext/spl | 646 | 796 | 81.2% | |
| ext/sqlite3 | 93 | 101 | 92.1% | `SQLite3` on the libsqlite3 pdo_sqlite links |
| ext/tokenizer | 51 | 54 | 94.4% | a port of the Zend scanner |
| ext/xml | 61 | 71 | 85.9% | on libxml2 |
| ext/xmlreader | 58 | 63 | 92.1% | on libxml2 |
| ext/xmlwriter | 50 | 51 | 98.0% | on libxml2 |
| ext/zip | 120 | 125 | 96.0% | `ZipArchive` on libzip |
| ext/zlib | 133 | 161 | 82.6% | system zlib; no incremental deflate/inflate contexts |
| other ext/ | 1 | 2,858 | 0.0% | the 1 passing is an ext/uri test that needs none of its code |
| other sapi/ | 0 | 217 | 0.0% | |
| **total** | **16,840** | **22,219** | **75.8%** | |

Not implemented, so counted under other ext/: com_dotnet, dba, dl_test, enchant, exif, ffi, ftp, imap, ldap, mysqli, odbc, pdo_dblib, pdo_firebird, pdo_mysql, pdo_odbc, pdo_pgsql, pgsql, shmop, skeleton, snmp, soap, sysvmsg, sysvsem, sysvshm, tidy, uri, xsl, zend_test; other sapi/: fpm, phpdbg.
<!-- phpt-table end -->

## Application support

Measured on 2026-11-08 against php 8.5.12; only what was checked is listed.

| application | version | what works |
|---|---|---|
| Composer | 2.11.1 (phar) | `composer_diff.py`: 29 commands on a small project (`install`, `update`, `require`, `remove`, `reinstall`, `audit`, `archive`, `init`, ...), downloads over the stream layer instead of curl, extraction with ZipArchive instead of `unzip`, and installs of the Symfony 7.4 fixture (Flex plugin, auto-scripts and `cache:clear` run by rhp): every command prints what php prints and leaves the same `composer.lock` and `vendor/` (`show --platform` compared on the extensions both engines load, `lib-*` lines included, the OpenSSL header version masked). On the Laravel fixture, `install`, `dump-autoload -o` (php's `installed.php` and `autoload_static.php`), `package:discover`, `require`, `update`, `outdated` and the app's `artisan --version` print and write what php does, with `ext-fileinfo` required (no platform requirement ignored). `create-project laravel/laravel` (with `ext-fileinfo` ignored) runs the app's migration and writes the same SQLite database; at the last check it counted 67 suggestions where php counts 51 (before ext/sockets, sodium, calendar and gettext existed; not re-measured). |
| Symfony (skeleton + twig-bundle + mime, `tests/apps/symfony-7.4`) | 7.4.22 | All 1,835 of its PHP files compile under rhp. Its 32-request corpus (sessions, flashes, redirects, URL generation, cookies, error pages, streamed and file responses with Range and HEAD, ETags, uploads, Twig, PUT/PATCH) answers like php-cgi in all three passes (`default`, `debug` with `APP_DEBUG=1`, `cold` from an empty cache), and the files it writes (Twig cache, logs, compiled container, the routing `*.meta` files of a cold cache) are identical. All 33 `bin/console` commands of its console battery (`about`, `list`, `debug:router`, `debug:container`, `debug:twig`, `lint:*`, `cache:clear`, `cache:warmup`, `cache:pool:clear`, ...) print and write what php does. |
| Symfony (skeleton + twig-bundle + mime, `tests/apps/symfony-8.1`) | 8.1.11 | All 1,901 of its PHP files compile under rhp. Its 32-request corpus (sessions, flashes, redirects, URL generation, cookies, error pages, streamed and file responses with Range and HEAD, ETags, uploads, Twig, PUT/PATCH) answers like php-cgi in all three passes (`default`, `debug` with `APP_DEBUG=1`, `cold` from an empty cache), and the files it writes (cold-cache routing `*.meta` included) are identical. All 33 `bin/console` commands of its console battery print and write what php does. |
| Laravel (laravel/laravel, `tests/apps/laravel-13`) | 13.39.0 | All 8,167 of its PHP files compile under rhp. Its 23-request corpus (CSRF flows, Blade components and slots, `Js::from`/`@js`, a sync job and a queued closure, `CarbonPeriod`, `Rule::enum` with old input, multipart uploads, a throwing route, `Number::*`) answers like php-cgi in both passes (`default`, `debug` with `APP_DEBUG=true`, so the exception page too), with the same compiled views and session files. Its own `vendor/bin/phpunit` passes (2 tests) as under php. Its console battery: all 76 commands print and write what php does: the 56 `artisan` commands (`vendor:publish --tag=laravel-errors` included), the 10 `tinker --execute` snippets and the 10 workload scripts (Blade, security, validation, mail, HTTP, queue and events, database, `Storage`, `Number::*`, ...). Eloquent and migrations on SQLite match php. |
| WordPress (+ SQLite Database Integration 3.1.3, `tests/apps/wordpress-7.1`) | 7.1.4 | All 1,553 of its PHP files (core, the SQLite plugin, the fixture's config and mu-plugin) compile under rhp. Its 61-request corpus (archives, feeds, embeds, REST, sitemaps, login, xmlrpc, a comment, admin-ajax and 19 wp-admin screens logged in through a test-only mu-plugin) answers like php-cgi, and the SQLite database it writes dumps the same. Of the 73 wp-cli 2.12.0 commands of its console battery (post, user, option, term, comment, menu, widget, rewrite, cron, role, export, search-replace, db, `help`, `eval` and `eval-file` probes), 68 print what php prints and leave the same database (`media import` of a PNG and a JPEG included). `i18n make-pot` needs `mb_ereg`, one probe sees the missing ext/exif and ext/sodium, and two print other `stdClass` object ids. With stdout redirected to a file, `help` keeps a boot-time deprecation that php's pager overwrites (rhp does not seek a dup'd `STDOUT`). |
| PHPUnit (`tests/apps/tools`) | 13.4.6 | All 34 checked commands print what php prints: `--version`, `--help`, the `--list-*` commands, configuration validation and migration, test runs (passing, failing and skipped tests, data providers, `--filter`, `--group`, `--stop-on-failure`, random order with a seed), every output format (testdox, TeamCity, debug, JUnit, event log), and a synthetic suite of 331 tests (mocks and stubs, failure diffs, warnings, notices and deprecations, `#[RunInSeparateProcess]`, `#[Depends]`, a 300-case data provider). The nightly big tier runs that suite with `--process-isolation`, `--debug`, `--teamcity` and `--repeat 3`, also as php does. |
| PHP-CS-Fixer (`tests/apps/tools`) | 3.95.28 | All 25 checked commands print what php prints: `--version`, `list`, `help`, `describe`, `list-sets`, `list-files`, `check` and `fix` (dry runs with `--diff`, real fixes that write the same files, `--rules`, `--stop-on-violation`, a single path, without cache) in every output format, and with 2 parallel workers (the harness compares the per-file blocks of a parallel run in any order: php's own order changes from run to run). The nightly big tier fixes 57 files of symfony/console with `@Symfony`, `@Symfony:risky`, `@PER-CS` and `@PHP8x4Migration`, sequentially and with 4 workers, like php. |
| Psalm (`tests/apps/tools`) | 7.0.3 | All 24 checked commands print what php prints: `--version`, `--help`, analysis single-threaded and with `--threads=2` (amphp's event loop on fibers), every checked output format including `xml`, `--show-info`, `--stats`, `--find-dead-code`, `--set-baseline` and `--alter` (psalter, with and without `--dry-run`). The nightly big tier (error level 1) on `vendor/nikic/php-parser/lib` and `vendor/symfony/console`: single-threaded, the same issues as php; with 4 threads php's own output varies from run to run (issue order, an issue reported in some runs only), and rhp's varies the same way. |
| PHPStan (`tests/apps/tools`) | 2.3.5 | All 25 checked commands print what php prints, with the default 128M memory limit: `--version`, `list`, `help analyse`, `diagnose`, `dump-parameters`, the result-cache commands, and `analyse` at several levels, on one file, in parallel, with `--generate-baseline` and in every error format. rhp cannot load PHPStan's optional `phpstan_turbo` extension, so PHPStan spawns its worker where php forks it; the harness masks the word in `analyse -v`'s peak-memory line. The nightly big tier analyses `vendor/symfony/console` (682 errors at level 6, in parallel and in one process, and level max) like php. |
| Rector (`tests/apps/tools`) | 2.8.4 | All 20 checked commands print what php prints: `--version`, `list`, `help process`, `list-rules` (text and JSON) and `process` (dry runs with and without diffs, real runs that write the same files, a single file, `--only`, `--max-changes`, `--rules-summary`, `--clear-cache`, `--debug`, 2 parallel workers, and JSON, GitHub and GitLab output). The nightly big tier runs the PHP 8.4 set and five prepared sets over symfony/console (154 changed files), sequentially and with 4 workers, like php. |

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
php's own Homebrew dependencies (pcre2, openssl, curl, icu4c, libzip, gd, gmp) and
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
