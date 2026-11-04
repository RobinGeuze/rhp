//! `phpinfo()` / `phpcredits()` in php's CLI text layout (ext/standard/info.c
//! with `sapi_module.phpinfo_as_text`), plus `php_uname()` backed by uname(2).
//!
//! The INFO_GENERAL block follows php's row order and formatting exactly; the
//! values describe this engine (rhp) where php would describe its build.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::{version, Host, Thrown};

pub const INFO_GENERAL: i64 = 1;
pub const INFO_CREDITS: i64 = 2;
pub const INFO_CONFIGURATION: i64 = 4;
pub const INFO_MODULES: i64 = 8;
pub const INFO_ENVIRONMENT: i64 = 16;
pub const INFO_VARIABLES: i64 = 32;
pub const INFO_LICENSE: i64 = 64;
pub const INFO_ALL: i64 = 0xFFFF_FFFF;

/// Constants this module defines (`INFO_*`, `CREDITS_*`).
pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("INFO_GENERAL", Value::Int(INFO_GENERAL)),
        ("INFO_CREDITS", Value::Int(INFO_CREDITS)),
        ("INFO_CONFIGURATION", Value::Int(INFO_CONFIGURATION)),
        ("INFO_MODULES", Value::Int(INFO_MODULES)),
        ("INFO_ENVIRONMENT", Value::Int(INFO_ENVIRONMENT)),
        ("INFO_VARIABLES", Value::Int(INFO_VARIABLES)),
        ("INFO_LICENSE", Value::Int(INFO_LICENSE)),
        ("INFO_ALL", Value::Int(INFO_ALL)),
        ("CREDITS_GROUP", Value::Int(1)),
        ("CREDITS_GENERAL", Value::Int(2)),
        ("CREDITS_SAPI", Value::Int(4)),
        ("CREDITS_MODULES", Value::Int(8)),
        ("CREDITS_DOCS", Value::Int(16)),
        ("CREDITS_FULLPAGE", Value::Int(32)),
        ("CREDITS_QA", Value::Int(64)),
        ("CREDITS_ALL", Value::Int(INFO_ALL)),
    ]
}

/// uname(2) fields: (sysname, nodename, release, version, machine).
fn uname_fields() -> [String; 5] {
    // SAFETY: utsname is plain old data; uname fills it with NUL-terminated strings.
    unsafe {
        let mut u: libc::utsname = std::mem::zeroed();
        if libc::uname(&mut u) != 0 {
            return Default::default();
        }
        let f = |a: &[libc::c_char]| {
            std::ffi::CStr::from_ptr(a.as_ptr())
                .to_string_lossy()
                .into_owned()
        };
        [
            f(&u.sysname),
            f(&u.nodename),
            f(&u.release),
            f(&u.version),
            f(&u.machine),
        ]
    }
}

/// `php_get_uname(mode)` (ext/standard/info.c).
pub fn php_get_uname(mode: u8) -> String {
    let [s, n, r, v, m] = uname_fields();
    match mode {
        b's' => s,
        b'n' => n,
        b'r' => r,
        b'v' => v,
        b'm' => m,
        _ => format!("{s} {n} {r} {v} {m}"),
    }
}

fn fn_php_uname(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mode = match args.first().map(deref) {
        None => b'a',
        Some(v) => {
            let b = conv::to_string_bytes(&v);
            if b.len() != 1 {
                return Err(Thrown::error(
                    "ValueError",
                    "php_uname(): Argument #1 ($mode) must be a single character",
                ));
            }
            if !b"amnrsv".contains(&b[0]) {
                return Err(Thrown::error(
                    "ValueError",
                    "php_uname(): Argument #1 ($mode) must be one of \"a\", \"m\", \"n\", \"r\", \"s\", or \"v\"",
                ));
            }
            b[0]
        }
    };
    Ok(Value::str(php_get_uname(mode).into_bytes()))
}

/// Text-mode table/row printer mirroring info.c's helpers.
struct Info(Vec<u8>);

impl Info {
    fn print(&mut self, s: &str) {
        self.0.extend_from_slice(s.as_bytes());
    }
    fn table_start(&mut self) {
        self.print("\n");
    }
    fn header(&mut self, cols: &[&str]) {
        for (i, c) in cols.iter().enumerate() {
            self.print(if c.is_empty() { " " } else { c });
            self.print(if i + 1 < cols.len() { " => " } else { "\n" });
        }
    }
    fn row(&mut self, cols: &[&str]) {
        for (i, c) in cols.iter().enumerate() {
            if c.is_empty() {
                self.print(" ");
            } else {
                self.print(c);
                if i + 1 < cols.len() {
                    self.print(" => ");
                }
            }
            if i + 1 == cols.len() {
                self.print("\n");
            }
        }
    }
    fn section(&mut self, name: &str) {
        self.table_start();
        self.header(&[name]);
    }
    fn hr(&mut self) {
        self.print(
            "\n\n _______________________________________________________________________\n\n",
        );
    }
}

