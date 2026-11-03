# C libraries: what rhp links and how

Status: policy (Phase L). It replaces the "pure-Rust by default" stance of
[`extensions.md`](./extensions.md) §4 and §6.

## The rule

- **Where php calls an external C library, rhp calls the same library.** The
  same library at the same version gives the same bytes; no reimplementation
  matched php on both reference machines (the mac and CI link different
  versions of libxml2, ICU, PCRE2 and curl).
- **Where php runs its own C** with no Zend dependency (libmbfl, timelib,
  crypt), rhp compiles that C from the pinned php-src tag.
- **Where php-src owns the data** (stubs, entity tables, Bison tables), rhp
  generates it from php-src.
- A pure-Rust crate replaces library behaviour only with evidence: it is
  spec-identical (hashes, CRC-32), or it passed a prototype comparison
  against php.
- **Version constants come from the library rhp linked:** a header macro
  where php reads a header macro, a runtime getter where php calls one.
  `PCRE_VERSION` comes from `pcre2_config()`, `PCRE_VERSION_MAJOR`/`MINOR`
  from pcre2.h. `version.rs` keeps hard-coded values only for libraries rhp
  does not link yet; each goes when its library is linked.

## Linking: static by default

| Kind | Linked | Examples |
|---|---|---|
| Code rhp compiles: php-src C, frozen vendored libraries | static | libmbfl, timelib, argon2, oniguruma |
| A third-party library whose static archive is the build php links | static | PCRE2; later gmp, icu4c@79, openssl@4, curl |
| The listed exceptions below | dynamic | |

A static archive is "the build php links" when it comes from the same package
as the shared library php loads: Homebrew's `.a` next to the `.dylib` on the
mac, the `-dev` package's `.a` on Debian/Ubuntu.

**Dynamic only for these.** Each one is a reviewed decision; add to the list
rather than linking dynamically quietly.

- OS libraries that php also takes from the OS and that ship no static
  archive. macOS `/usr/lib`: libxml2, libxslt, libiconv, libz, libbz2,
  libedit, libffi. Linux: glibc's iconv.
- Libraries with no static archive of php's version: Homebrew's libzip.

Today rhp-builtins links:

| Library | macOS | Debian/Ubuntu |
|---|---|---|
| PCRE2 (libpcre2-8) | static, Homebrew pcre2 | static, libpcre2-dev (CI: php's release, see below) |
| zlib (via libz-sys; also crc32, adler32) | dynamic, /usr/lib/libz | dynamic, zlib1g |
| bzip2 (bzip2 `=0.5.2`, feature `static`) | static, bzip2-sys's bundled C libbz2 1.0.8 | static, bzip2-sys's bundled C libbz2 1.0.8 |

zlib and bzip2 predate this policy and still go through their -sys crates.
Known gap: libz-sys builds its bundled zlib when it finds no system zlib, a
silent fallback.

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
to the `.a`. A static link's `Libs.private` dependencies are linked
dynamically from the default paths; one that is a Homebrew or `-dev`
library needs its own row.

**Checks.** `crates/rhp-cli/tests/ref_build.rs` requires the "linked" section
of `tools/capture-ref-build.php` to print the same under rhp and the host's
php 8.5. CI prints what its php links (the setup-php action's last step).

On CI, php is not always the distro's build. setup-php's php on Ubuntu 26.04
ships its own `libpcre2-8.so` (PCRE2 10.47, against which it was compiled)
next to the distro's 10.46, outside dpkg. It becomes the copy php loads once
`ldconfig` runs, so the setup-php action runs `ldconfig` straight away. The
native-deps action then compares the PCRE2 php reports with the distro's
`libpcre2-dev`. When they differ, it builds php's release from the upstream
tarball, checked against `.github/actions/native-deps/sha256sums` (the
checksums Homebrew's formula pins), static only, and points `PKG_CONFIG_PATH`
at it. A release with no recorded checksum fails the job; add its checksum.
The `php` input names a php that is not the one on PATH (the nightly
sanitizer build uses php-src's bundled PCRE2).
`otool -L target/release/rhp` (or `ldd`) shows no libpcre2; `nm` shows the
`pcre2_*` symbols. rhp-builtins' `pcre2_is_linked_statically` test checks
its own test binary, which is linked straight from the crate.

## Licences

- Never GPL.
- Permissive licences (MIT, BSD, Apache-2.0, ISC, Zlib, curl, Unicode-3.0,
  public domain) may be linked or vendored; their notices go in
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
- **Global hooks** a library offers only process-wide (libxml2's entity
  loader, libxslt's error function) are set once, to a dispatcher that reads
  thread-local VM state.
- **No process-global side effects from a VM:** no `setlocale` (use
  `newlocale`/`uselocale`), no relative paths handed to a library that opens
  files itself (absolutize against the VM's cwd), no `putenv("TZ=…")`.
- **C→Rust callbacks** are `extern "C"`, catch panics, find their VM through
  a thread-local, and stop the C operation on `exit` or an exception, then
  rethrow once it returns. php longjmps out of the library; rhp cannot.
- **Guard aborts.** A C library that can abort the process (libgmp on huge
  sizes) needs size checks first: with one VM per thread, an abort kills
  every request.

## Adding a library

1. Add a row to `LIBS` in `crates/rhp-builtins/build/native.rs`: pkg-config
   name, Homebrew formula, link name, the header that defines its version
   macros, the macros to export, and the link mode per OS. The helper finds
   it with pkg-config, or on macOS without pkg-config through
   `/opt/homebrew/opt/<formula>` (`/usr/local/opt/<formula>`); checks the
   header's version against the .pc file; links it; and exports each macro
   as `env!("RHP_<MACRO>")`.
2. Declare the functions rhp calls in a hand-written `extern "C"` block
   (no bindgen). Put struct access and macros behind a small C shim with
   `_Static_assert` layout checks if needed.
3. Add the `-dev` package to `.github/actions/native-deps`. If CI's php can
   link a release the distro has no `-dev` package for, build it there the
   way PCRE2 is built.
4. Read every version constant the way php's source reads it, and add the
   constants to the "linked" section of `tools/capture-ref-build.php`.
5. Add the licence to `THIRD-PARTY-NOTICES`.
