# C libraries: what rhp links and how

Status: policy (Phase L). It replaces the "pure-Rust by default" stance of
[`extensions.md`](./extensions.md) §4 and §6.

## The rule

- **Where php calls an external C library, rhp calls the same library.** The
  same library at the same version gives the same bytes; no reimplementation
  matched php on both reference machines (the mac and CI link different
  versions of libxml2, ICU, PCRE2 and curl).
- **Where php runs its own C** with no Zend dependency (libmbfl, timelib,
  crypt), rhp compiles that C from the pinned php-src tag, in
  `crates/rhp-csrc` (see [Compiled from source](#compiled-from-source-rhp-csrc)).
  LGPL C such as libmbfl goes in its own crate (see [Licences](#licences)).
- **Where php-src owns the data** (stubs, entity tables, Bison tables), rhp
  generates it from php-src.
- A pure-Rust crate replaces library behaviour only with evidence: it is
  spec-identical (hashes, CRC-32), or it passed a prototype comparison
  against php.
- **Version constants come from the library rhp linked:** a header macro
  where php reads a header macro, a runtime getter where php calls one.
  `PCRE_VERSION` comes from `pcre2_config()`, `PCRE_VERSION_MAJOR`/`MINOR`
  from pcre2.h; `LIBXML_VERSION`/`LIBXML_DOTTED_VERSION` from xmlversion.h
  and `LIBXML_LOADED_VERSION` from `xmlParserVersion`. `version.rs` keeps
  hard-coded values only for libraries rhp does not link yet; each goes when
  its library is linked.

## Linking: static by default

| Kind | Linked | Examples |
|---|---|---|
| Code rhp compiles: php-src C, frozen vendored libraries | static | php-src's crypt C, argon2 (in `rhp-csrc`); later timelib, oniguruma, and libmbfl in its own crate (LGPL) |
| A third-party library whose static archive is the build php links | static | PCRE2, libcurl; libxml2 on Linux; later gmp, icu4c@79, openssl@4 |
| The listed exceptions below | dynamic | |

A static archive is "the build php links" when it comes from the same package
as the shared library php loads: Homebrew's `.a` next to the `.dylib` on the
mac, the `-dev` package's `.a` on Debian/Ubuntu.

**Dynamic only for these.** Each one is a reviewed decision; add to the list
rather than linking dynamically quietly.

- OS libraries that php also takes from the OS and that ship no static
  archive. macOS `/usr/lib`: libxml2, libxslt, libiconv, libz, libbz2,
  libedit, libffi, and what Homebrew's libcurl takes from the SDK
  (libicucore, libldap, libgssapi_krb5, libresolv, frameworks). Linux:
  glibc's iconv.
- Libraries with no static archive of php's version: Homebrew's libzip.
- libcurl's private dependencies on Debian/Ubuntu (nghttp2, libidn2,
  librtmp, OpenLDAP, libssh2, libpsl, OpenSSL, MIT Kerberos, zstd, brotli,
  zlib): libcurl.a is linked statically, they are not. MIT Kerberos ships no
  static archive, and librtmp and libidn2 are LGPL.

Today rhp-builtins links:

| Library | macOS | Debian/Ubuntu |
|---|---|---|
| PCRE2 (libpcre2-8) | static, Homebrew pcre2 | static, libpcre2-dev (CI: php's release, see below) |
| libxml2 | dynamic, the SDK's /usr/lib/libxml2.2.dylib (php's; found with `xcrun --show-sdk-path`) | static, libxml2-dev (CI: php's release, see below) |
| zlib (via libz-sys; also crc32, adler32) | dynamic, /usr/lib/libz | dynamic, zlib1g |
| bzip2 (bzip2 `=0.5.2`, feature `static`) | static, bzip2-sys's bundled C libbz2 1.0.8 | static, bzip2-sys's bundled C libbz2 1.0.8 |
| php-src crypt C (via rhp-csrc) | static, compiled from php-8.5.12 | the same |
| argon2 (via rhp-csrc) | static, compiled from release 20190702 | the same |
| libcurl | static, Homebrew curl (keg-only), with its Homebrew dependencies (nghttp2, nghttp3, ngtcp2, libssh2, libpsl, openssl@4, zstd, brotli) static | static, libcurl4-openssl-dev; its dependencies dynamic (see above) |

zlib and bzip2 predate this policy and still go through their -sys crates.
bzip2 is built from bzip2-sys's 1.0.8 source, not taken from macOS's
/usr/lib/libbz2 as the list above would have it. Known gap: libz-sys builds
its bundled zlib when it finds no system zlib, a silent fallback.
rhp-csrc's code is the same on every machine, so it needs no row in
`native.rs` or the native-deps action, only a C compiler (and `nm` for its
test).

**Never a silent fallback.** A missing library or static archive is a build
error that says what to install. rhp does not use -sys crates that quietly
build a bundled copy when the system library is missing (pcre2-sys did; a
binary that links two PCRE2 copies is wrong too, so rhp-difftest uses
`rhp_builtins::pcre::Pattern` rather than the `pcre2` crate).

`RHP_<LIB>_LINK=static|dynamic` (e.g. `RHP_PCRE2_LINK=dynamic`) overrides the
policy for one build, for experiments.

The helper links a static archive from a directory under `OUT_DIR` that holds
only that archive. Searching the library's own libdir would not work: ld64
has no `-Bstatic`, so `-lpcre2-8` takes Homebrew's `libpcre2-8.dylib` next
to the `.a`.

A static archive's own dependencies come from its .pc file (`Libs.private`
and the modules in `Requires.private`):

- macOS: each one a Homebrew formula ships as `lib<name>.a` is linked
  statically the same way, with its own dependencies (the .pc files of
  Homebrew's curl name its dependencies' Cellar paths, which the helper maps
  to the formulae's opt prefixes: the versions php's dylibs load). Every
  other one must be an OS library in the SDK, else the build fails; `-framework`s
  are linked as frameworks.
- Linux: the archive's direct dependencies are linked dynamically (its
  `Libs.private`, plus the `Libs` of its `Requires.private` modules), not
  `pkg-config --static`'s whole closure, whose libraries need not all have
  `.so` links installed.

The helper drops the library itself from these: a second, dynamic `-l` for
it would override the static one.

**Checks.** `crates/rhp-cli/tests/ref_build.rs` requires the "linked" section
of `tools/capture-ref-build.php` to print the same under rhp and the host's
php 8.5. CI prints what its php links (the setup-php action's last step).

On CI, php is not always the distro's build. setup-php's php on Ubuntu 26.04
ships its own `libpcre2-8.so` (PCRE2 10.47, against which it was compiled)
next to the distro's 10.46, outside dpkg. It becomes the copy php loads once
`ldconfig` runs, so the setup-php action runs `ldconfig` straight away. The
native-deps action then compares the PCRE2 and libxml2 versions php reports
(`PCRE_VERSION`, `LIBXML_LOADED_VERSION`) with the distro's `-dev` packages.
For each that differs, `build-from-source.sh` builds php's release from the
upstream tarball, checked against `.github/actions/native-deps/sha256sums`
(PCRE2: the checksums Homebrew's formula pins; libxml2: GNOME's published
ones), static only, and points `PKG_CONFIG_PATH` at it. libxml2 is
configured like Debian/Ubuntu's build where that changes parsing (zlib on,
ICU off). A release with no recorded checksum fails the job; add its
checksum.
The `php` input names a php that is not the one on PATH (the nightly
sanitizer build uses php-src's bundled PCRE2).
`otool -L target/release/rhp` (or `ldd`) shows no libpcre2 or libcurl (and
on Linux no libxml2); `nm` shows the `pcre2_*` and `curl_*` symbols.
rhp-builtins' `pcre2_is_linked_statically`, `libxml2_is_linked_statically`
and `libcurl_is_linked_statically` tests check its own test binary, which is
linked straight from the crate. The last also requires `curl_version_info()`
to report the version of the curl.h rhp compiled against.

php links libcurl from Homebrew's keg-only curl on the mac, not the SDK's
older libcurl, whose error texts differ: the CURL row names the formula, so
the build never falls back to a bare `-lcurl`. rhp declares libcurl's API
itself (`ext/curl/sys.rs`) instead of using curl-sys, which links `-lcurl`
from the SDK on macOS and builds its bundled libcurl when pkg-config finds
none. ext/curl's constants are php's names and version gates
(`rhp-difftest gen curl-consts`, from curl.stub.php), evaluated against the
linked curl.h at build time (`ext/curl/consts.c`).

## Compiled from source: rhp-csrc

`crates/rhp-csrc` holds the C rhp compiles itself, with `cc`, into static
archives, except LGPL C (see [Licences](#licences)):

- **php-src's own C**, copied unmodified into `vendor/php-src` by
  `crates/rhp-csrc/tools/sync-php-src-c.sh <php-src-dir>` from a checkout at
  the commit `tests/phpt/php-src.ref` pins. Today: ext/standard's
  crypt_freesec, crypt_blowfish, crypt_sha256 and crypt_sha512. All php-src
  C shares one stand-in for the Zend and PHP headers, `shim/zend_shim.h`, and
  builds with php's flags (`php_src()` in `build.rs`).
- **Frozen libraries**, whose last release is the one php links: the argon2
  reference C 20190702 in `vendor/argon2`, copied by
  `crates/rhp-csrc/tools/sync-argon2.sh` from the release tarball, checked
  against the SHA-256 Homebrew's formula pins. A library that still has
  releases is linked from the system instead (above), so rhp follows the
  version php links.

Both scripts write `VERSION` and `SHA256SUMS` next to the files and change
nothing when rerun. Vendored files are never edited; what php's build
provides around them goes in the shim or in `glue/`.

**The rhp_ prefix.** Every global symbol the C defines gets an `rhp_`
prefix: each archive force-includes `glue/prefix_<lib>.h`, a list of
`#define`s, so another static C library in the binary can neither clash with
it nor bind to it. The crate's `archives_define_only_prefixed_symbols` test
runs `nm` over the archives and fails on an unprefixed symbol or an
unexpected archive. Rust calls the prefixed names through safe wrappers in
`src/`, one module and one cargo feature per library.

php reports no version for its crypt C or for libargon2
(`ARGON2_VERSION_NUMBER` is the algorithm version, the `v=19` in a hash), so
neither has a row in the "linked" section of `tools/capture-ref-build.php`.

## Licences

- Never GPL.
- Permissive licences (MIT, BSD, Apache-2.0, ISC, Zlib, curl, Unicode-3.0,
  CC0-1.0, public domain) may be linked or vendored; their notices go in
  `THIRD-PARTY-NOTICES`.
- LGPL may be linked statically or vendored: rhp ships its source, so users
  can relink. Keep LGPL code in its own crate with an exact licence
  expression (libmbfl: `rhp-mbfl-sys`, `LGPL-2.1-only AND PHP-3.01 AND MIT`).
  Never compile or transliterate libbcmath's LGPL arithmetic.
- Code ported from php-src is under the PHP License 3.01 (Zend Engine
  License 2.00 for `Zend/`). Keep both notices and php's acknowledgement in
  `THIRD-PARTY-NOTICES`.
- Adding or vendoring a library means adding its entry to
  `THIRD-PARTY-NOTICES` in the same change.

## Threading

One VM per OS thread (see the web SAPI plan). For C libraries:

- **Process-wide init happens once**, under `std::sync::Once` (e.g.
  `xmlInitParser`, `curl_global_init`). PCRE2 needs none.
- **Everything else is per thread.** PCRE2's compile and match contexts, JIT
  stack, settings and pattern cache live in a `thread_local!` in
  `rhp-builtins/src/pcre/mod.rs`, as php keeps them in TLS globals. Library
  handles are `!Send` and stay with their VM.
- **Per-thread hooks are installed when each VM starts.** libxml2's generic
  error handler, structured error handler and file I/O hooks are per thread
  in both 2.9 and 2.15: `ext::libxml::reset_state` sets php's handlers on the
  VM's thread, as php's RINIT does.
- **Global hooks** a library offers only process-wide (libxml2's entity
  loader, libxslt's error function) are set once, to a dispatcher that reads
  thread-local VM state: libxml2's entity loader runs php's loader on a
  thread whose VM installed the glue while a builtin is running, and
  libxml2's own otherwise. Nothing on libxml2 loads external entities yet
  (ext/xml keeps php's no-DTD-loading options), so the dispatcher and the
  stream input hooks are first exercised when DOM moves to libxml2 (L3c).
- **No process-global side effects from a VM:** no `setlocale` (use
  `newlocale`/`uselocale`), no relative paths handed to a library that opens
  files itself (absolutize against the VM's cwd), no `putenv("TZ=…")`.
- **C→Rust callbacks** (`ext/libxml/ffi.rs`):
  - a builtin runs the C call inside `ffi::with_host(host, fname, …)`, which
    pushes its `Host` on a thread-local stack;
  - every `extern "C"` callback runs its body in `ffi::guard`, which catches
    a panic (it must not unwind into C), stops the C operation and resumes
    the panic after the C call returns;
  - a callback reaches PHP through `ffi::call_host`. What PHP throws is
    stashed and rethrown by `with_host` once the C call returns: php
    longjmps out of the library, rhp cannot. An exception leaves the C
    operation running, as in php, where handlers then see `EG(exception)`
    and do nothing (`ffi::aborted`); `exit` and other non-exceptions stop it
    through the function the builtin registered (`ffi::set_stop`, e.g.
    `xmlStopParser`). That leaves the stopped state behind: after `exit` in
    an ext/xml handler, a shutdown function sees error 111 (user stop) and
    the byte index libxml2 resets, where php's longjmp leaves the parser as
    it was;
  - printf-style callbacks are C functions in the shim that format with
    `vasprintf` (no length limit, like php's `vspprintf`) and hand Rust the
    finished message;
  - libcurl's callbacks (ext/curl) use the same layer. php's callbacks
    neither stop a transfer on an exception nor on `exit`: later callbacks
    see `EG(exception)` and return their defaults (a write callback the
    length it was given), so rhp registers no stop function and the transfer
    runs to its end before `curl_exec`/`curl_multi_exec` rethrows. Each
    callback's `void *` is its handle's `RefCell`, kept alive by the builtin
    driving the transfer; freeing a handle clears the pointers first;
  - a handle whose free can call back into PHP (an XMLWriter flushing to a
    stream) is freed in the class's native `dtor_obj`
    (`rhp_builtins::object_dtor`), which the engine runs with a host before
    `__destruct`, as php does. The object-free hook has no host; it frees
    only what php's `dtor_obj` would have skipped.
- **Guard aborts.** A C library that can abort the process (libgmp on huge
  sizes) needs size checks first: with one VM per thread, an abort kills
  every request.

## Adding a library

1. Add a row to `LIBS` in `crates/rhp-builtins/build/native.rs`: pkg-config
   name, Homebrew formula (or `macos_sdk` for a library php takes from the
   macOS SDK), link name, the header that defines its version macros, the
   macros to export, and the link mode per OS. The helper finds it with
   pkg-config, or on macOS without pkg-config through
   `/opt/homebrew/opt/<formula>` (`/usr/local/opt/<formula>`) or the SDK
   (`xcrun --show-sdk-path`); checks the header's version against the .pc
   file; links it; exports each macro as `env!("RHP_<MACRO>")`; and returns
   the include directory, for a C shim built against the same headers.
2. Declare the functions rhp calls in a hand-written `extern "C"` block
   (no bindgen). Put struct access and macros behind a small C shim with
   `_Static_assert` layout checks if needed (libxml2: `ext/libxml/sys.rs`
   and `shim.c`; its structs are opaque to Rust because their layouts
   differ between releases).
3. Add the `-dev` package to `.github/actions/native-deps`. If CI's php can
   link a release the distro has no `-dev` package for, add the library to
   its version comparison and to `build-from-source.sh`.
4. Read every version constant the way php's source reads it, and add the
   constants to the "linked" section of `tools/capture-ref-build.php`.
5. Add the licence to `THIRD-PARTY-NOTICES`.

## Adding C to rhp-csrc

1. php-src C: add the files to `FILES` in
   `crates/rhp-csrc/tools/sync-php-src-c.sh` and run it on a php-src
   checkout at the pinned commit. A frozen library: write a
   `crates/rhp-csrc/tools/sync-<lib>.sh` like `sync-argon2.sh` (release
   tarball, pinned SHA-256, file list) and run it.
2. Add a cargo feature and its build to `build.rs`; php-src C goes through
   `php_src()`. Missing Zend or PHP definitions go in `shim/zend_shim.h`;
   never add a second shim.
3. Add `glue/prefix_<lib>.h` with an `rhp_` `#define` for every global
   symbol the C defines, force-include it (`-include`), and add the archive
   to the prefix test's list.
4. Wrap the C in a safe module in `src/` that takes what php passes and
   returns what php's C returns.
5. Add each file's licence, as its header states it, to the crate's
   `license` expression, its `NOTICE` and `THIRD-PARTY-NOTICES`.

## Generated tables

Tables php-src owns are generated, committed, and checked by
`rhp-difftest gen <name> --check` (`crates/rhp-difftest/src/gen/`):

| Generator | Source | Output | Checked |
|---|---|---|---|
| `html-tables` | php-src `ext/standard/html_tables/` (+ `html_tables.h` for HTML5 ties) | `rhp-builtins/src/html_tables.rs` | CI job `generated` (php-src checkout) |
| `parser` | the release tarball (`tools/php-dist.sha256`): Bison's `zend_language_parser.{c,h}`, `zend_ini_parser.{c,h}`; the numbers of the grammar rules whose actions rhp runs | `ext/tokenizer/ids.rs`, `zend/*_parser_tables.rs` | CI job `generated` (tarball cached on the pin) |
| `dumps` | the reference php: `tools/capture-ref-build.php tables`, plus constants gated on a newer library (`CONST_EXTRAS`) | `ini_table.rs`, `const_modules.rs` | `cargo test` on the reference host |
| `curl-consts` | php-src `ext/curl/curl.stub.php` (names, `@cvalue`, `LIBCURL_VERSION_NUM` gates, `#[\Deprecated]`) and `curl_private.h` | `ext/curl/consts.rs`, `ext/curl/consts.c` (values from the linked curl.h) | CI job `generated` (php-src checkout) |
