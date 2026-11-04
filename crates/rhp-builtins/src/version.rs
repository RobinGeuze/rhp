//! The php build rhp mirrors: the one place that names its release, its build
//! identity and the versions of the libraries it links.
//!
//! The reference is the Homebrew `php` bottle (macOS, arm64). Every value here
//! is printed by `tools/capture-ref-build.php`; `crates/rhp-cli/tests/ref_build.rs`
//! runs that script under both php and rhp and requires identical output.
//!
//! Libraries rhp links report their own versions (PCRE2: `crate::pcre`,
//! libcurl: `curl_version()`); the ones still listed here are for code that
//! does not link its library yet.
//!
//! Built for another platform (Linux), rhp keeps the reference's release,
//! [`BUILD_DATE`] and the library versions below: they describe behaviour rhp
//! implements the same way everywhere. Only the
//! values that describe the platform follow the Ubuntu php CI runs against
//! (setup-php's build): the install paths, [`BUILD_PROVIDER`],
//! [`BUILD_SYSTEM`] and [`ZEND_VM_KIND`]. There
//! ref_build compares the release section only.
//!
//! Bumping to a new patch release (e.g. 8.5.13):
//! 1. upgrade the reference php (`brew upgrade php`), and check out the
//!    matching php-src tag; put its commit in `tests/phpt/php-src.ref`;
//! 2. edit `release!()`, `brew_rev!()` and [`BUILD_DATE`] below (every other
//!    version number is derived from them), then run
//!    `php -n tools/capture-ref-build.php` and copy any changed library or
//!    build value (e.g. [`BUILD_SYSTEM`]) into this file;
//!    `cargo test -p rhp-cli --test ref_build` fails on the reference host
//!    until rhp and php agree;
//! 3. pin the release tarball's SHA-256 (php.net's releases JSON) in
//!    `tools/php-dist.sha256` and run `rhp-difftest gen parser` on it;
//! 4. regenerate the phpt ratchet. No golden or test spells out the release.

/// `PHP_RELEASE_VERSION`, as a literal so it can be spliced with `concat!`.
macro_rules! release {
    () => {
        "12"
    };
}

/// The Homebrew formula revision suffix of the reference bottle (`"_1"` for
/// `8.5.12_1`, `""` when the formula has none). It shows up in the Cellar
/// paths below, which are therefore not simply `PHP_VERSION`.
#[cfg(target_os = "macos")]
macro_rules! brew_rev {
    () => {
        "_1"
    };
}

/// The installation prefix the reference php was configured with.
#[cfg(target_os = "macos")]
macro_rules! prefix {
    () => {
        concat!("/opt/homebrew/Cellar/php/8.5.", release!(), brew_rev!())
    };
}
// Elsewhere: the Ubuntu (Debian) package layout.
#[cfg(not(target_os = "macos"))]
macro_rules! prefix {
    () => {
        "/usr"
    };
}

/// `PHP_API_VERSION` (= `ZEND_MODULE_API_NO`), as a literal.
macro_rules! php_api {
    () => {
        "20250925"
    };
}

// --- php / Zend identity ---------------------------------------------------

pub const PHP_MAJOR_VERSION: i64 = 8;
pub const PHP_MINOR_VERSION: i64 = 5;
/// Parsed from `release!()`, so the release number is written once.
pub const PHP_RELEASE_VERSION: i64 = parse_int(release!());
pub const PHP_EXTRA_VERSION: &str = "";
/// `PHP_VERSION` (also `phpversion()`, X-Powered-By, phpinfo).
pub const PHP_VERSION: &str = concat!("8.5.", release!());
pub const PHP_VERSION_ID: i64 =
    PHP_MAJOR_VERSION * 10000 + PHP_MINOR_VERSION * 100 + PHP_RELEASE_VERSION;