/// ini directives shown under "PHP Core" (name, value as php displays it).
const CORE_INI: &[(&str, &str)] = &[
    ("allow_url_fopen", "On"),
    ("allow_url_include", "Off"),
    ("default_charset", "UTF-8"),
    ("disable_functions", "no value"),
    ("display_errors", "STDOUT"),
    ("error_reporting", "no value"),
    ("max_execution_time", "0"),
    ("memory_limit", "128M"),
    ("open_basedir", "no value"),
    ("precision", "14"),
    ("serialize_precision", "-1"),
    ("short_open_tag", "On"),
];

fn print_info(flag: i64) -> Vec<u8> {
    let mut o = Info(Vec::new());
    o.print("phpinfo()\n");
    if flag & INFO_GENERAL != 0 {
        o.row(&["PHP Version", version::PHP_VERSION]);
        o.table_start();
        o.row(&["System", &php_get_uname(b'a')]);
        o.row(&["Build Date", version::BUILD_DATE]);
        o.row(&["Build System", version::BUILD_SYSTEM]);
        o.row(&["Build Provider", version::BUILD_PROVIDER]);
        o.row(&["Configure Command", " './configure' "]);
        o.row(&["Server API", "Command Line Interface"]);
        o.row(&["Virtual Directory Support", "disabled"]);
        o.row(&[
            "Configuration File (php.ini) Path",
            version::CONFIG_FILE_PATH,
        ]);
        o.row(&["Loaded Configuration File", "(none)"]);
        o.row(&["Scan this dir for additional .ini files", "(none)"]);
        o.row(&["Additional .ini files parsed", "(none)"]);
        o.row(&["PHP API", version::PHP_API]);
        o.row(&["PHP Extension", version::PHP_API]);
        o.row(&["Zend Extension", version::ZEND_EXTENSION_API]);
        o.row(&[
            "Zend Extension Build",
            &format!("API{},NTS", version::ZEND_EXTENSION_API),
        ]);
        o.row(&[
            "PHP Extension Build",
            &format!("API{},NTS", version::PHP_API),
        ]);
        o.row(&["PHP Integer Size", "64 bits"]);
        o.row(&["Debug Build", "no"]);
        o.row(&["Thread Safety", "disabled"]);
        o.row(&["Zend Signal Handling", "enabled"]);
        o.row(&["Zend Memory Manager", "enabled"]);
        let multibyte = if cfg!(feature = "mbstring-libmbfl") {
            "provided by mbstring"
        } else {
            "disabled"
        };
        o.row(&["Zend Multibyte Support", multibyte]);
        o.row(&["Zend Max Execution Timers", "disabled"]);
        o.row(&["IPv6 Support", "enabled"]);
        o.row(&["DTrace Support", "disabled"]);
        o.print("\nRegistered PHP Streams => https, compress.zlib, compress.bzip2, php, file, glob, data, http, phar, zip");
        o.print("\nRegistered Stream Socket Transports => tcp, udp, unix, ssl, tls");
        o.print("\nRegistered Stream Filters => zlib.*, bzip2.*, string.rot13, string.toupper, string.tolower, convert.*");
        o.print("\n");
        // Zend Engine box
        o.table_start();
        o.print("\n");
        o.print("This program makes use of the Zend Scripting Language Engine:\n");
        o.print(&format!(
            "Zend Engine v{}, Copyright (c) Zend Technologies\n",
            version::ZEND_VERSION
        ));
    }
    if flag & INFO_CONFIGURATION != 0 {
        o.hr();
        o.print("Configuration\n");
        if flag & INFO_MODULES == 0 {
            o.section("PHP Core");
            print_core_ini(&mut o);
        }
    }
    if flag & INFO_MODULES != 0 {
        let mut exts: Vec<&str> = crate::php_fns::loaded_extension_names().to_vec();
        exts.sort_by_key(|e| e.to_ascii_lowercase());
        for e in &exts {
            o.section(e);
            o.table_start();
            if e.eq_ignore_ascii_case("core") {
                o.row(&["PHP Version", version::PHP_VERSION]);
                print_core_ini(&mut o);
            } else if *e == "pcre" {
                print_pcre(&mut o);
            } else if *e == "openssl" {
                print_openssl(&mut o);
            } else if *e == "curl" {
                print_curl(&mut o);
            } else if *e == "iconv" {
                print_iconv(&mut o);
            } else if *e == "gmp" {
                o.row(&["gmp support", "enabled"]);
                o.row(&["GMP version", crate::ext::gmp::sys::version()]);
            } else if *e == "mbstring" {
                print_mbstring(&mut o);
            } else if *e == "date" {
                print_date(&mut o);
            } else if *e == "zip" {
                for (k, v) in crate::ext::zip::info_rows() {
                    o.row(&[&k, &v]);
                }
            } else {
                o.row(&[&format!("{e} support"), "enabled"]);
            }
        }
        o.section("Additional Modules");
        o.table_start();
        o.header(&["Module Name"]);
    }
    if flag & INFO_ENVIRONMENT != 0 {
        o.section("Environment");
        o.table_start();
        o.header(&["Variable", "Value"]);
        for (k, v) in std::env::vars_os() {
            o.row(&[&k.to_string_lossy(), &v.to_string_lossy()]);
        }
    }
    if flag & INFO_VARIABLES != 0 {
        o.section("PHP Variables");
        o.table_start();
        o.header(&["Variable", "Value"]);
        for (k, v) in std::env::vars_os() {
            o.row(&[
                &format!("$_SERVER['{}']", k.to_string_lossy()),
                &v.to_string_lossy(),
            ]);
        }
    }
    if flag & INFO_CREDITS != 0 {
        o.hr();
        o.print("PHP Credits\n");
    }
    if flag & INFO_LICENSE != 0 {
        o.print("\nPHP License\n");
        o.print("This program is free software; you can redistribute it and/or modify\n");
        o.print("it under the terms of the PHP License as published by the PHP Group\n");
        o.print("and included in the distribution in the file:  LICENSE\n");
        o.print("\n");
        o.print("This program is distributed in the hope that it will be useful,\n");
        o.print("but WITHOUT ANY WARRANTY; without even the implied warranty of\n");
        o.print("MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.\n");
        o.print("\n");
        o.print("If you did not receive a copy of the PHP license, or have any\n");
        o.print("questions about PHP licensing, please contact license@php.net.\n");
    }
    o.0
}

