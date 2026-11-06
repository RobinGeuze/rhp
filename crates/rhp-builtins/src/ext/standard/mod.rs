//! php's standard extension (php-src ext/standard/), one file per php-src
//! file.

use std::sync::OnceLock;

use rhp_values::Value;

use crate::module::{no_wrappers, ModuleDef};

pub mod array;
pub mod assert;
pub mod base64;
pub mod basic_functions;
pub mod browscap;
pub mod crc32;
pub mod crypt;
pub mod datetime;
pub mod dir;
pub mod dl;
pub mod dns;
pub mod exec;
pub mod file;
pub mod filestat;
pub mod formatted_print;
pub mod fsock;
pub mod head;
pub mod hebrev;
pub mod hrtime;
pub mod html;
#[rustfmt::skip]
pub mod html_tables;
pub mod http;
pub mod http_fopen_wrapper;
pub mod image;
pub mod info;
pub mod iptc;
pub mod levenshtein;
pub mod link;
pub mod mail;
pub mod math;
pub mod md5;
pub mod meta_tags;
pub mod metaphone;
pub mod microtime;
pub mod net;
pub mod pack;
pub mod pageinfo;
pub mod password;
pub mod proc_open;
pub mod quot_print;
pub mod scanf;
pub mod sha1;
pub mod soundex;
pub mod streamsfuncs;
pub mod string;
pub mod syslog;
pub mod r#type;
pub mod uniqid;
pub mod url;
pub mod url_scanner_ex;
pub mod uuencode;
pub mod var;
pub mod versioning;

/// The standard module. Its functions include the ones of main/ that php
/// declares in basic_functions.stub.php (output control, `header`,
/// `set_time_limit`). php registers a SAPI's additional functions under it
/// too; rhp keeps those with the SAPI (sapi/).
pub(crate) fn module() -> ModuleDef {
    ModuleDef {
        name: "standard",
        cli_only: false,
        functions: register,
        constants,
        prelude_php: prelude(),
        wrappers: no_wrappers,
        reset: reset_state,
        object_freed: Some(array::object_freed),
    }
}

fn register(m: &mut crate::Registry) {
    string::register(m);
    base64::register(m);
    md5::register(m);
    sha1::register(m);
    crc32::register(m);
    math::register(m);
    array::register(m);
    basic_functions::register(m);
    browscap::register(m);
    file::register(m);
    filestat::register(m);
    dir::register(m);
    link::register(m);
    r#type::register(m);
    password::register(m);
    url::register(m);
    url_scanner_ex::register(m);
    head::register(m);
    html::register(m);
    var::register(m);
    formatted_print::register(m);
    scanf::register(m);
    soundex::register(m);
    metaphone::register(m);
    hebrev::register(m);
    quot_print::register(m);
    uuencode::register(m);
    meta_tags::register(m);
    levenshtein::register(m);
    pack::register(m);
    uniqid::register(m);
    microtime::register(m);
    hrtime::register(m);
    assert::register(m);
    pageinfo::register(m);
    dns::register(m);
    syslog::register(m);
    net::register(m);
    datetime::register(m);
    iptc::register(m);
    image::register(m);
    mail::register(m);
    versioning::register(m);
    info::register(m);
    streamsfuncs::register(m);
    fsock::register(m);
    http::register(m);
    crypt::register(m);
    proc_open::register(m);
    exec::register(m);
    crate::main::output::register(m);
    crate::main::register(m);
    crate::main::sapi::register(m);
}

fn constants() -> Vec<(&'static str, Value)> {
    [
        basic_functions::constants(),
        array::constants(),
        string::constants(),
        math::constants(),
        file::constants(),
        file::fnmatch_constants(),
        dir::constants(),
        url::constants(),
        http::constants(),
        html::constants(),
        info::constants(),
        crypt::constants(),
        streamsfuncs::constants(),
        password::constants(),
        assert::constants(),
        image::constants(),
        dns::constants(),
        syslog::constants(),
    ]
    .concat()
}

/// The module's PHP source: dir.c's `Directory` class, string.c's
/// prelude functions and math.c's `RoundingMode` enum.
fn prelude() -> &'static str {
    static PRELUDE: OnceLock<String> = OnceLock::new();
    PRELUDE.get_or_init(|| {
        format!(
            "{}\n{}\n{}",
            dir::PRELUDE_PHP,
            string::PRELUDE_PHP,
            math::PRELUDE_PHP
        )
    })
}

fn reset_state() {
    var::clear_views();
    array::reset_state();
    string::reset_state();
    basic_functions::reset_state();
    url_scanner_ex::reset_state();
}