/// `zend_version()`.
pub const ZEND_VERSION: &str = concat!("4.5.", release!());
/// `PHP_API_VERSION` / `ZEND_MODULE_API_NO` (phpinfo "PHP API", "PHP Extension").
pub const PHP_API: &str = php_api!();
/// `ZEND_EXTENSION_API_NO` (phpinfo "Zend Extension").
pub const ZEND_EXTENSION_API: &str = concat!("4", php_api!());
/// `ZEND_VM_KIND`: the executor the reference was built with.
#[cfg(target_os = "macos")]
pub const ZEND_VM_KIND: &str = "ZEND_VM_KIND_TAILCALL";
#[cfg(not(target_os = "macos"))]
pub const ZEND_VM_KIND: &str = "ZEND_VM_KIND_HYBRID";
/// `PHP_BUILD_DATE` (`__DATE__ " " __TIME__` of the reference build). The
/// same on every platform: it names the build rhp mirrors, not a Linux
/// package's (whose rebuilds would each change it).
pub const BUILD_DATE: &str = "Oct 19 2026 17:02:46";
/// phpinfo's "Build System": `uname -a` of the host the reference bottle
/// was built on. Homebrew builds a bottle per macOS version and CPU, so its
/// Darwin major version and architecture name the bottle; ref_build compares
/// the build section only against a Homebrew php that agrees on both.
#[cfg(target_os = "macos")]
pub const BUILD_SYSTEM: &str =
    "Darwin Golden-Gate-arm64.local 27.0.0 Darwin Kernel Version 27.0.0: \
    Tue Aug 11 21:00:07 PDT 2026; root:xnu-13432.1.9~1/RELEASE_ARM64_VMAPPLE arm64";
/// What setup-php's Ubuntu php reports: just the kernel name.
#[cfg(not(target_os = "macos"))]
pub const BUILD_SYSTEM: &str = "Linux";

/// A decimal literal as an integer, at compile time.
pub(crate) const fn parse_int(s: &str) -> i64 {
    let b = s.as_bytes();
    assert!(!b.is_empty());
    let mut n = 0i64;
    let mut i = 0;
    while i < b.len() {
        assert!(b[i].is_ascii_digit());
        n = n * 10 + (b[i] - b'0') as i64;
        i += 1;
    }
    n
}

/// The `-v` / `--version` banner of the reference build for the SAPI named
/// `sapi` (`"cli"`, `"cgi-fcgi"`): `php_cli.c` / `cgi_main.c`'s
/// `PHP %s (%s) (built: %s) (NTS)` block plus the loaded Zend extensions.
pub fn banner(sapi: &str) -> String {
    format!(
        "PHP {PHP_VERSION} ({sapi}) (built: {BUILD_DATE}) (NTS)\n\
         Copyright (c) The PHP Group\n\
         Built by {BUILD_PROVIDER}\n\
         Zend Engine v{ZEND_VERSION}, Copyright (c) Zend Technologies\n    \
         with Zend OPcache v{PHP_VERSION}, Copyright (c), by Zend Technologies\n"
    )
}

// --- compile-time paths ------------------------------------------------------

/// `PHP_PREFIX`.
pub const PREFIX: &str = prefix!();
/// `PHP_BINDIR`.
pub const BINDIR: &str = concat!(prefix!(), "/bin");
/// `DEFAULT_INCLUDE_PATH`, the default `include_path`.
#[cfg(target_os = "macos")]
pub const INCLUDE_PATH: &str = concat!(".:", prefix!(), "/share/php/pear");
#[cfg(not(target_os = "macos"))]
pub const INCLUDE_PATH: &str = ".:/usr/share/php";
/// `PHP_EXTENSION_DIR`, the default `extension_dir`.
#[cfg(target_os = "macos")]
pub const EXTENSION_DIR: &str = concat!(prefix!(), "/lib/php/", php_api!());
#[cfg(not(target_os = "macos"))]
pub const EXTENSION_DIR: &str = concat!("/usr/lib/php/", php_api!());

/// `PHP_SBINDIR`.
pub const SBINDIR: &str = concat!(prefix!(), "/sbin");
/// `PHP_MANDIR`.
pub const MANDIR: &str = concat!(prefix!(), "/share/man");
/// `PHP_LIBDIR`.
pub const LIBDIR: &str = concat!(prefix!(), "/lib/php");
/// `PHP_DATADIR`.
#[cfg(target_os = "macos")]
pub const DATADIR: &str = concat!(prefix!(), "/share/php");
#[cfg(not(target_os = "macos"))]
pub const DATADIR: &str = "/usr/share/php/8.5";
/// `PHP_SHLIB_SUFFIX`.
pub const SHLIB_SUFFIX: &str = "so";
/// `PEAR_EXTENSION_DIR` (= `PHP_EXTENSION_DIR`).
pub const PEAR_EXTENSION_DIR: &str = EXTENSION_DIR;

