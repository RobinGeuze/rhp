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
  LGPL C (libmbfl) goes in a crate of its own, `crates/rhp-mbfl-sys` (see
  [Licences](#licences)).
- **Where php-src owns the data** (stubs, entity tables, Bison tables), rhp
  generates it from php-src.
- A pure-Rust crate replaces library behaviour only with evidence: it is
  spec-identical (hashes, CRC-32), or it passed a prototype comparison
  against php.
- **Version constants come from the library rhp linked:** a header macro
  where php reads a header macro, a runtime getter where php calls one.
  `PCRE_VERSION` comes from `pcre2_config()`, `PCRE_VERSION_MAJOR`/`MINOR`
  from pcre2.h; `LIBXML_VERSION`/`LIBXML_DOTTED_VERSION` from xmlversion.h
  and `LIBXML_LOADED_VERSION` from `xmlParserVersion`; `INTL_ICU_VERSION`/
  `INTL_ICU_DATA_VERSION` from ICU's uvernum.h;
  `OPENSSL_VERSION_TEXT`/`NUMBER` from opensslv.h and phpinfo's "OpenSSL
  Library Version" from `OpenSSL_version()`; `ICONV_VERSION` from
  `_libiconv_version` (macOS) or `gnu_get_libc_version()` (glibc);
  `GMP_VERSION` from libgmp's `gmp_version`;
  `ZipArchive::LIBZIP_VERSION` from `zip_libzip_version()`, phpinfo's
  "Libzip version" from zipconf.h.
  `version.rs` keeps hard-coded values only for libraries rhp does not link
  yet; each goes when its library is linked.

## Linking: static by default

| Kind | Linked | Examples |
|---|---|---|
| Code rhp compiles: php-src C, frozen vendored libraries | static | php-src's crypt C, timelib, argon2 (in `rhp-csrc`); libmbfl and php_unicode.c (in `rhp-mbfl-sys`, LGPL); later oniguruma |
| A third-party library whose static archive is the build php links | static | PCRE2, ICU, OpenSSL, libcurl, libxml2 on Linux |
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
- Libraries with no static archive of php's version: libzip (Homebrew's
  libzip and Debian/Ubuntu's libzip-dev ship only the shared library).
- LGPL libraries taken from the system: libgmp (LGPL-3.0+ or GPL-2.0+).
  Homebrew and libgmp-dev ship a `libgmp.a`, but linking it would put LGPL
  code into rhp's binary; linked dynamically, the user can replace it (see
  [Licences](#licences)).
- The C++ runtime a static C++ library needs (ICU), which php also loads
  dynamically. macOS: the SDK has no static libc++, only /usr/lib/libc++.
  Linux: Ubuntu ships libstdc++.a, but libstdc++ is GPL-3.0 with the GCC
  Runtime Library Exception, and "never GPL" (see [Licences](#licences))
  keeps it out of the binary; the system's libstdc++.so is used, as libgcc_s
  is for Rust's std. ICU's .pc files leave the runtime to the linker
  driver, so build.rs names it.
- libcurl's private dependencies on Debian/Ubuntu (nghttp2, libidn2,
  librtmp, OpenLDAP, libssh2, libpsl, MIT Kerberos, zstd, brotli, zlib):
  libcurl.a is linked statically, they are not. MIT Kerberos ships no
  static archive, and librtmp and libidn2 are LGPL. libcurl itself uses
  the OpenSSL row's static copy, but the shared libssh2 loads the distro's
  libssl.so and libcrypto.so: the OpenSSL static-link test checks the
  binary's own NEEDED entries, not ldd's closure.

Today rhp-builtins links:

| Library | macOS | Debian/Ubuntu |
|---|---|---|
| PCRE2 (libpcre2-8) | static, Homebrew pcre2 | static, libpcre2-dev (CI: php's release, see below) |
| libxml2 | dynamic, the SDK's /usr/lib/libxml2.2.dylib (php's; found with `xcrun --show-sdk-path`) | static, libxml2-dev (CI: php's release, see below) |
| ICU (libicui18n, libicuuc, libicudata) | static, Homebrew icu4c@79 (keg-only; php's formula) | static, libicu-dev (CI: 78.2, the release php links) |
| OpenSSL (libcrypto, libssl) | static, Homebrew openssl@4 | static, libssl-dev |
| zlib (via libz-sys; also crc32, adler32) | dynamic, /usr/lib/libz | dynamic, zlib1g |
| iconv | dynamic, the SDK's /usr/lib/libiconv.2.dylib | glibc's (libc) |
| bzip2 (bzip2 `=0.5.2`, feature `static`) | static, bzip2-sys's bundled C libbz2 1.0.8 | static, bzip2-sys's bundled C libbz2 1.0.8 |
| php-src crypt C (via rhp-csrc) | static, compiled from php-8.5.12 | the same |
| timelib 2022.17 with php's embedded tz database 2026.5 (via rhp-csrc) | static, compiled from php-8.5.12's ext/date/lib | the same |
| argon2 (via rhp-csrc) | static, compiled from release 20190702 | the same |
| libcurl | static, Homebrew curl (keg-only), with its Homebrew dependencies (nghttp2, nghttp3, ngtcp2, libssh2, libpsl, zstd, brotli) static; OpenSSL is the row above | static, libcurl4-openssl-dev; its dependencies dynamic (see above), OpenSSL the row above |
| libmbfl, php_unicode.c (via rhp-mbfl-sys; feature `mbstring-libmbfl`, on by default) | static, compiled from php-8.5.12 | the same |
| libgmp | dynamic, Homebrew gmp | dynamic, libgmp-dev |
| libzip | dynamic, Homebrew libzip (its own zlib, bzip2, xz and zstd links) | dynamic, libzip-dev |

ICU's data library is most of the size: linking ICU statically adds about
35 MB to `rhp` (13.5 MB → 48 MB on the mac with ICU 79.1, 33.5 MB of it
libicudata). ICU renames its C symbols with the major version
(`ubrk_open_79`); `ext/intl/sys.rs` appends the `U_ICU_VERSION_MAJOR_NUM`
of the linked uvernum.h, and build.rs refuses an ICU built with
`U_DISABLE_RENAMING`. Homebrew names the formula by major version: when
php's formula moves to a newer `icu4c@N`, the `ICU_*` rows in `native.rs`
follow (ref_build's "linked" section fails until they do).

iconv has no row in `native.rs`: there is no .pc file or version macro to
check (php reads `_libiconv_version` or `gnu_get_libc_version()` at run
time), and on Linux it is part of libc. `ext/iconv/sys.rs` links it, and its
`links_the_library_php_links` test checks with `dladdr` that `iconv_open`
comes from that library.

zlib and bzip2 predate this policy and still go through their -sys crates.
bzip2 is built from bzip2-sys's 1.0.8 source, not taken from macOS's
/usr/lib/libbz2 as the list above would have it. zlib is a listed
exception to "static where possible": libz-sys can only link the system
zlib dynamically (its `static` mode builds its own copy), so rhp links
/usr/lib/libz on macOS and zlib1g on Linux dynamically, the library php
links. libz-sys would build its bundled zlib when it finds no system one;
rhp-builtins' build.rs fails instead (libz-sys reports the bundled build as
`DEP_Z_ROOT`), and the `zraw` test `links_the_library_php_links` checks
with `dladdr` that `zlibVersion` comes from the system library.
rhp-csrc's and rhp-mbfl-sys's code is the same on every machine, so it
needs no row in `native.rs` or the native-deps action, only a C compiler
(and `nm` for their tests).

**OpenSSL goes through openssl-sys**, for its declarations (no bindgen, no
`vendored` feature); `ext/openssl/sys.rs` declares what it lacks. openssl-sys
links OpenSSL itself, so its settings live in the repo's `.cargo/config.toml`:
`OPENSSL_STATIC=1`, and on Linux the directories of Debian/Ubuntu's
libssl-dev (`<TARGET>_OPENSSL_LIB_DIR`, `_INCLUDE_DIR`), because through
pkg-config it would link a library under /usr dynamically. On macOS it
finds Homebrew's openssl@4 on its own. `native::check_openssl` then fails the
build unless openssl-sys used the headers of the OpenSSL php links (Homebrew's
openssl@4, pkg-config's libcrypto), their version matches the .pc file, and
both static archives are there. `OPENSSL_STATIC=0` links dynamically, for an
experiment. Ubuntu's libcrypto.a needs its `Libs.private`:
`-l:libjitterentropy.a` (a static archive, from libjitterentropy3-dev;
BSD-3-Clause or GPL-2.0, taken under BSD-3-Clause), and `-lz -lzstd`
dynamically (zlib1g-dev, libzstd-dev). The native-deps action installs them.

php reports `OPENSSL_VERSION_TEXT` from the headers it was compiled against.
A Homebrew bottle built before an openssl@4 update reports the older headers
(php 8.5.12_1: 4.0.3, while the library and the headers rhp builds against
are 4.1.0), so tests do not compare the header constants; the "linked" check
compares the library version.

**Never a silent fallback.** A missing library or static archive is a build
error that says what to install. rhp does not use -sys crates that quietly
build a bundled copy when the system library is missing (pcre2-sys did; a
binary that links two PCRE2 copies is wrong too, so rhp-difftest uses
`rhp_builtins::pcre::Pattern` rather than the `pcre2` crate).

`RHP_<LIB>_LINK=static|dynamic` (e.g. `RHP_PCRE2_LINK=dynamic`) overrides the
policy for one build, for experiments. An LGPL row (`Mode::Lgpl`, libgmp)
refuses `static`.

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

The helper drops the library itself from these (a second, dynamic `-l` for
it would override the static one), the ones with rows of their own (ICU's
libicuuc and libicudata) and the ones openssl-sys links (libcurl's libssl
and libcrypto), so each library is linked once.

**Checks.** `crates/rhp-cli/tests/ref_build.rs` requires the "linked" section
of `tools/capture-ref-build.php` to print the same under rhp and the host's
php 8.5. CI prints what its php links (the setup-php action's last step).

On CI, php is not always the distro's build. setup-php's php on Ubuntu 26.04
ships its own `libpcre2-8.so` (PCRE2 10.47, against which it was compiled)
next to the distro's 10.46, outside dpkg. It becomes the copy php loads once
`ldconfig` runs, so the setup-php action runs `ldconfig` straight away. The
native-deps action then compares the PCRE2 and libxml2 versions php reports
(`PCRE_VERSION`, `LIBXML_LOADED_VERSION`) with the distro's `-dev` packages.
ICU has no source build: when php's `INTL_ICU_VERSION` is not the distro's
libicu-dev, the job fails. For PCRE2 or libxml2 that differ,
`build-from-source.sh` builds php's release from the upstream tarball,
checked against `.github/actions/native-deps/sha256sums` (PCRE2: the
checksums Homebrew's formula pins; libxml2: GNOME's published ones), static
only, and points `PKG_CONFIG_PATH` at it. libxml2 is
configured like Debian/Ubuntu's build where that changes parsing (zlib on,
ICU off). A release with no recorded checksum fails the job; add its
checksum.
The `php` input names a php that is not the one on PATH (the nightly
sanitizer build uses php-src's bundled PCRE2).
`otool -L target/release/rhp` (or `ldd`) shows no libpcre2, libicu, libssl,
libcrypto or libcurl (and on Linux no libxml2); `nm` shows the `pcre2_*` and
`curl_*` symbols. rhp-builtins' `pcre2_is_linked_statically`,
`libxml2_is_linked_statically`, `icu_is_linked_statically`,
`openssl_is_linked_statically` and `libcurl_is_linked_statically` tests
check its own test binary, which is linked straight from the crate. The last
also requires `curl_version_info()` to report the version of the curl.h rhp
compiled against. ext/gmp's `links_libgmp_dynamically` checks with
`dladdr` that libgmp comes from the shared library, and
`runtime_library_matches_the_header` that its `gmp_version` is the gmp.h
rhp compiled against. libzip is dynamic too: `libzip_is_the_library_php_links`
checks with `dladdr` that `zip_open` loads from the directory the build
found it in, and that `zip_libzip_version()` is the release of zipconf.h.
The native-deps action fails when CI's php reports another
`ZipArchive::LIBZIP_VERSION` than libzip-dev's; there is no source build.

php links libcurl from Homebrew's keg-only curl on the mac, not the SDK's
older libcurl, whose error texts differ: the CURL row names the formula, so
the build never falls back to a bare `-lcurl`. rhp declares libcurl's API
itself (`ext/curl/sys.rs`) instead of using curl-sys, which links `-lcurl`
from the SDK on macOS and builds its bundled libcurl when pkg-config finds
none. ext/curl's constants are php's names and version gates
(`rhp-difftest gen curl-consts`, from curl.stub.php), evaluated against the
linked curl.h at build time (`ext/curl/consts.c`).

rhp declares the libzip API ext/zip calls itself (`ext/zip/sys.rs`; the
`axfive-libzip-sys` crate runs bindgen and CMake). `ext/zip/shim.c` holds
what php's build adds around zip.h: `ZipArchive`'s constants (names,
attributes and `#ifdef` guards from php_zip.stub.php, `rhp-difftest gen
zip-consts`, evaluated against the linked zip.h, so mac's 1.12.1 and CI's
1.11.4 each define what php defines with them), the `LIBZIP_ATLEAST(1,9,1)`
gate on entry seeking, and the layouts of `zip_stat_t` and `zip_error_t`,
which sys.rs mirrors and a test compares. libzip opens archive files
itself, so ext/zip hands it absolute paths (php's `expand_filepath`).

## Compiled from source: rhp-csrc

`crates/rhp-csrc` holds the C rhp compiles itself, with `cc`, into static
archives, except LGPL C (see [Licences](#licences)):

- **php-src's own C**, copied unmodified into `vendor/php-src` by
  `crates/rhp-csrc/tools/sync-php-src-c.sh <php-src-dir>` from a checkout at
  the commit `tests/phpt/php-src.ref` pins. Today: ext/standard's
  crypt_freesec, crypt_blowfish, crypt_sha256 and crypt_sha512, and timelib
  (ext/date/lib). All php-src C shares one stand-in for the Zend and PHP
  headers, `shim/zend_shim.h`, and builds with php's flags (`php_src()` in
  `build.rs`). timelib needs no Zend: it builds with its own flags from
  ext/date/config0.m4 (`-fwrapv`), and `glue/timelib_config.h` stands in for
  php's generated timelib_config.h (timelib's allocator on the shim's
  emalloc family, so it counts against memory_limit). `timezonedb.h` carries the tz database
  twice; the sync script keeps the "slim" branch parse_tz.c compiles, which
  gives a byte-identical object at a third of the size.
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

**LGPL C: rhp-mbfl-sys.** php-src's libmbfl (`ext/mbstring/libmbfl`, LGPL-2.1)
and the PHP-licensed code mbstring.c compiles next to it (php_unicode.c,
unicode_data.h, eaw_table.h, rare_cp_bitvec.h) live in
`crates/rhp-mbfl-sys/vendor/php-src`, written by the same
`sync-php-src-c.sh`, which copies every `.c` and `.h` under libmbfl so a
php bump brings new files along. The crate builds against rhp-csrc's shim
(`DEP_RHP_CSRC_SHIM`), whose symbols, `zend_empty_string` and the `emalloc`
family included, are defined once, in rhp-csrc's zend archive. It has its
own prefix header (`glue/prefix_mbfl.h`) and tests: every global symbol is
`rhp_`-prefixed (the failing test prints the lines to add), and no symbol
is defined in both crates. `glue/mbfl.c` wraps libmbfl's static inline
buffer helpers and checks with `_Static_assert` the layouts and constants
the Rust side mirrors. rhp-builtins depends on it only through its
`mbstring-libmbfl` feature (on by default); without it there is no
mbstring extension and no LGPL code in the binary. phpinfo reports
libmbfl's version from `MBFL_VERSION_*`, read through the glue, as php's
does.

**Memory.** The shim's `emalloc`, `erealloc` and `ecalloc` go through Rust's
global allocator, so C allocations count against `memory_limit` as php's
do. php 8.5's libmbfl grows its buffers with the fallible
`safe_ferealloc`: past the limit it fails, the conversion stops, and the
error waits for the VM's next check, as php delays it. The shim's version
does the same (`rhp_values::mem::reserve_realloc`). emalloc blocks are
allocated byte-aligned (the system allocator still returns them
malloc-aligned; a block that isn't is allocated again 16-aligned), so a
string the C returns becomes a Rust `Vec<u8>` in place
(`rhp_csrc::emalloc_into_vec`), and mbstring reads its string arguments
without copying them: a call holds its data once, as in php.

**timelib's tz database.** rhp always uses php's embedded database
(`timezone_version_get()` is its version, 2026.5 for php 8.5.12), as php.net
and Homebrew builds do. CI's php is Debian/Ubuntu's build, patched to read the
system zoneinfo: it reports `"Olson" Timezone Database Version => 0.system`
(and `Timezone Database => internal`; setup-php's library report, run
41756527132). Both are tz release 2026e (Ubuntu's tzdata 2026e), and both
are built the same way: Debian runs zic with `PACKRATDATA=backzone
PACKRATLIST=zone.tab`, and php's database also keeps the pre-1970 history
of zone.tab's zones (Europe/Amsterdam) while the other backzone zones are
links (America/Montreal). So zone data should agree, though nothing
compares it on CI yet. What reads the database itself differs
(`timezone_version_get()`, the identifier list and its backward-compatible
flags, locations, transitions), so the ratchet and the goldens leave out
output that depends on those; tools/rhp-difftest/tests/date.rs checks it
against a php whose database version is rhp's. A system-tzdata mode
(Debian's MIT-licensed patch with runtime selection) is plan item L7c, only
if the CI ratchet needs it. timelib's version and the database's are not in
the "linked" section for that reason.

## Licences

- Never GPL.
- Permissive licences (MIT, BSD, Apache-2.0, ISC, Zlib, curl, Unicode-3.0,
  CC0-1.0, public domain) may be linked or vendored; their notices go in
  `THIRD-PARTY-NOTICES`.
- MPL-2.0 data or code may be linked unmodified if its notice names where
  its source is (the Public Suffix List inside Homebrew's static libpsl).
- LGPL only as a dynamic system library (libgmp), or vendored in its own
  default-on feature crate: rhp ships its source, so users can relink. That
  crate gets an exact licence expression (libmbfl: `rhp-mbfl-sys`,
  `LGPL-2.1-only AND PHP-3.01 AND MIT`).
  Never compile or transliterate libbcmath's LGPL arithmetic.
- Code ported from php-src is under the PHP License 3.01 (Zend Engine
  License 2.00 for `Zend/`). Keep both notices and php's acknowledgement in
  `THIRD-PARTY-NOTICES`.
- Adding or vendoring a library means adding its entry to
  `THIRD-PARTY-NOTICES` in the same change.

## Threading

One VM per OS thread (see the web SAPI plan). For C libraries:

- **Process-wide init happens once**, under `std::sync::Once` (e.g.
  `xmlInitParser`, `curl_global_init`). PCRE2 and libmbfl need none:
  libmbfl's tables are `const` or never written, and the AVX2 function
  pointers in mbfilter_utf16.c, set at startup in php, are compiled out.
- **Everything else is per thread.** PCRE2's compile and match contexts, JIT
  stack, settings and pattern cache live in a `thread_local!` in
  `rhp-builtins/src/pcre/mod.rs`, as php keeps them in TLS globals. Library
  handles are `!Send` and stay with their VM.
- **ICU** keeps no state php changes: php never calls `uloc_setDefault`,
  `u_setDataDirectory` or `u_setMemoryFunctions`, and neither does rhp.
  Handles (break iterators, searches, collators) belong to one call or to
  the thread (intl's character break iterator, cloned per call, as php's
  `INTL_G(grapheme_iterator)`). Never call `u_cleanup`. ICU's default
  locale comes from the environment (`LC_ALL`/`LC_MESSAGES`/`LANG`) once
  per process, so the phpt harness and the intl live tests
  (`crates/rhp-cli/tests/intl.rs`) run php and rhp with `LC_ALL=C`.
- **Per-thread hooks are installed when each VM starts.** libxml2's generic
  error handler, structured error handler and file I/O hooks are per thread
  in both 2.9 and 2.15: `ext::libxml::reset_state` sets php's handlers on the
  VM's thread, as php's RINIT does.
- **Global hooks** a library offers only process-wide (libxml2's entity
  loader, libxslt's error function) are set once, to a dispatcher that reads
  thread-local VM state: libxml2's entity loader runs php's loader on a
  thread whose VM installed the glue while a builtin is running, and
  libxml2's own otherwise. DOM's loads, validation and XInclude reach both
  through `ffi::with_host`.
- **libxml2 trees** belong to the objects that wrap them, counted as php
  counts them (`ext/libxml/node.rs`, a port of libxml.c's node and document
  references): a document goes with the last object that references it, a
  detached subtree with the last object that wraps a node of it. DOM,
  SimpleXML and XMLReader's `expand()` share these trees and counts, so
  `simplexml_import_dom` and `dom_import_simplexml` hand out the same
  nodes. Each object keeps its references in its payload; there are no
  per-thread document tables. Trees never leave their VM's thread.
- **No process-global side effects from a VM:** no `setlocale` (use
  `newlocale`/`uselocale`), no relative paths handed to a library that opens
  files itself (absolutize against the VM's cwd), no `putenv("TZ=…")`.
- **C→Rust callbacks** (`ext/libxml/ffi.rs`):
  - a builtin runs the C call inside `ffi::with_host(host, fname, …)`, which
    pushes its `Host` on a thread-local stack;
  - every `extern "C"` callback runs its body in `ffi::guard`, which catches
    a panic (it must not unwind into C), stops the C operation and resumes
    the panic after the C call returns. OpenSSL calls its callbacks
    (passphrase, name lists) without a host: ext/openssl's own `guard`
    keeps the panic and `with_callbacks` resumes it once the C call
    returns. A callback whose body only calls C (OpenSSL's free shims,
    `d2i`) needs no guard;
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
  - libzip's progress and cancel callbacks (ext/zip) run while an archive
    closes, inside `with_host`. Their state is the archive
    (`php_zip_archive`), shared by the `ZipArchive`'s payload and the entry
    streams opened from it; libzip frees the callbacks with the archive.
    The last owner closes it: a stream's close, `close()`/`open()`, or the
    class's `free_obj_host` after any `__destruct` when the object goes
    (an archive the destructor pass never reaches closes in the payload's
    `Drop`, where the callbacks see no host and do nothing). An exception
    a callback throws while a stream closes comes out of `fclose()`, or of
    the next builtin call for a stream freed by its last reference. A
    warning raised while a callback's exception is pending skips a user
    error handler, as php's does;
  - a library handle belongs in its object's payload (ext/openssl's and
    ext/curl's still sit in per-thread tables keyed by object id); one whose
    free can call back into PHP (an XMLWriter flushing to a stream) is freed
    by the class's `dtor_obj` handler, which the engine runs with a host
    before `__destruct`, as php does. The payload's `Drop` (php's `free_obj`) has
    no host; it frees only what `dtor_obj` would have skipped. See
    [object-handlers.md](./object-handlers.md).
- **OpenSSL's library context.** php 8.5 runs ext/openssl in its own
  `OSSL_LIB_CTX` (`openssl.libctx=custom`, the default): one per thread
  (module globals under ZTS), loaded with `CONF_modules_load_file_ex` from
  the default openssl.cnf, and passed to the `*_ex` calls and fetches. rhp
  does the same with raw FFI: a thread-local context created on first use
  and freed with the thread, and `openssl.libctx=default` selects OpenSSL's
  global one. The two differ only when openssl.cnf activates providers; the
  calls php makes without a context (`EVP_PKEY_CTX_new`, `X509_REQ_new`,
  phar's signature check) stay without one. Errors stay per thread: OpenSSL
  keeps its queue in thread-local storage, and php's 16-entry ring
  (`openssl_error_string`) is a thread-local too.
- **TLS streams** (`ext/openssl/xp_ssl.rs`, a port of xp_ssl.c) run the
  session on the socket's own descriptor with `SSL_set_fd`, as php does, so
  the errno-based errors (`SSL: Connection reset by peer`) and the
  EAGAIN/poll retry loop are php's. Stream reads and writes happen inside the
  VM without a host: the TLS layer queues its warnings
  (`fdstream::push_diag`) and the code that ran the operation reports them
  under its builtin; peer certificates for `capture_peer_cert` come back to
  a caller with a host, which wraps them in `OpenSSLCertificate` objects.
  What php calls back into PHP from inside a read (the context notifier's
  progress and completed events, `reneg_limit_callback`) is recorded by the
  read (`fdstream::NotifyEvent`) and delivered right after it.
  The verify, passphrase, SNI and ALPN callbacks read their data from a
  per-session allocation behind the SSL's ex_data and run under
  ext/openssl's `guard`. `SSL_CTX_new` takes no library context, as in php.
  CA certificates come from the `cafile`/`capath` context options, then
  `openssl.cafile`/`openssl.capath`, then OpenSSL's default paths
  (`SSL_CTX_set_default_verify_paths`): no bundled root store. The
  `OPENSSL_NO_*` defines xp_ssl.c checks (SSLv3, TLS 1.1/1.2/1.3, TLS
  extensions) are read from the headers' configuration.h by build.rs.
- **The http:// wrapper** (`http.rs`, a port of http_fopen_wrapper.c) opens
  its connection through the socket transports, so https:// is the TLS
  layer above. URLs are parsed by `url.rs`, a port of `php_url_parse_ex2`.
- **Guard aborts.** A C library that can abort the process needs size
  checks first: with one VM per thread, an abort kills every request.
  libgmp aborts when a number would need more than `INT_MAX` limbs (php
  then dies with SIGFPE or SIGABRT, e.g. `gmp_pow(2, PHP_INT_MAX)`). ext/gmp
  sizes each result that can grow (`+ - * ** <<`, `gmp_pow`, `gmp_mul`,
  `gmp_lcm`, `gmp_com`, `gmp_nextprime`…) as libgmp allocates it, and
  factorials and binomials from an estimate with a 2x margin, and throws
  "is too large" (a `ValueError`) where libgmp would abort. Running out of
  memory below that limit still aborts, in php and in rhp.

## Adding a library

1. Add a row to `LIBS` in `crates/rhp-builtins/build/native.rs`: pkg-config
   name, Homebrew formula (or `macos_sdk` for a library php takes from the
   macOS SDK), link name, the header that defines its version macros, the
   macros to export, and the link mode per OS. The helper finds it with
   pkg-config, or on macOS without pkg-config through
   `/opt/homebrew/opt/<formula>` (`/usr/local/opt/<formula>`) or the SDK
   (`xcrun --show-sdk-path`); checks the header's version against the .pc
   file (a header Debian keeps in the multiarch directory, such as gmp.h,
   is found there); links it; exports each macro as `env!("RHP_<MACRO>")`;
   and returns the include directory, for a C shim built against the same
   headers.
2. Declare the functions rhp calls in a hand-written `extern "C"` block
   (no bindgen). Put struct access and macros behind a small C shim with
   `_Static_assert` layout checks if needed (libxml2: `ext/libxml/sys.rs`
   and `shim.c`; most of its structs are opaque to Rust because their
   layouts differ between releases; the tree structs that have not changed
   since 2.9 are mirrored, checked against the headers by a test).
3. Add the `-dev` package to `.github/actions/native-deps`. If CI's php can
   link a release the distro has no `-dev` package for, add the library to
   its version comparison and to `build-from-source.sh`.
4. Read every version constant the way php's source reads it, and add the
   constants to the "linked" section of `tools/capture-ref-build.php`.
5. Add the licence to `THIRD-PARTY-NOTICES`.

## Adding C to rhp-csrc

1. php-src C: add the files to `FILES` in
   `crates/rhp-csrc/tools/sync-php-src-c.sh` and run it on a php-src
   checkout at the pinned commit. LGPL C goes in a crate of its own that
   builds against this shim, like rhp-mbfl-sys (`MBFL_FILES`). A frozen library: write a
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
`rhp-difftest gen <name> --check` (`tools/rhp-difftest/src/gen/`):

| Generator | Source | Output | Checked |
|---|---|---|---|
| `html-tables` | php-src `ext/standard/html_tables/` (+ `html_tables.h` for HTML5 ties) | `rhp-builtins/src/html_tables.rs` | CI job `generated` (php-src checkout) |
| `intl-consts` | php-src `ext/intl` stubs: global constants' names, types, `@cvalue` and `#if` guards (values come from the linked ICU's headers when `ext/intl/shim.c` compiles) | `rhp-builtins/src/ext/intl/consts.h` | CI job `generated` (php-src checkout) |
| `parser` | the release tarball (`tools/php-dist.sha256`): Bison's `zend_language_parser.{c,h}`, `zend_ini_parser.{c,h}`; the numbers of the grammar rules whose actions rhp runs | `ext/tokenizer/ids.rs`, `zend/*_parser_tables.rs` | CI job `generated` (tarball cached on the pin) |
| `dumps` | the reference php: `tools/capture-ref-build.php tables`, plus constants gated on a newer library (`CONST_EXTRAS`) | `ini_table.rs`, `const_modules.rs` | `cargo test` on the reference host |
| `curl-consts` | php-src `ext/curl/curl.stub.php` (names, `@cvalue`, `LIBCURL_VERSION_NUM` gates, `#[\Deprecated]`) and `curl_private.h` | `ext/curl/consts.rs`, `ext/curl/consts.c` (values from the linked curl.h) | CI job `generated` (php-src checkout) |
| `zip-consts` | php-src `ext/zip/php_zip.stub.php`: `ZipArchive`'s constants, `@cvalue`, `#ifdef` guards, attributes | `ext/zip/consts.h` (values from the linked zip.h, through `ext/zip/shim.c`) | CI job `generated` (php-src checkout) |