/// PHP_MINFO_FUNCTION(pcre), then its ini entries.
fn print_pcre(o: &mut Info) {
    o.row(&[
        "PCRE (Perl Compatible Regular Expressions) Support",
        "enabled",
    ]);
    o.row(&["PCRE Library Version", &crate::pcre::version()]);
    o.row(&["PCRE Unicode Version", &crate::pcre::unicode_version()]);
    let jit = crate::pcre::jit_supported();
    o.row(&["PCRE JIT Support", if jit { "enabled" } else { "disabled" }]);
    if let Some(t) = crate::pcre::jit_target() {
        o.row(&["PCRE JIT Target", &t]);
    }
    o.table_start();
    o.header(&["Directive", "Local Value", "Master Value"]);
    for name in ["pcre.backtrack_limit", "pcre.jit", "pcre.recursion_limit"] {
        let Some(local) = crate::runtime_fns::ini_value(name.as_bytes()) else {
            continue;
        };
        let master = crate::runtime_fns::ini_default_value(name.as_bytes()).flatten();
        let show = |v: Option<Vec<u8>>| {
            let v = v.unwrap_or_default();
            if name == "pcre.jit" {
                let on = crate::runtime_fns::ini_parse_bool(&v);
                (if on { "On" } else { "Off" }).to_string()
            } else {
                String::from_utf8_lossy(&v).into_owned()
            }
        };
        o.row(&[name, &show(local), &show(master)]);
    }
}

/// PHP_MINFO_FUNCTION(openssl), then its ini entries.
fn print_openssl(o: &mut Info) {
    for (k, v) in crate::ext::openssl::info_rows() {
        o.row(&[k, &v]);
    }
    o.table_start();
    o.header(&["Directive", "Local Value", "Master Value"]);
    for name in ["openssl.cafile", "openssl.capath", "openssl.libctx"] {
        let show = |v: Option<Vec<u8>>| match v {
            Some(v) => String::from_utf8_lossy(&v).into_owned(),
            None => "no value".to_string(),
        };
        let local = crate::runtime_fns::ini_value(name.as_bytes()).flatten();
        let master = crate::runtime_fns::ini_default_value(name.as_bytes()).flatten();
        o.row(&[name, &show(local), &show(master)]);
    }
}

/// PHP_MINFO_FUNCTION(date), then its ini entries.
fn print_date(o: &mut Info) {
    for (k, v) in crate::ext::date::info_rows() {
        o.row(&[k, &String::from_utf8_lossy(&v)]);
    }
    o.table_start();
    o.header(&["Directive", "Local Value", "Master Value"]);
    for name in [
        "date.default_latitude",
        "date.default_longitude",
        "date.sunrise_zenith",
        "date.sunset_zenith",
        "date.timezone",
    ] {
        let show = |v: Option<Vec<u8>>| match v {
            Some(v) if !v.is_empty() => String::from_utf8_lossy(&v).into_owned(),
            _ => "no value".to_string(),
        };
        let local = crate::runtime_fns::ini_value(name.as_bytes()).flatten();
        let master = crate::runtime_fns::ini_default_value(name.as_bytes()).flatten();
        o.row(&[name, &show(local), &show(master)]);
    }
}