/// `PHP_BUILD_PROVIDER` (also phpinfo "Build Provider", `php -v` "Built by").
#[cfg(target_os = "macos")]
pub const BUILD_PROVIDER: &str = "Homebrew";
/// `PEAR_INSTALL_DIR`.
#[cfg(target_os = "macos")]
pub const PEAR_INSTALL_DIR: &str = concat!(prefix!(), "/share/php/pear");
/// `PHP_SYSCONFDIR`.
#[cfg(target_os = "macos")]
pub const SYSCONFDIR: &str = "/opt/homebrew/etc/php/8.5";
/// `PHP_LOCALSTATEDIR`.
#[cfg(target_os = "macos")]
pub const LOCALSTATEDIR: &str = "/opt/homebrew/var";
/// `PHP_CONFIG_FILE_PATH` (also phpinfo "Configuration File (php.ini) Path").
#[cfg(target_os = "macos")]
pub const CONFIG_FILE_PATH: &str = "/opt/homebrew/etc/php/8.5";
/// `PHP_CONFIG_FILE_SCAN_DIR`.
#[cfg(target_os = "macos")]
pub const CONFIG_FILE_SCAN_DIR: &str = "/opt/homebrew/etc/php/8.5/conf.d";

/// What setup-php's Ubuntu build reports.
#[cfg(not(target_os = "macos"))]
pub const BUILD_PROVIDER: &str = "Ubuntu";
#[cfg(not(target_os = "macos"))]
pub const PEAR_INSTALL_DIR: &str = "/usr/share/php";
#[cfg(not(target_os = "macos"))]
pub const SYSCONFDIR: &str = "/etc";
#[cfg(not(target_os = "macos"))]
pub const LOCALSTATEDIR: &str = "/var";
#[cfg(not(target_os = "macos"))]
pub const CONFIG_FILE_PATH: &str = "/etc/php/8.5/cli";
#[cfg(not(target_os = "macos"))]
pub const CONFIG_FILE_SCAN_DIR: &str = "/etc/php/8.5/cli/conf.d";

// --- extension versions (`phpversion($ext)`) ---------------------------------

/// `phpversion('zip')` (`PHP_ZIP_VERSION`).
pub const EXT_ZIP: &str = "1.22.8";
/// `phpversion('dom')` (the DOM API date).
pub const EXT_DOM: &str = "20031129";

// --- linked libraries --------------------------------------------------------

// PCRE_VERSION and friends come from the linked PCRE2 (crate::pcre), the
// LIBXML_* versions from the linked libxml2 (crate::ext::libxml), the
// INTL_ICU_* versions from the linked ICU (crate::ext::intl), the OPENSSL_*
// ones from the linked OpenSSL (crate::ext::openssl), curl_version() from the
// linked libcurl (crate::ext::curl), ICONV_* from the system iconv
// (crate::ext::iconv), GMP_VERSION from the linked libgmp (crate::ext::gmp),
// ZipArchive::LIBZIP_VERSION from the linked libzip (crate::ext::zip).
/// `MB_ONIGURUMA_VERSION`.
pub const ONIGURUMA_VERSION: &str = "6.9.10";

/// `ZLIB_VERSION`, `ZLIB_VERNUM` (the zlib headers php was compiled against).
pub const ZLIB_VERSION: &str = "1.2.12";
pub const ZLIB_VERNUM: i64 = 0x12c0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_values() {
        assert_eq!(
            PHP_VERSION_ID,
            80500 + PHP_RELEASE_VERSION,
            "PHP_VERSION_ID follows release!()"
        );
        assert_eq!(parse_int("12"), 12);
        assert_eq!(parse_int("0"), 0);
        assert_eq!(
            PHP_VERSION,
            format!(
                "{PHP_MAJOR_VERSION}.{PHP_MINOR_VERSION}.{PHP_RELEASE_VERSION}{PHP_EXTRA_VERSION}"
            )
        );
        assert_eq!(ZEND_VERSION, format!("4.5.{PHP_RELEASE_VERSION}"));
        assert!(EXTENSION_DIR.ends_with(PHP_API));
        // ZLIB_VERNUM packs the dotted version one hex digit per component.
        let z: Vec<i64> = ZLIB_VERSION
            .split('.')
            .map(|p| p.parse().unwrap())
            .collect();
        let at = |i: usize| z.get(i).copied().unwrap_or(0) & 0xF;
        assert_eq!(
            ZLIB_VERNUM,
            (at(0) << 12) | (at(1) << 8) | (at(2) << 4) | at(3)
        );
    }
}
