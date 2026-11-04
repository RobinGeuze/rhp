//! `gen curl-consts --php-src <dir> [--check]`: ext/curl's constants.
//!
//! Names, order, version gates and deprecations come from php-src's
//! ext/curl/curl.stub.php (each constant's `@cvalue` and the
//! `#if LIBCURL_VERSION_NUM` blocks around it), plus the `#define`s php adds
//! in curl_private.h. The values are not generated: `consts.c` evaluates
//! each `@cvalue` against the curl.h of the libcurl rhp links, at build time,
//! as php's curl_arginfo.h does. `INTERNAL` adds the C names rhp's port of
//! ext/curl calls libcurl with that php does not export.

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use super::util::{finish, flag, read};

const OUT_C: &str = "crates/rhp-builtins/src/ext/curl/consts.c";
const OUT_RS: &str = "crates/rhp-builtins/src/ext/curl/consts.rs";

/// libcurl names the port passes to libcurl that are not php constants.
const INTERNAL: &[&str] = &[
    "CURL_ERROR_SIZE",
    "CURL_GLOBAL_DEFAULT",
    "CURL_BLOB_COPY",
    "CURL_READFUNC_ABORT",
    "CURL_SEEKFUNC_OK",
    "CURL_SEEKFUNC_CANTSEEK",
    "CURL_WRITEFUNC_ERROR",
    "CURLE_UNKNOWN_OPTION",
    "CURLM_UNKNOWN_OPTION",
    "CURLSHE_OK",
    "CURLSHE_BAD_OPTION",
    "CURLINFO_TYPEMASK",
    "CURLINFO_STRING",
    "CURLINFO_LONG",
    "CURLINFO_DOUBLE",
    "CURLINFO_SLIST",
    "CURLINFO_OFF_T",
    "CURLOPT_ERRORBUFFER",
    "CURLOPT_WRITEDATA",
    "CURLOPT_HEADERDATA",
    "CURLOPT_DEBUGDATA",
    "CURLOPT_PROGRESSDATA",
    "CURLOPT_XFERINFODATA",
    "CURLOPT_FNMATCH_DATA",
    "CURLOPT_PREREQDATA",
    "CURLOPT_SSH_HOSTKEYDATA",
    "CURLOPT_POSTFIELDSIZE",
    "CURLOPT_COPYPOSTFIELDS",
    "CURLOPT_MIMEPOST",
    "CURLMOPT_PUSHDATA",
];

struct Const {
    name: String,
    cvalue: String,
    /// The `#if` conditions it sits in, outermost first.
    conds: Vec<String>,
    /// `#[\Deprecated]`: (since, message).
    deprecated: Option<(String, String)>,
}

pub fn main(args: &[String]) -> ExitCode {
    let check = args.iter().any(|a| a == "--check");
    finish("curl-consts", check, files(args))
}

fn files(args: &[String]) -> Result<Vec<(&'static str, String)>, String> {
    let src =
        flag(args, "--php-src").ok_or("gen curl-consts needs --php-src <php-src checkout>")?;
    let dir = Path::new(src).join("ext/curl");
    let consts = parse_stub(&read(&dir.join("curl.stub.php"))?)?;
    let defines = php_defines(&read(&dir.join("curl_private.h"))?);
    Ok(vec![
        (OUT_C, gen_c(&consts, &defines)),
        (OUT_RS, gen_rs(&consts)),
    ])
}

fn parse_stub(text: &str) -> Result<Vec<Const>, String> {
    let mut out = Vec::new();
    let mut conds: Vec<String> = Vec::new();
    let mut cvalue: Option<String> = None;
    let mut deprecated: Option<(String, String)> = None;
    let mut in_doc = false;
    for (n, line) in text.lines().enumerate() {
        let t = line.trim();
        if in_doc {
            if let Some(v) = t.strip_prefix("* @cvalue ") {
                cvalue = Some(v.trim().to_string());
            }
            if t.ends_with("*/") {
                in_doc = false;
            }
            continue;
        }
        if t.starts_with("/**") {
            in_doc = !t.ends_with("*/");
            cvalue = None;
        } else if let Some(c) = t.strip_prefix("#if ") {
            conds.push(c.to_string());
        } else if t == "#endif" {
            conds
                .pop()
                .ok_or_else(|| format!("curl.stub.php:{}: #endif without #if", n + 1))?;
        } else if let Some(attr) = t.strip_prefix("#[\\Deprecated(") {
            deprecated = Some(parse_deprecated(attr).ok_or_else(|| {
                format!("curl.stub.php:{}: unreadable Deprecated attribute", n + 1)
            })?);
        } else if t.starts_with('#') {
            return Err(format!(
                "curl.stub.php:{}: unsupported preprocessor line {t:?}",
                n + 1
            ));
        } else if let Some(rest) = t.strip_prefix("const ") {
            let name = rest
                .split(|c: char| c == '=' || c.is_whitespace())
                .next()
                .unwrap_or_default()
                .to_string();
            let cvalue = cvalue
                .take()
                .ok_or_else(|| format!("curl.stub.php:{}: {name} has no @cvalue", n + 1))?;
            out.push(Const {
                name,
                cvalue,
                conds: conds.clone(),
                deprecated: deprecated.take(),
            });
        } else if !t.is_empty() && !t.starts_with("//") && !t.starts_with("/*") {
            // A function or class: its doc comment and attributes are not ours.
            cvalue = None;
            deprecated = None;
        }
    }
    if !conds.is_empty() {
        return Err("curl.stub.php: unterminated #if".into());
    }
    if out.is_empty() {
        return Err("curl.stub.php: no constants found".into());
    }
    Ok(out)
}

