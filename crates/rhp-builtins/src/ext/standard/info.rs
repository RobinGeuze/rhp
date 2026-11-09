//! ext/standard/info.c: `phpinfo()` / `phpcredits()` in php's CLI text
//! layout (`sapi_module.phpinfo_as_text`), `php_uname()` backed by uname(2),
//! `phpversion()` and the `php_ini_*` queries.
//!
//! The INFO_GENERAL block follows php's row order and formatting exactly; the
//! values describe this engine (rhp) where php would describe its build.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::zend::builtin_functions::{ext_version, loaded_ext};
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

/// info.c's printing helpers, in text (`phpinfo_as_text`, the CLI) or
/// HTML (php-cgi) mode.
pub(crate) struct Info(pub(crate) Vec<u8>, pub(crate) bool);

/// php_info_print_html_esc (htmlspecialchars, ENT_QUOTES).
fn html_esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#039;"),
            _ => o.push(c),
        }
    }
    o
}

impl Info {
    /// A printer for the running SAPI: HTML unless it is the CLI.
    pub(crate) fn for_sapi() -> Info {
        Info(Vec::new(), crate::main::sapi::sapi_name() != "cli")
    }
    pub(crate) fn print(&mut self, s: &str) {
        self.0.extend_from_slice(s.as_bytes());
    }
    pub(crate) fn table_start(&mut self) {
        self.print(if self.1 { "<table>\n" } else { "\n" });
    }
    pub(crate) fn table_end(&mut self) {
        if self.1 {
            self.print("</table>\n");
        }
    }
    /// php_info_print_table_header.
    pub(crate) fn header(&mut self, cols: &[&str]) {
        if self.1 {
            self.print("<tr class=\"h\">");
            for c in cols {
                self.print("<th>");
                self.print(if c.is_empty() { " " } else { c });
                self.print("</th>");
            }
            self.print("</tr>\n");
            return;
        }
        for (i, c) in cols.iter().enumerate() {
            self.print(if c.is_empty() { " " } else { c });
            self.print(if i + 1 < cols.len() { " => " } else { "\n" });
        }
    }
    /// php_info_print_table_row.
    pub(crate) fn row(&mut self, cols: &[&str]) {
        if self.1 {
            self.print("<tr>");
            for (i, c) in cols.iter().enumerate() {
                self.print(if i == 0 {
                    "<td class=\"e\">"
                } else {
                    "<td class=\"v\">"
                });
                if c.is_empty() {
                    self.print("<i>no value</i>");
                } else {
                    let e = html_esc(c);
                    self.print(&e);
                }
                self.print(" </td>");
            }
            self.print("</tr>\n");
            return;
        }
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
    /// A row of DISPLAY_INI_ENTRIES (display_ini_entries): in HTML its
    /// cells have no trailing space, unlike php_info_print_table_row's.
    pub(crate) fn ini_row(&mut self, cols: &[&str]) {
        if !self.1 {
            return self.row(cols);
        }
        self.print("<tr>");
        for (i, c) in cols.iter().enumerate() {
            self.print(if i == 0 {
                "<td class=\"e\">"
            } else {
                "<td class=\"v\">"
            });
            if c.is_empty() || (i > 0 && *c == "no value") {
                self.print("<i>no value</i>");
            } else {
                let e = html_esc(c);
                self.print(&e);
            }
            self.print("</td>");
        }
        self.print("</tr>\n");
    }
    /// php_info_print_table_colspan_header.
    pub(crate) fn colspan_header(&mut self, n: usize, header: &str) {
        if self.1 {
            self.print(&format!(
                "<tr class=\"h\"><th colspan=\"{n}\">{header}</th></tr>\n"
            ));
        } else {
            let spaces = (74 - header.len() as i32) / 2;
            let pad = " ".repeat(spaces.max(1) as usize);
            self.print(&format!("{pad}{header}{pad}\n"));
        }
    }
    /// The SECTION macro.
    pub(crate) fn section(&mut self, name: &str) {
        if self.1 {
            self.print(&format!("<h2>{name}</h2>\n"));
        } else {
            self.table_start();
            self.header(&[name]);
            self.table_end();
        }
    }
    /// php_info_print_module's heading.
    fn module_section(&mut self, name: &str) {
        if self.1 {
            let url = name.to_ascii_lowercase().replace(' ', "+");
            self.print(&format!(
                "<h2><a name=\"module_{url}\" href=\"#module_{url}\">{name}</a></h2>\n"
            ));
        } else {
            self.section(name);
        }
    }
    fn box_start(&mut self, head: bool) {
        self.table_start();
        if self.1 {
            self.print(if head {
                "<tr class=\"h\"><td>\n"
            } else {
                "<tr class=\"v\"><td>\n"
            });
        } else if !head {
            self.print("\n");
        }
    }
    fn box_end(&mut self) {
        if self.1 {
            self.print("</td></tr>\n");
        }
        self.table_end();
    }
    fn hr(&mut self) {
        self.print(if self.1 {
            "<hr />\n"
        } else {
            "\n\n _______________________________________________________________________\n\n"
        });
    }
}

/// php_print_info_htmlhead.
pub(crate) fn print_htmlhead(o: &mut Info) {
    o.print("<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Transitional//EN\" \"DTD/xhtml1-transitional.dtd\">\n");
    o.print("<html xmlns=\"http://www.w3.org/1999/xhtml\"><head>\n");
    o.print("<style type=\"text/css\">\n");
    o.print(CSS);
    o.print("</style>\n");
    o.print(&format!(
        "<title>PHP {} - phpinfo()</title>",
        version::PHP_VERSION
    ));
    o.print("<meta name=\"ROBOTS\" content=\"NOINDEX,NOFOLLOW,NOARCHIVE\" /></head>\n");
    o.print("<body><div class=\"center\">\n");
}

/// ext/standard/css.c.
const CSS: &str = "body {background-color: #fff; color: #222; font-family: sans-serif;}
pre {margin: 0; font-family: monospace;}
a {color: inherit;}
a:hover {text-decoration: none;}
table {border-collapse: collapse; border: 0; width: 934px; box-shadow: 1px 2px 3px rgba(0, 0, 0, 0.2);}
.center {text-align: center;}
.center table {margin: 1em auto; text-align: left;}
.center th {text-align: center !important;}
td, th {border: 1px solid #666; font-size: 75%; vertical-align: baseline; padding: 4px 5px;}
th {position: sticky; top: 0; background: inherit;}
h1 {font-size: 150%;}
h2 {font-size: 125%;}
h2 > a {text-decoration: none;}
h2 > a:hover {text-decoration: underline;}
.p {text-align: left;}
.e {background-color: #ccf; width: 300px; font-weight: bold;}
.h {background-color: #99c; font-weight: bold;}
.v {background-color: #ddd; max-width: 300px; overflow-x: auto; word-wrap: break-word;}
.v i {color: #999;}
img {float: right; border: 0;}
hr {width: 934px; background-color: #ccc; border: 0; height: 1px;}
:root {--php-dark-grey: #333; --php-dark-blue: #4F5B93; --php-medium-blue: #8892BF; --php-light-blue: #E2E4EF; --php-accent-purple: #793862}@media (prefers-color-scheme: dark) {
  body {background: var(--php-dark-grey); color: var(--php-light-blue)}
  .h td, td.e, th {border-color: #606A90}
  td {border-color: #505153}
  .e {background-color: #404A77}
  .h {background-color: var(--php-dark-blue)}
  .v {background-color: var(--php-dark-grey)}
  hr {background-color: #505153}
}
";

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

/// php_print_gpcse_array in text mode: each element of superglobal `name`
/// as it is now, arrays print_r'd.
fn print_gpcse_array(host: &mut dyn Host, o: &mut Info, name: &str) {
    let Some(Value::Array(a)) = host.global_var(name.as_bytes()) else {
        return;
    };
    let html = o.1;
    for (k, v) in a.iter() {
        if html {
            o.print("<tr><td class=\"e\">");
        }
        o.print("$");
        o.print(name);
        o.print("['");
        let key = conv::to_string_bytes(&k.to_value());
        if html {
            o.print(&html_esc(&String::from_utf8_lossy(&key)));
        } else {
            o.0.extend_from_slice(&key);
        }
        o.print(if html {
            "']</td><td class=\"v\">"
        } else {
            "'] => "
        });
        let v = deref(v);
        let mut val = Vec::new();
        if matches!(v, Value::Array(_)) {
            crate::ext::standard::var::print_r(host, &mut val, &v, 0);
        } else {
            val = conv::to_string_bytes(&v);
        }
        if !html {
            o.0.extend_from_slice(&val);
            o.print("\n");
            continue;
        }
        let esc = html_esc(&String::from_utf8_lossy(&val));
        if matches!(v, Value::Array(_)) {
            o.print(&format!("<pre>{esc}</pre>"));
        } else if val.is_empty() {
            o.print("<i>no value</i>");
        } else {
            o.print(&esc);
        }
        o.print("</td></tr>\n");
    }
}

fn print_info(host: &mut dyn Host, flag: i64) -> Vec<u8> {
    let mut o = Info::for_sapi();
    if o.1 {
        print_htmlhead(&mut o);
    } else {
        o.print("phpinfo()\n");
    }
    if flag & INFO_GENERAL != 0 {
        if o.1 {
            o.box_start(true);
            o.print(&format!(
                "<h1 class=\"p\">PHP Version {}</h1>\n",
                version::PHP_VERSION
            ));
            o.box_end();
        } else {
            o.row(&["PHP Version", version::PHP_VERSION]);
        }
        o.table_start();
        o.row(&["System", &php_get_uname(b'a')]);
        o.row(&["Build Date", version::BUILD_DATE]);
        o.row(&["Build System", version::BUILD_SYSTEM]);
        o.row(&["Build Provider", version::BUILD_PROVIDER]);
        if version::HAS_CONFIGURE_COMMAND {
            o.row(&["Configure Command", " './configure' "]);
        }
        let api = if o.1 {
            "CGI/FastCGI"
        } else {
            "Command Line Interface"
        };
        o.row(&["Server API", api]);
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
        for (name, list) in [
            (
                "PHP Streams",
                "https, compress.zlib, compress.bzip2, php, file, glob, data, http, phar, zip",
            ),
            ("Stream Socket Transports", "tcp, udp, unix, ssl, tls"),
            (
                "Stream Filters",
                "zlib.*, bzip2.*, string.rot13, string.toupper, string.tolower, convert.*",
            ),
        ] {
            if o.1 {
                o.print(&format!(
                    "<tr><td class=\"e\">Registered {name}</td><td class=\"v\">{list}</td></tr>\n"
                ));
            } else {
                o.print(&format!("\nRegistered {name} => {list}"));
            }
        }
        o.table_end();
        // Zend Engine box
        o.box_start(false);
        o.print("This program makes use of the Zend Scripting Language Engine:");
        o.print(if o.1 { "<br />" } else { "\n" });
        o.print(&format!(
            "Zend Engine v{}, Copyright (c) Zend Technologies\n",
            version::ZEND_VERSION
        ));
        o.print(&format!(
            "    with Zend OPcache v{}, Copyright (c), by Zend Technologies\n",
            version::PHP_VERSION
        ));
        o.box_end();
    }
    if flag & INFO_CONFIGURATION != 0 {
        o.hr();
        if o.1 {
            o.print("<h1>Configuration</h1>\n");
        } else {
            o.section("Configuration");
        }
        if flag & INFO_MODULES == 0 {
            o.section("PHP Core");
            print_core_ini(&mut o);
            o.table_end();
        }
    }
    if flag & INFO_MODULES != 0 {
        let mut exts: Vec<&str> = crate::zend::builtin_functions::loaded_extension_names();
        exts.sort_by_key(|e| e.to_ascii_lowercase());
        for e in &exts {
            print_module(&mut o, e);
        }
        o.section("Additional Modules");
        o.table_start();
        o.header(&["Module Name"]);
        o.table_end();
    }
    if flag & INFO_ENVIRONMENT != 0 {
        o.section("Environment");
        o.table_start();
        o.header(&["Variable", "Value"]);
        for (k, v) in std::env::vars_os() {
            o.row(&[&k.to_string_lossy(), &v.to_string_lossy()]);
        }
        o.table_end();
    }
    if flag & INFO_VARIABLES != 0 {
        o.section("PHP Variables");
        o.table_start();
        o.header(&["Variable", "Value"]);
        for name in ["PHP_SELF", "PHP_AUTH_TYPE", "PHP_AUTH_USER", "PHP_AUTH_PW"] {
            if let Some(Value::Str(v)) = host.global_var(name.as_bytes()) {
                o.row(&[name, &String::from_utf8_lossy(v.as_bytes())]);
            }
        }
        for name in [
            "_REQUEST", "_GET", "_POST", "_FILES", "_COOKIE", "_SERVER", "_ENV",
        ] {
            print_gpcse_array(host, &mut o, name);
        }
        o.table_end();
    }
    if flag & INFO_CREDITS != 0 {
        o.hr();
        use crate::ext::standard::credits;
        credits::print_credits(&mut o, credits::CREDITS_ALL & !credits::CREDITS_FULLPAGE);
    }
    if flag & INFO_LICENSE != 0 && o.1 {
        o.section("PHP License");
        o.box_start(false);
        o.print("<p>\n");
        o.print("This program is free software; you can redistribute it and/or modify ");
        o.print("it under the terms of the PHP License as published by the PHP Group ");
        o.print("and included in the distribution in the file:  LICENSE\n");
        o.print("</p>\n");
        o.print("<p>");
        o.print("This program is distributed in the hope that it will be useful, ");
        o.print("but WITHOUT ANY WARRANTY; without even the implied warranty of ");
        o.print("MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.\n");
        o.print("</p>\n");
        o.print("<p>");
        o.print("If you did not receive a copy of the PHP license, or have any questions about ");
        o.print("PHP licensing, please contact license@php.net.\n");
        o.print("</p>\n");
        o.box_end();
    } else if flag & INFO_LICENSE != 0 {
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
    if o.1 {
        o.print("</div></body></html>");
    }
    o.0
}

/// php_info_print_module: one loaded module's heading and its MINFO
/// section.
fn print_module(o: &mut Info, e: &str) {
    o.module_section(e);
    o.table_start();
    if e.eq_ignore_ascii_case("core") {
        o.row(&["PHP Version", version::PHP_VERSION]);
        o.table_end();
        print_core_ini(o);
    } else if e == "pcre" {
        print_pcre(o);
    } else if e == "openssl" {
        print_openssl(o);
    } else if e == "curl" {
        print_curl(o);
    } else if e == "iconv" {
        print_iconv(o);
    } else if e == "gmp" {
        o.row(&["gmp support", "enabled"]);
        o.row(&["GMP version", crate::ext::gmp::sys::version()]);
    } else if e == "mbstring" {
        print_mbstring(o);
    } else if e == "date" {
        print_date(o);
    } else if e == "session" {
        print_session(o);
    } else if e == "PDO" || e == "pdo_sqlite" {
        let rows = if e == "PDO" {
            crate::ext::pdo::info_rows()
        } else {
            crate::ext::pdo_sqlite::info_rows()
        };
        for (k, v) in rows {
            o.row(&[&k, &v]);
        }
    } else if e == "sqlite3" {
        o.row(&["SQLite3 support", "enabled"]);
        o.row(&["SQLite Library", &crate::ext::pdo_sqlite::sys::libversion()]);
        o.table_end();
        print_ini(
            o,
            &["sqlite3.defensive", "sqlite3.extension_dir"],
            &["sqlite3.defensive"],
        );
    } else if e == "fileinfo" {
        o.row(&["fileinfo support", "enabled"]);
        o.row(&["libmagic", &crate::ext::fileinfo::libmagic_version()]);
    } else if e == "gd" {
        for (k, v) in crate::ext::gd::info_rows() {
            o.row(&[k, &v]);
        }
        o.table_end();
        print_ini(o, &["gd.jpeg_ignore_warning"], &["gd.jpeg_ignore_warning"]);
    } else if e == "intl" {
        for (k, v) in crate::ext::intl::info_rows() {
            o.row(&[k, &v]);
        }
        o.table_end();
        print_ini(
            o,
            &[
                "intl.default_locale",
                "intl.error_level",
                "intl.use_exceptions",
            ],
            &["intl.use_exceptions"],
        );
    } else if e == "zip" {
        for (k, v) in crate::ext::zip::info_rows() {
            o.row(&[&k, &v]);
        }
    } else if e == "bz2" {
        for (k, v) in crate::ext::bz2::info_rows() {
            o.row(&[k, &v]);
        }
    } else if e == "readline" {
        o.row(&["Readline Support", "enabled"]);
        let v = crate::ext::readline::library_version();
        o.row(&["Readline library", &String::from_utf8_lossy(&v)]);
    } else if let Some(rows) = static_minfo(e) {
        for (k, v) in rows {
            o.row(&[k, &v]);
        }
    } else if e == "hash" {
        o.row(&["hash support", "enabled"]);
        let mut engines = String::new();
        for a in crate::ext::hash::algo_names() {
            engines.push_str(a);
            engines.push(' ');
        }
        o.row(&["Hashing Engines", &engines]);
    } else if e == "zlib" {
        // SAFETY: zlibVersion returns a static NUL-terminated string.
        let linked = unsafe { std::ffi::CStr::from_ptr(libz_sys::zlibVersion()) };
        o.row(&["ZLib Support", "enabled"]);
        o.row(&["Stream Wrapper", "compress.zlib://"]);
        o.row(&["Stream Filter", "zlib.inflate, zlib.deflate"]);
        o.row(&["Compiled Version", version::zlib_version()]);
        o.row(&["Linked Version", &linked.to_string_lossy()]);
        o.table_end();
        print_ini(
            o,
            &[
                "zlib.output_compression",
                "zlib.output_compression_level",
                "zlib.output_handler",
            ],
            &["zlib.output_compression"],
        );
    } else {
        o.row(&[&format!("{e} support"), "enabled"]);
    }
    let ini: &[&str] = match e {
        "bcmath" => &["bcmath.scale"],
        "filter" => &["filter.default", "filter.default_flags"],
        "readline" => &["cli.pager", "cli.prompt"],
        "exif" => &[
            "exif.decode_jis_intel",
            "exif.decode_jis_motorola",
            "exif.decode_unicode_intel",
            "exif.decode_unicode_motorola",
            "exif.encode_jis",
            "exif.encode_unicode",
        ],
        _ => &[],
    };
    if !ini.is_empty() {
        o.table_end();
        print_ini(o, ini, &[]);
    }
    o.table_end();
}

/// The rows of the PHP_MINFO functions that print fixed text and
/// versions, then their ini entries (DISPLAY_INI_ENTRIES).
fn static_minfo(e: &str) -> Option<Vec<(&'static str, String)>> {
    let s = |v: &str| v.to_string();
    let xml = || s(crate::ext::libxml::header_dotted_version());
    Some(match e {
        "bcmath" => vec![("BCMath support", s("enabled"))],
        "calendar" => vec![("Calendar support", s("enabled"))],
        "gettext" => vec![("GetText Support", s("enabled"))],
        "sockets" => vec![("Sockets Support", s("enabled"))],
        "sodium" => crate::ext::sodium::info_rows(),
        "exif" => crate::ext::exif::info_rows(),
        "ctype" => vec![("ctype functions", s("enabled"))],
        "filter" => vec![("Input Validation and Filtering", s("enabled"))],
        "posix" => vec![("POSIX support", s("enabled"))],
        "random" => vec![("Version", s(version::PHP_VERSION))],
        "Reflection" => vec![("Reflection", s("enabled"))],
        "tokenizer" => vec![("Tokenizer Support", s("enabled"))],
        "xmlreader" => vec![("XMLReader", s("enabled"))],
        "xmlwriter" => vec![("XMLWriter", s("enabled"))],
        "SimpleXML" => vec![
            ("SimpleXML support", s("enabled")),
            ("Schema support", s("enabled")),
        ],
        "libxml" => vec![
            ("libXML support", s("active")),
            ("libXML Compiled Version", xml()),
            (
                "libXML Loaded Version",
                s(crate::ext::libxml::loaded_version()),
            ),
            ("libXML streams", s("enabled")),
        ],
        "xml" => vec![
            ("XML Support", s("active")),
            ("XML Namespace Support", s("active")),
            ("libxml2 Version", xml()),
        ],
        "dom" => vec![
            ("DOM/XML", s("enabled")),
            ("DOM/XML API Version", s("20031129")),
            ("libxml Version", xml()),
            ("HTML Support", s("enabled")),
            ("XPath Support", s("enabled")),
            ("XPointer Support", s("enabled")),
            ("Schema Support", s("enabled")),
            ("RelaxNG Support", s("enabled")),
        ],
        _ => return None,
    })
}

/// DISPLAY_INI_ENTRIES for `names` (php_ini_boolean_displayer_cb for
/// `bools`), after a module's rows.
fn print_ini(o: &mut Info, names: &[&str], bools: &[&str]) {
    o.table_start();
    o.header(&["Directive", "Local Value", "Master Value"]);
    for name in names {
        let boolean = bools.contains(name);
        let show = |v: Option<Vec<u8>>| match v {
            v if boolean => {
                let on = v.as_deref().is_some_and(crate::main::ini::ini_parse_bool);
                (if on { "On" } else { "Off" }).to_string()
            }
            Some(v) if !v.is_empty() => String::from_utf8_lossy(&v).into_owned(),
            _ => "no value".to_string(),
        };
        let local = crate::main::ini::ini_value(name.as_bytes()).flatten();
        let master = crate::main::ini::ini_default_value(name.as_bytes()).flatten();
        o.ini_row(&[name, &show(local), &show(master)]);
    }
}

/// ReflectionExtension::info(): php_info_print_module for one module, as the
/// running SAPI prints it.
pub(crate) fn module_info(e: &str) -> Vec<u8> {
    let mut o = Info::for_sapi();
    print_module(&mut o, e);
    o.0
}

/// PHP_MINFO_FUNCTION(pcre), then its ini entries.
fn print_pcre(o: &mut Info) {
    o.row(&[
        "PCRE (Perl Compatible Regular Expressions) Support",
        "enabled",
    ]);
    o.row(&["PCRE Library Version", &crate::ext::pcre::version()]);
    o.row(&["PCRE Unicode Version", &crate::ext::pcre::unicode_version()]);
    let jit = crate::ext::pcre::jit_supported();
    o.row(&["PCRE JIT Support", if jit { "enabled" } else { "disabled" }]);
    if let Some(t) = crate::ext::pcre::jit_target() {
        o.row(&["PCRE JIT Target", &t]);
    }
    o.table_end();
    o.table_start();
    o.header(&["Directive", "Local Value", "Master Value"]);
    // php_pcre.c's PHP_INI_BEGIN order.
    for name in ["pcre.backtrack_limit", "pcre.recursion_limit", "pcre.jit"] {
        let Some(local) = crate::main::ini::ini_value(name.as_bytes()) else {
            continue;
        };
        let master = crate::main::ini::ini_default_value(name.as_bytes()).flatten();
        let show = |v: Option<Vec<u8>>| {
            let v = v.unwrap_or_default();
            if name == "pcre.jit" {
                let on = crate::main::ini::ini_parse_bool(&v);
                (if on { "On" } else { "Off" }).to_string()
            } else {
                String::from_utf8_lossy(&v).into_owned()
            }
        };
        o.ini_row(&[name, &show(local), &show(master)]);
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
        let local = crate::main::ini::ini_value(name.as_bytes()).flatten();
        let master = crate::main::ini::ini_default_value(name.as_bytes()).flatten();
        o.ini_row(&[name, &show(local), &show(master)]);
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
        let local = crate::main::ini::ini_value(name.as_bytes()).flatten();
        let master = crate::main::ini::ini_default_value(name.as_bytes()).flatten();
        o.ini_row(&[name, &show(local), &show(master)]);
    }
}

/// PHP_MINFO_FUNCTION(session), then its ini entries.
fn print_session(o: &mut Info) {
    o.row(&["Session Support", "enabled"]);
    o.row(&["Registered save handlers", "files user "]);
    o.row(&[
        "Registered serializer handlers",
        "php_serialize php php_binary ",
    ]);
    o.table_start();
    o.header(&["Directive", "Local Value", "Master Value"]);
    const BOOLS: &[&str] = &[
        "auto_start",
        "cookie_httponly",
        "cookie_partitioned",
        "cookie_secure",
        "lazy_write",
        "upload_progress.cleanup",
        "upload_progress.enabled",
        "use_cookies",
        "use_only_cookies",
        "use_strict_mode",
        "use_trans_sid",
    ];
    let names: Vec<&str> = crate::main::ini_table::INI_TABLE
        .iter()
        .filter(|e| e.0 == "session")
        .map(|e| e.1)
        .collect();
    for name in names {
        let boolean = BOOLS.contains(&&name["session.".len()..]);
        let show = |v: Option<Vec<u8>>| match v {
            Some(v) if boolean => {
                let on = crate::main::ini::ini_parse_bool(&v);
                (if on { "On" } else { "Off" }).to_string()
            }
            Some(v) if !v.is_empty() => String::from_utf8_lossy(&v).into_owned(),
            _ => "no value".to_string(),
        };
        let local = crate::main::ini::ini_value(name.as_bytes()).flatten();
        let master = crate::main::ini::ini_default_value(name.as_bytes()).flatten();
        o.ini_row(&[name, &show(local), &show(master)]);
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
    let local = crate::main::ini::ini_value(b"curl.cainfo").flatten();
    let master = crate::main::ini::ini_default_value(b"curl.cainfo").flatten();
    o.ini_row(&["curl.cainfo", &show(local), &show(master)]);
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
        let local = show(crate::main::ini::ini_value(name.as_bytes()));
        let master = show(crate::main::ini::ini_default_value(name.as_bytes()));
        o.ini_row(&[name, &local, &master]);
    }
}

/// PHP_MINFO(mbstring), with PHP_MINFO(mb_regex).
fn print_mbstring(o: &mut Info) {
    let ini = |name: &str| crate::main::ini::ini_value(name.as_bytes()).flatten();
    let on = |v: &Option<Vec<u8>>| v.as_deref().is_some_and(crate::main::ini::ini_parse_bool);
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
    #[cfg(feature = "mbstring-libmbfl")]
    {
        o.table_start();
        o.row(&["Multibyte (japanese) regex support", "enabled"]);
        o.row(&[
            "Multibyte regex (oniguruma) version",
            crate::ext::mbstring::ONIGURUMA_VERSION,
        ]);
    }
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
        let Some(local) = crate::main::ini::ini_value(name.as_bytes()) else {
            continue;
        };
        let master = crate::main::ini::ini_default_value(name.as_bytes()).flatten();
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
        o.ini_row(&[name, &show(local), &show(master)]);
    }
}

fn print_core_ini(o: &mut Info) {
    o.table_start();
    o.header(&["Directive", "Local Value", "Master Value"]);
    for (k, v) in CORE_INI {
        o.ini_row(&[k, v, v]);
    }
}

fn fn_phpinfo(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let flag = match args.first().map(deref) {
        None => INFO_ALL,
        Some(v) => conv::to_int(&v),
    };
    let out = print_info(host, flag);
    host.write_out(&out);
    Ok(Value::Bool(true))
}

fn fn_phpversion(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // phpversion($extension) returns the ext's version or false; plain → engine.
    match args.first() {
        Some(v) if !matches!(deref(v), Value::Null) => {
            match loaded_ext(&conv::to_string_bytes(&deref(v))) {
                Some(e) => Ok(Value::str(ext_version(e))),
                None => Ok(Value::Bool(false)),
            }
        }
        _ => Ok(Value::str(crate::version::PHP_VERSION)),
    }
}

fn fn_php_sapi_name(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(crate::main::sapi::sapi_name()))
}

/// rhp has no php.ini, so this reports "no scanned files".
fn fn_php_ini_scanned_files(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(crate::main::ini::config_files()
        .1
        .map_or(Value::Bool(false), Value::str))
}

fn fn_php_ini_loaded_file(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(crate::main::ini::config_files()
        .0
        .map_or(Value::Bool(false), Value::str))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("phpinfo", fn_phpinfo),
            ("php_uname", fn_php_uname),
            ("phpversion", fn_phpversion),
            ("php_sapi_name", fn_php_sapi_name),
            ("php_ini_scanned_files", fn_php_ini_scanned_files),
            ("php_ini_loaded_file", fn_php_ini_loaded_file),
        ],
    );
}
