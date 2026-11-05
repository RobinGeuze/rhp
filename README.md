# rhp

rhp is an engine for PHP 8.5 code, written in Rust. It parses PHP, compiles it
to a register-based bytecode and runs that on a VM, with the aim of printing
byte for byte what php 8.5 prints (the reference is php 8.5.12). Much of its
runtime is translated from php-src's C, and some php-src C is compiled in as is.
It is a very experimental hobby project: incomplete, and not for production use.

## Crates

All in `crates/`.

- **rhp-values**: PHP's value model (the zval): copy-on-write byte strings and ordered arrays,
  objects, references, closures, resources. Type juggling, php's float formatting, no I/O.
- **rhp-frontend**: a safe wrapper around the [mago-syntax](https://github.com/carthage-software/mago)
  parser that owns the parse arena, so the rest of rhp does not depend on the parser's API.
- **rhp-semantics**: PHP's operators (arithmetic, comparison, concatenation, bitwise, increment).
  Constant folding, the VM and the builtins all use it, so each rule is written once.
- **rhp-builtins**: the runtime library: the standard library's functions, constants and classes,
  streams, ini and about 30 extensions. Links PCRE2, libxml2, zlib and bzip2.
- **rhp-csrc**: compiles C that rhp uses unmodified (php-src's crypt() code, the Argon2 reference
  code) into static archives, with safe Rust wrappers and every C symbol renamed `rhp_*`.
- **rhp-bytecode**: the register-based instruction set: plain-data instructions plus side tables
  for each function, shared by the compiler and the VM without either depending on the other.
- **rhp-compiler**: compiles the AST to bytecode (registers, functions, classes, constant folding).
  Refuses what rhp does not support (property hooks, backticks, ...) with the file and line.
- **rhp-vm**: runs the bytecode: calls, classes and objects, generators, exceptions, references,
  include and eval. Also output buffering, reflection and the engine state of a request.
- **rhp-sapi**: the request layer: `run_request` runs one PHP request through a `Sapi` trait that
  the embedding server implements. Contains the CLI SAPI and a php-cgi compatible CGI SAPI.
- **rhp-cli**: the `rhp` binary: php's command line (a script, `-r`, `-d`, `-n`, `-f`, ...) on top
  of rhp-sapi. With `--cgi`, or when the binary is named `php-cgi`, it behaves as php-cgi.
- **rhp-difftest**: differential tests against a real php and php-cgi: php-src's .phpt suite, the
  fixture apps and extension sweeps. Also generates the tables taken from php-src, and this README's.

## PHP test support

php-src's own tests (every `.phpt` file at the php-8.5.12 tag) that rhp
passes, meaning its output equals php's. CI checks every listed test against
php 8.5 on each change (`tests/phpt/known-passing.txt`, see
[tests/phpt/README.md](tests/phpt/README.md)). A test that php skips, or that
the harness does not run, counts as not passing, so the percentages are of all
php-src tests.

<!-- phpt-table start: written by `rhp-difftest gen readme`; edit only the notes -->
| area | passing | total | % | notes |
|---|---:|---:|---:|---|
| Zend/tests | 2,290 | 5,396 | 42.4% | no Fiber; property hooks and the pipe operator are refused |
| tests | 506 | 895 | 56.5% | php-src's tests of the language, functions, classes and output |
| sapi/cli | 6 | 117 | 5.1% | only php's script-running options (no `-S`, `-h`, `-m`, `-l`, `-i`, `-a`) |
| sapi/cgi | 4 | 24 | 16.7% | `rhp --cgi` against php-cgi |
| ext/standard | 1,970 | 3,880 | 50.8% | no user stream wrappers, stream filters, `dns_*`, `mail`, `getimagesize` |
| ext/bcmath | 31 | 170 | 18.2% | no `BcMath\Number` |
| ext/bz2 | 5 | 27 | 18.5% | libbz2 1.0.8, linked statically |
| ext/ctype | 24 | 49 | 49.0% | |
| ext/curl | 61 | 180 | 33.9% | rhp's own HTTP/1.1 client on rustls, not libcurl |
| ext/date | 125 | 705 | 17.7% | system zoneinfo; no `DatePeriod`, `date_parse`, `strftime`, `createFromFormat` |
| ext/dom | 239 | 901 | 26.5% | rhp's own parser and tree, modelled on libxml2 2.9; no `Dom\` (HTML5) API |
| ext/filter | 40 | 129 | 31.0% | `filter_var` only |
| ext/gmp | 18 | 99 | 18.2% | on num-bigint, not libgmp |
| ext/hash | 55 | 83 | 66.3% | all 60 algorithms; no `mhash`, `hash_hkdf` |
| ext/iconv | 21 | 82 | 25.6% | a built-in charset table, not iconv(3) |
| ext/intl | 30 | 511 | 5.9% | `grapheme_*`, `Normalizer`, IDN and the `intl_*` error functions only (on ICU4X) |
| ext/json | 67 | 94 | 71.3% | |
| ext/libxml | 8 | 32 | 25.0% | on libxml2 |
| ext/mbstring | 68 | 446 | 15.2% | UTF-8, ASCII and ISO-8859-1 only |
| ext/opcache | 31 | 936 | 3.3% | the `opcache_*` functions only; no opcode cache or JIT |
| ext/openssl | 5 | 236 | 2.1% | X.509 parsing and public keys only |
| ext/pcntl | 22 | 66 | 33.3% | |
| ext/pcre | 152 | 174 | 87.4% | PCRE2, linked statically |
| ext/phar | 16 | 574 | 2.8% | enough to run composer.phar |
| ext/posix | 30 | 61 | 49.2% | |
| ext/random | 17 | 69 | 24.6% | the functions only; no `Random\Randomizer` or engines |
| ext/readline | 0 | 28 | 0.0% | `readline()`, `readline_add_history()`, `readline_clear_history()` only; no line editing |
| ext/reflection | 311 | 540 | 57.6% | |
| ext/simplexml | 103 | 165 | 62.4% | on rhp's own XML tree |
| ext/sockets | 3 | 119 | 2.5% | constants and options only; no `socket_create` |
| ext/spl | 478 | 796 | 60.1% | |
| ext/tokenizer | 46 | 54 | 85.2% | a port of the Zend scanner |
| ext/xml | 55 | 71 | 77.5% | on libxml2 |
| ext/xmlreader | 24 | 63 | 38.1% | on rhp's own XML tree |
| ext/xmlwriter | 50 | 51 | 98.0% | on libxml2 |
| ext/zip | 54 | 125 | 43.2% | `ZipArchive` only, in Rust |
| ext/zlib | 77 | 161 | 47.8% | system zlib; no incremental deflate/inflate contexts |
| other ext/ | 2 | 3,954 | 0.1% | the passing tests need none of the extension's code |
| other sapi/ | 0 | 217 | 0.0% | |
| **total** | **7,044** | **22,280** | **31.6%** | |

Not implemented, so counted under other ext/: calendar, com_dotnet, dba, dl_test, enchant, exif, ffi, fileinfo, ftp, gd, gettext, imap, ldap, mysqli, odbc, pdo, pdo_dblib, pdo_firebird, pdo_mysql, pdo_odbc, pdo_pgsql, pdo_sqlite, pgsql, session, shmop, skeleton, snmp, soap, sodium, sqlite3, sysvmsg, sysvsem, sysvshm, tidy, uri, xsl, zend_test; other sapi/: fpm, phpdbg.
<!-- phpt-table end -->

## Application support

Measured on 2026-11-05 against php 8.5.12; only what was checked is listed.

| application | version | what works |
|---|---|---|
| Composer | 2.11.1 (phar) | Runs from its phar: 27 of the 29 commands tried match php, and `composer.lock` and `vendor/` come out identical; `show --platform` and the install suggestion count differ (fewer extensions). It installs symfony-7.4's lock and refuses laravel-13's (no ext-session, ext-fileinfo). Installed code may still fail: monolog's `Logger` needs `Fiber`. |
| Symfony (skeleton + twig-bundle, `tests/apps/symfony-7.4`) | 7.4.22 | Compiles only: 1,749 of its 1,751 PHP files compile under rhp. It does not boot: every console command and request stops at `get_error_handler()`, a PHP 8.5 function rhp lacks. |
| Laravel (laravel/laravel, `tests/apps/laravel-13`) | 13.39.0 | Compiles only: 8,164 of its 8,165 PHP files compile under rhp. It does not boot: loading `vendor/autoload.php` fails on a class declared inside an `if`, so every artisan command and request fails. |

## Build and test

```sh
cargo build --release
cargo test --workspace           # unit tests, and the golden suite against recorded php output
target/release/rhp script.php
```

With a php on PATH (or `RHP_PHP` naming one), `cargo test` also compares rhp
against it live. Those comparisons expect php 8.5.12 and can fail on another
version.

The build links PCRE2 and libxml2 (macOS: Homebrew's pcre2 and the Command
Line Tools; Debian/Ubuntu: `pkg-config libpcre2-dev libxml2-dev`). Design
notes are in [docs/design/](docs/design/), starting with
[bytecode.md](docs/design/bytecode.md); the fixture apps are described in
[tests/apps/README.md](tests/apps/README.md).

## Licence

- rhp's own code is MIT OR Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).
- Translated php-src code is also PHP-3.01 or Zend-2.0; other third-party code keeps its licence ([THIRD-PARTY-NOTICES](THIRD-PARTY-NOTICES)).
- This product includes PHP software, freely available from <http://www.php.net/software/>, and the Zend Engine, freely available at http://www.zend.com.
- rhp is not affiliated with or endorsed by The PHP Group or Zend; it prints php's version and credit strings only for compatibility.
- Source only: anyone shipping an rhp binary must include LICENSE-*, THIRD-PARTY-NOTICES and the Rust crates' notices.