/// `since: '8.4', message: '...')]` → (since, message).
fn parse_deprecated(attr: &str) -> Option<(String, String)> {
    let quoted = |key: &str| -> Option<String> {
        let at = attr.find(key)? + key.len();
        let rest = attr[at..].trim_start();
        let q = rest.chars().next()?;
        let body = &rest[1..];
        Some(body[..body.find(q)?].to_string())
    };
    Some((quoted("since:")?, quoted("message:")?))
}

/// curl_private.h's `#define CURL... <number>` lines (option numbers php
/// gives its own options).
fn php_defines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| {
            let mut w = l.split_whitespace();
            w.next() == Some("#define")
                && w.next().is_some_and(|n| n.starts_with("CURL"))
                && w.next().is_some_and(|v| v.parse::<i64>().is_ok())
        })
        .map(|l| {
            let w: Vec<&str> = l.split_whitespace().take(3).collect();
            w.join(" ")
        })
        .collect()
}

/// A condition without its trailing comment.
fn bare(cond: &str) -> &str {
    cond.split("/*").next().unwrap_or(cond).trim()
}

fn gen_c(consts: &[Const], defines: &[String]) -> String {
    let mut o = String::new();
    o.push_str(
        "/* ext/curl's constants, evaluated against the curl.h rhp links.\n \
         *\n \
         * Generated by `rhp-difftest gen curl-consts` from php-src's\n \
         * ext/curl/curl.stub.php and curl_private.h. Do not edit. */\n\n\
         #include <curl/curl.h>\n\
         #include <curl/multi.h>\n\n\
         /* curl_private.h */\n",
    );
    for d in defines {
        let _ = writeln!(o, "{d}");
    }
    o.push_str(
        "\n#ifdef __GNUC__\n\
         # pragma GCC diagnostic ignored \"-Wdeprecated-declarations\"\n\
         #endif\n\n\
         struct rhp_curl_const {\n\
         \tlong long value;\n\
         \tint defined;\n\
         };\n\n\
         #define D(c) { (long long) (c), 1 }\n\
         #define U { 0, 0 }\n\n\
         /* consts.rs PHP, in the same order. */\n\
         const struct rhp_curl_const rhp_curl_php_consts[] = {\n",
    );
    let mut i = 0;
    while i < consts.len() {
        let conds = &consts[i].conds;
        let mut j = i;
        while j < consts.len() && consts[j].conds == *conds {
            j += 1;
        }
        let group = &consts[i..j];
        if conds.is_empty() {
            for c in group {
                let _ = writeln!(o, "\tD({}),", c.cvalue);
            }
        } else {
            let cond = if conds.len() == 1 {
                conds[0].clone()
            } else {
                conds
                    .iter()
                    .map(|c| format!("({})", bare(c)))
                    .collect::<Vec<_>>()
                    .join(" && ")
            };
            let _ = writeln!(o, "#if {cond}");
            for c in group {
                let _ = writeln!(o, "\tD({}),", c.cvalue);
            }
            o.push_str("#else\n");
            for _ in group {
                o.push_str("\tU,\n");
            }
            o.push_str("#endif\n");
        }
        i = j;
    }
    o.push_str(
        "};\n\n\
         const unsigned long rhp_curl_php_count =\n\
         \tsizeof(rhp_curl_php_consts) / sizeof(rhp_curl_php_consts[0]);\n\n\
         /* consts.rs INTERNAL, in the same order. */\n\
         const long long rhp_curl_internal_consts[] = {\n",
    );
    for n in INTERNAL {
        let _ = writeln!(o, "\t(long long) ({n}),");
    }
    o.push_str(
        "};\n\n\
         const unsigned long rhp_curl_internal_count =\n\
         \tsizeof(rhp_curl_internal_consts) / sizeof(rhp_curl_internal_consts[0]);\n",
    );
    o
}

fn gen_rs(consts: &[Const]) -> String {
    let mut o = String::new();
    o.push_str(
        "//! ext/curl's constant names; the values are in consts.c, evaluated\n\
         //! against the linked curl.h.\n\
         //!\n\
         //! Generated by `rhp-difftest gen curl-consts` from php-src's\n\
         //! ext/curl/curl.stub.php. Do not edit.\n\n\
         /// php's constants, in registration order: `rhp_curl_php_consts[i]` is\n\
         /// `PHP[i]`'s value, if its libcurl version gate passed.\n\
         pub(super) const PHP: &[&str] = &[\n",
    );
    for c in consts {
        let _ = writeln!(o, "    {:?},", c.name);
    }
    o.push_str(
        "];\n\n\
         /// `#[\\Deprecated]` constants: (name, since, message).\n\
         pub(super) const DEPRECATED: &[(&str, &str, &str)] = &[\n",
    );
    for c in consts {
        if let Some((since, msg)) = &c.deprecated {
            let _ = writeln!(o, "    ({:?}, {since:?}, {msg:?}),", c.name);
        }
    }
    o.push_str(
        "];\n\n\
         /// libcurl names the port uses that php does not export:\n\
         /// `rhp_curl_internal_consts[i]` is `INTERNAL[i]`'s value.\n\
         pub(super) const INTERNAL: &[&str] = &[\n",
    );
    for n in INTERNAL {
        let _ = writeln!(o, "    {n:?},");
    }
    o.push_str("];\n");
    o
}
