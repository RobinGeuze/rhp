//! The php build rhp mirrors: the one place that names its release, its build
//! identity and the versions of the libraries it links.
//!
//! The reference is the Homebrew `php` bottle (macOS, arm64). Every value here
//! is printed by `tools/capture-ref-build.php`; `crates/rhp-cli/tests/ref_build.rs`
//! runs that script under both php and rhp and requires identical output.
//!
//! Bumping to a new patch release (e.g. 8.5.13):
//! 1. upgrade the reference php (`brew upgrade php`), and check out the
//!    matching php-src tag; put its commit in `tests/phpt/php-src.ref`;
//! 2. edit `release!()`, `brew_rev!()` and [`BUILD_DATE`] below, then run
//!    `php -n tools/capture-ref-build.php` and copy any changed library value
//!    into this file;
//! 3. re-bless the goldens that print the version (`tests/golden/sys_misc.out`,
//!    `phpinfo_text.out`: `php -n NAME.php > NAME.out`) and
//!    `crates/rhp-sapi/tests/isolation.rs`, then regenerate the phpt ratchet.

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
// Elsewhere: the Debian/Ubuntu package layout.
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

/// libzip's version (`ZipArchive::LIBZIP_VERSION`), as a literal: the zip
/// prelude splices it into PHP source.
macro_rules! libzip {
    () => {
        "1.12.1"
    };
}
pub(crate) use libzip;

// --- php / Zend identity ---------------------------------------------------

pub const PHP_MAJOR_VERSION: i64 = 8;
pub const PHP_MINOR_VERSION: i64 = 5;
pub const PHP_RELEASE_VERSION: i64 = 12;
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
/// `PHP_BUILD_DATE` (`__DATE__ " " __TIME__` of the reference build).
pub const BUILD_DATE: &str = "Oct 19 2026 17:02:46";

// The release macro and the numeric constant must agree.
const _: () = {
    let r = release!().as_bytes();
    let mut n = 0i64;
    let mut i = 0;
    while i < r.len() {
        n = n * 10 + (r[i] - b'0') as i64;
        i += 1;
    }
    assert!(n == PHP_RELEASE_VERSION);
};

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

// --- extension versions (`phpversion($ext)`) ---------------------------------

/// `phpversion('zip')` (`PHP_ZIP_VERSION`).
pub const EXT_ZIP: &str = "1.22.8";
/// `phpversion('dom')` (the DOM API date).
pub const EXT_DOM: &str = "20031129";

// --- linked libraries --------------------------------------------------------

/// `PCRE_VERSION`, `PCRE_VERSION_MAJOR`, `PCRE_VERSION_MINOR`.
pub const PCRE_VERSION: &str = "10.49 2026-09-28";
pub const PCRE_VERSION_MAJOR: i64 = 10;
pub const PCRE_VERSION_MINOR: i64 = 49;
/// `OPENSSL_VERSION_TEXT` / `OPENSSL_VERSION_NUMBER` (the headers php was
/// compiled against; curl reports its own runtime OpenSSL below).
pub const OPENSSL_VERSION_TEXT: &str = "OpenSSL 4.0.3 29 Sep 2026";
pub const OPENSSL_VERSION_NUMBER: i64 = 0x4000_0030;
/// `INTL_ICU_VERSION`, `INTL_ICU_DATA_VERSION`.
pub const ICU_VERSION: &str = "79.1";
pub const ICU_DATA_VERSION: &str = "79.1";
/// `ZipArchive::LIBZIP_VERSION`.
pub const LIBZIP_VERSION: &str = libzip!();
/// `LIBXML_VERSION`, `LIBXML_DOTTED_VERSION`.
pub const LIBXML_VERSION: i64 = 20913;
pub const LIBXML_DOTTED_VERSION: &str = "2.9.13";
/// `ICONV_VERSION`.
pub const ICONV_VERSION: &str = "1.11";
/// `MB_ONIGURUMA_VERSION`.
pub const ONIGURUMA_VERSION: &str = "6.9.10";
/// `GMP_VERSION`.
pub const GMP_VERSION: &str = "6.3.0";

/// `curl_version()`: the libcurl the reference php links at run time.
pub mod curl {
    pub const VERSION: &str = "8.23.0";
    pub const VERSION_NUMBER: i64 = 0x08_17_00;
    pub const AGE: i64 = 11;
    /// The `CURL_VERSION_*` feature bits (no TLS-SRP: OpenSSL 4 dropped SRP).
    pub const FEATURES: i64 = 1472137101;
    pub const HOST: &str = "aarch64-apple-darwin27.0.0";
    pub const SSL_VERSION: &str = "OpenSSL/4.1.0";
    pub const LIBZ_VERSION: &str = "1.2.12";
    pub const LIBSSH_VERSION: &str = "libssh2/1.11.1";
    pub const BROTLI_VERSION: &str = "brotli/1.2.1";
    pub const BROTLI_VER_NUM: i64 = 16785409;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_values() {
        assert_eq!(PHP_VERSION_ID, 80512);
        assert_eq!(
            PHP_VERSION,
            format!(
                "{PHP_MAJOR_VERSION}.{PHP_MINOR_VERSION}.{PHP_RELEASE_VERSION}{PHP_EXTRA_VERSION}"
            )
        );
        assert_eq!(ZEND_VERSION, format!("4.5.{PHP_RELEASE_VERSION}"));
        assert_eq!(
            PCRE_VERSION.split(' ').next().unwrap(),
            format!("{PCRE_VERSION_MAJOR}.{PCRE_VERSION_MINOR}")
        );
        assert!(EXTENSION_DIR.ends_with(PHP_API));
        assert_eq!(
            LIBXML_DOTTED_VERSION,
            format!(
                "{}.{}.{}",
                LIBXML_VERSION / 10000,
                LIBXML_VERSION / 100 % 100,
                LIBXML_VERSION % 100
            )
        );
    }
}