/// PHP_MINFO_FUNCTION(curl), then its ini entry.
fn print_curl(o: &mut Info) {
    for row in crate::ext::curl::info_rows() {
        let cols: Vec<&str> = row.iter().map(String::as_str).collect();
        o.row(&cols);
    }
    o.table_start();
    o.header(&["Directive", "Local Value", "Master Value"]);
    let show = |v: Option<Vec<u8>>| match v {
        Some(v) if !v.is_empty() => String::from_utf8_lossy(&v).into_owned(),
        _ => "no value".to_string(),
    };
    let local = crate::runtime_fns::ini_value(b"curl.cainfo").flatten();
    let master = crate::runtime_fns::ini_default_value(b"curl.cainfo").flatten();
    o.row(&["curl.cainfo", &show(local), &show(master)]);
}

/// PHP_MINFO_FUNCTION(miconv), then its ini entries.
fn print_iconv(o: &mut Info) {
    for (k, v) in crate::ext::iconv::info_rows() {
        o.row(&[k, &v]);
    }
    o.table_start();
    o.header(&["Directive", "Local Value", "Master Value"]);
    for name in [
        "iconv.input_encoding",
        "iconv.internal_encoding",
        "iconv.output_encoding",
    ] {
        let show = |v: Option<Option<Vec<u8>>>| match v.flatten() {
            Some(v) if !v.is_empty() => String::from_utf8_lossy(&v).into_owned(),
            _ => "no value".to_string(),
        };
        let local = show(crate::runtime_fns::ini_value(name.as_bytes()));
        let master = show(crate::runtime_fns::ini_default_value(name.as_bytes()));
        o.row(&[name, &local, &master]);
    }
}

/// PHP_MINFO(mbstring), without PHP_MINFO(mb_regex): rhp has no mb_ereg.
fn print_mbstring(o: &mut Info) {
    let ini = |name: &str| crate::runtime_fns::ini_value(name.as_bytes()).flatten();
    let on = |v: &Option<Vec<u8>>| v.as_deref().is_some_and(crate::runtime_fns::ini_parse_bool);
    o.row(&["Multibyte Support", "enabled"]);
    o.row(&["Multibyte string engine", "libmbfl"]);
    let translation = on(&ini("mbstring.encoding_translation"));
    o.row(&[
        "HTTP input encoding translation",
        if translation { "enabled" } else { "disabled" },
    ]);
    #[cfg(feature = "mbstring-libmbfl")]
    o.row(&["libmbfl version", rhp_mbfl_sys::version()]);
    o.table_start();
    o.header(&["mbstring extension makes use of \"streamable kanji code filter and converter\", which is distributed under the GNU Lesser General Public License version 2.1."]);
    o.table_start();
    o.header(&["Directive", "Local Value", "Master Value"]);
    for name in [
        "mbstring.detect_order",
        "mbstring.encoding_translation",
        "mbstring.http_input",
        "mbstring.http_output",
        "mbstring.http_output_conv_mimetypes",
        "mbstring.internal_encoding",
        "mbstring.language",
        "mbstring.regex_retry_limit",
        "mbstring.regex_stack_limit",
        "mbstring.strict_detection",
        "mbstring.substitute_character",
    ] {
        let Some(local) = crate::runtime_fns::ini_value(name.as_bytes()) else {
            continue;
        };
        let master = crate::runtime_fns::ini_default_value(name.as_bytes()).flatten();
        // php_ini_boolean_displayer_cb for the two boolean entries.
        let boolean = matches!(
            name,
            "mbstring.encoding_translation" | "mbstring.strict_detection"
        );
        let show = |v: Option<Vec<u8>>| match v {
            _ if boolean => (if on(&v) { "On" } else { "Off" }).to_string(),
            Some(v) if !v.is_empty() => String::from_utf8_lossy(&v).into_owned(),
            _ => "no value".to_string(),
        };
        o.row(&[name, &show(local), &show(master)]);
    }
}

fn print_core_ini(o: &mut Info) {
    o.table_start();
    o.header(&["Directive", "Local Value", "Master Value"]);
    for (k, v) in CORE_INI {
        o.row(&[k, v, v]);
    }
}

fn fn_phpinfo(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let flag = match args.first().map(deref) {
        None => INFO_ALL,
        Some(v) => conv::to_int(&v),
    };
    let out = print_info(flag);
    host.write_out(&out);
    Ok(Value::Bool(true))
}

fn fn_phpcredits(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    host.write_out(b"PHP Credits\n");
    Ok(Value::Bool(true))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("phpinfo", fn_phpinfo),
            ("phpcredits", fn_phpcredits),
            ("php_uname", fn_php_uname),
        ],
    );
}
