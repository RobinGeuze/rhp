//! The `intl` extension: grapheme-cluster string functions (`grapheme_*`),
//! Unicode normalization (`normalizer_*` / `Normalizer`), IDNA
//! (`idn_to_ascii` / `idn_to_utf8`), `Locale`, `NumberFormatter`,
//! `Collator`, `IntlException` and the error state (`intl_get_error_code` /
//! `intl_get_error_message`, the objects' own, `intl.use_exceptions`).
//!
//! Like php, rhp calls ICU4C (the build php links, see
//! docs/design/libraries.md); each function is a port of php-src's
//! ext/intl C. The constants are php's (`consts.h`, `class_consts.h`,
//! generated from the stubs) with the linked ICU's values. The classes are
//! declared in PHP (`prelude.php`); their methods run the procedural
//! functions' code through `__collator`, `__numfmt` and `__locale`, named
//! as php's method aliases are in errors. A Collator's or NumberFormatter's
//! ICU object is its payload.
//!
//! `extension_loaded('intl')` stays false: the rest of the extension
//! (IntlDateFormatter, MessageFormatter, IntlCalendar, …) is not
//! implemented, and callers that gate on the extension would otherwise
//! assume it. Its ini directives are in force (main/ini.rs).

use std::ffi::CStr;
use std::ptr;
use std::sync::OnceLock;

use rhp_values::Value;

use crate::{Host, Thrown};

mod breakiterator;
mod calendar;
mod collator;
mod common;
mod converter;
mod cpp;
mod dateformat;
mod dates;
mod error;
mod formatter;
mod grapheme;
mod idn;
mod iterator;
mod listformatter;
mod locale;
mod msgformat;
mod normalizer;
mod resourcebundle;
mod spoofchecker;
mod sys;
mod timezone;
mod transliterator;
mod uchar;

pub(crate) use error::ERROR_LEVEL_DEPRECATION;
pub(crate) use error::{error_reset, error_set_code, u_error_name};

use sys::{UChar, UErrorCode};

// ---------------------------------------------------------------------------
// constants (consts.h through shim.c)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub(crate) enum ConstValue {
    Int(i64),
    Str(&'static str),
    Float(f64),
}

/// The shim's table, read once.
fn const_table() -> &'static [(&'static str, ConstValue)] {
    static TABLE: OnceLock<Vec<(&'static str, ConstValue)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut out = Vec::new();
        // SAFETY: the shim returns a static array ended by a null name; its
        // strings are C literals.
        unsafe {
            let mut p = sys::rhp_intl_constants();
            while !(*p).name.is_null() {
                let name = CStr::from_ptr((*p).name).to_str().expect("ASCII name");
                let value = if (*p).str.is_null() {
                    ConstValue::Int((*p).value)
                } else {
                    ConstValue::Str(CStr::from_ptr((*p).str).to_str().expect("ASCII value"))
                };
                out.push((name, value));
                p = p.add(1);
            }
        }
        out
    })
}

/// The value of one of php's integer intl constants.
pub(crate) fn int_const(name: &str) -> i64 {
    match const_table().iter().find(|(n, _)| *n == name) {
        Some((_, ConstValue::Int(v))) => *v,
        _ => panic!("consts.h has no integer {name}"),
    }
}

/// One class constant of the shim's table (class_consts.h).
pub(crate) struct ClassConst {
    pub(crate) class: &'static str,
    pub(crate) name: &'static str,
    pub(crate) value: Option<ConstValue>,
    pub(crate) attr: &'static str,
}

fn class_const_table() -> &'static [ClassConst] {
    static TABLE: OnceLock<Vec<ClassConst>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut out = Vec::new();
        let s = |p: *const std::ffi::c_char| unsafe { CStr::from_ptr(p) }.to_str().expect("ASCII");
        // SAFETY: the shim's static array, ended by a null class; its
        // strings are C literals.
        unsafe {
            let mut p = sys::rhp_intl_class_constants();
            while !(*p).class.is_null() {
                let value = match (*p).kind {
                    0 => Some(ConstValue::Int((*p).value)),
                    1 => Some(ConstValue::Str(s((*p).str))),
                    3 => Some(ConstValue::Float((*p).dvalue)),
                    _ => None,
                };
                out.push(ClassConst {
                    class: s((*p).class),
                    name: s((*p).name),
                    value,
                    attr: s((*p).attr),
                });
                p = p.add(1);
            }
        }
        out
    })
}

/// The value of an integer class constant (`NumberFormatter::DECIMAL`).
pub(crate) fn class_int(class: &str, name: &str) -> i64 {
    match class_const_table()
        .iter()
        .find(|c| c.class == class && c.name == name)
        .and_then(|c| c.value)
    {
        Some(ConstValue::Int(v)) => v,
        _ => panic!("class_consts.h has no integer {class}::{name}"),
    }
}

/// The PHP declarations of `class`'s constants, for the prelude.
fn class_const_decls(class: &str) -> String {
    let mut out = String::new();
    for c in class_const_table().iter().filter(|c| c.class == class) {
        if !c.attr.is_empty() {
            out.push_str(&format!("    {}\n", c.attr));
        }
        let v = match c.value {
            Some(ConstValue::Int(i)) => format!("{i}"),
            Some(ConstValue::Str(s)) => format!("'{s}'"),
            Some(ConstValue::Float(f)) => format!("{f:?}"),
            None => "null".to_string(),
        };
        out.push_str(&format!("    public const {} = {v};\n", c.name));
    }
    out
}

/// The compiler's flags for prelude function `class::name` (or `name`):
/// those of an intl stub function or method, see [`common::flags`].
pub(crate) fn prelude_flags(class: Option<&[u8]>, name: &[u8]) -> rhp_bytecode::ProtoFlags {
    use rhp_bytecode::signatures as sigs;
    let none = rhp_bytecode::ProtoFlags::default();
    let (Ok(name), class) = (std::str::from_utf8(name), class.map(std::str::from_utf8)) else {
        return none;
    };
    match class {
        None => match sigs::function(name.as_bytes()) {
            Some(f) if f.ext == "intl" => common::flags(f.name, f.params),
            _ => none,
        },
        Some(Ok(c)) => match sigs::class(c.as_bytes()) {
            Some(cl) if cl.ext == "intl" => match sigs::method(c.as_bytes(), name.as_bytes()) {
                Some(m) => common::flags(&format!("{}::{}", m.class, m.name), m.params),
                None => none,
            },
            _ => none,
        },
        Some(Err(_)) => none,
    }
}

/// `intl_error_reset(NULL)` ahead of zend_parse_parameters, for the
/// prelude functions with ProtoFlags::BEFORE_ZPP.
pub(crate) fn before_zpp() {
    error::error_reset();
}

/// [`before_zpp`] for intl's native functions: all but these reset the
/// global error first (measured with php, as common.rs's KEEPS_ERROR).
pub(crate) fn before_failed_zpp(name: &str) {
    const KEEPS_ERROR: &[&str] = &[
        "intl_error_name",
        "intl_get_error_code",
        "intl_get_error_message",
        "intl_is_failure",
        "locale_accept_from_http",
        "locale_add_likely_subtags",
        "locale_get_default",
        "locale_is_right_to_left",
        "locale_minimize_subtags",
        "locale_set_default",
    ];
    if !name.starts_with("grapheme_") && !KEEPS_ERROR.contains(&name) {
        error::error_reset();
    }
}

/// The E_DEPRECATED a fetch of common.stub.php's one `#[\Deprecated]`
/// global constant raises.
pub(crate) fn constant_deprecation(name: &[u8]) -> Option<String> {
    (name == b"U_MULTIPLE_DECIMAL_SEPERATORS").then(|| {
        "Constant U_MULTIPLE_DECIMAL_SEPERATORS is deprecated since 8.3, use U_MULTIPLE_DECIMAL_SEPARATORS instead".into()
    })
}

/// The E_DEPRECATED a fetch of a `#[\Deprecated]` intl class constant
/// raises (NumberFormatter::TYPE_CURRENCY).
pub(crate) fn class_constant_deprecation(class: &[u8], name: &[u8]) -> Option<String> {
    // Every internal class constant fetch asks: scan only the deprecated
    // ones, collected once.
    static DEPRECATED: std::sync::OnceLock<Vec<(&'static str, &'static str, &'static str)>> =
        std::sync::OnceLock::new();
    let list = DEPRECATED.get_or_init(|| {
        class_const_table()
            .iter()
            .filter_map(|c| {
                let since = c
                    .attr
                    .strip_prefix("#[\\Deprecated(since: '")?
                    .strip_suffix("')]")?;
                Some((c.class, c.name, since))
            })
            .collect()
    });
    let (c, n, since) = list
        .iter()
        .find(|(c, n, _)| c.as_bytes().eq_ignore_ascii_case(class) && n.as_bytes() == name)?;
    Some(format!("Constant {c}::{n} is deprecated since {since}"))
}

pub fn constants() -> Vec<(&'static str, Value)> {
    const_table()
        .iter()
        .map(|&(n, v)| {
            let v = match v {
                ConstValue::Int(i) => Value::Int(i),
                ConstValue::Str(s) => Value::str(s),
                ConstValue::Float(f) => Value::Float(f),
            };
            (n, v)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// global error state (intl_error.c)
// ---------------------------------------------------------------------------

fn fn_intl_get_error_code(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(error::error_code() as i64))
}

fn fn_intl_get_error_message(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(error::error_message()))
}

fn fn_intl_is_failure(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = crate::zend::zpp::int_arg(host, args, 0, "intl_is_failure", "errorCode")?;
    Ok(Value::Bool(code > sys::U_ZERO_ERROR as i64))
}

fn fn_intl_error_name(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = crate::zend::zpp::int_arg(host, args, 0, "intl_error_name", "errorCode")?;
    // php passes the zend_long to u_errorName's UErrorCode (an int).
    Ok(Value::str(u_error_name(code as i32)))
}

// ---------------------------------------------------------------------------
// shared helpers (intl_convert.c)
// ---------------------------------------------------------------------------

/// `intl_convert_utf8_to_utf16`: ICU's strict UTF-8 → UTF-16.
pub(crate) fn utf8_to_utf16(src: &[u8]) -> Result<Vec<UChar>, UErrorCode> {
    if src.len() > i32::MAX as usize {
        return Err(sys::U_BUFFER_OVERFLOW_ERROR);
    }
    let mut status = sys::U_ZERO_ERROR;
    let mut len = 0i32;
    // SAFETY: a preflight (null buffer, zero capacity) over `src`.
    unsafe {
        sys::u_strFromUTF8(
            ptr::null_mut(),
            0,
            &mut len,
            src.as_ptr().cast(),
            src.len() as i32,
            &mut status,
        );
    }
    if status != sys::U_BUFFER_OVERFLOW_ERROR && status != sys::U_STRING_NOT_TERMINATED_WARNING {
        return Err(status);
    }
    let mut buf: Vec<UChar> = vec![0; len as usize + 1];
    status = sys::U_ZERO_ERROR;
    // SAFETY: `buf` holds `len + 1` units.
    unsafe {
        sys::u_strFromUTF8(
            buf.as_mut_ptr(),
            len + 1,
            ptr::null_mut(),
            src.as_ptr().cast(),
            src.len() as i32,
            &mut status,
        );
    }
    if sys::failure(status) {
        return Err(status);
    }
    buf.truncate(len as usize);
    Ok(buf)
}

/// `intl_convert_utf16_to_utf8`.
pub(crate) fn utf16_to_utf8(src: &[UChar]) -> Result<Vec<u8>, UErrorCode> {
    let mut status = sys::U_ZERO_ERROR;
    let mut len = 0i32;
    // SAFETY: a preflight over `src`.
    unsafe {
        sys::u_strToUTF8(
            ptr::null_mut(),
            0,
            &mut len,
            src.as_ptr(),
            src.len() as i32,
            &mut status,
        );
    }
    if status != sys::U_BUFFER_OVERFLOW_ERROR && status != sys::U_STRING_NOT_TERMINATED_WARNING {
        return Err(status);
    }
    let mut buf: Vec<u8> = vec![0; len as usize];
    status = sys::U_ZERO_ERROR;
    // SAFETY: `buf` holds `len` bytes.
    unsafe {
        sys::u_strToUTF8(
            buf.as_mut_ptr().cast(),
            len,
            ptr::null_mut(),
            src.as_ptr(),
            src.len() as i32,
            &mut status,
        );
    }
    if sys::failure(status) {
        return Err(status);
    }
    Ok(buf)
}

// ---------------------------------------------------------------------------
// locales (php_intl.c)
// ---------------------------------------------------------------------------

/// `intl_locale_get_default`: intl.default_locale, else ICU's default.
pub(crate) fn locale_get_default() -> Vec<u8> {
    match crate::ini_value(b"intl.default_locale") {
        Some(Some(v)) if !v.is_empty() => v,
        // SAFETY: ICU returns its default locale's static name.
        _ => unsafe { CStr::from_ptr(sys::uloc_getDefault()) }
            .to_bytes()
            .to_vec(),
    }
}

/// zpp `l` for a parameter after the object, php's argument number `n`
/// (a procedural call's object is #1). A null had its deprecation from
/// the VM already; absent is `default`.
pub(crate) fn long_param(
    host: &mut dyn Host,
    v: Option<&Value>,
    n: usize,
    fname: &str,
    pname: &str,
    default: i64,
) -> Result<i64, Thrown> {
    match v.map(crate::zend::zpp::deref) {
        None => Ok(default),
        Some(Value::Null) => Ok(0),
        Some(v) => crate::zend::zpp::long_value(host, &v, n, fname, pname, "int"),
    }
}

/// zpp `s` for a parameter after the object (see [`long_param`]): in
/// strict mode only a string (or the VM's null) passes.
pub(crate) fn str_param(
    host: &mut dyn Host,
    v: Option<&Value>,
    n: usize,
    fname: &str,
    pname: &str,
) -> Result<Vec<u8>, Thrown> {
    let v = v.map(crate::zend::zpp::deref).unwrap_or(Value::Null);
    let strict = host.caller_strict_types();
    let s = match &v {
        Value::Str(s) => return Ok(s.as_bytes().to_vec()),
        Value::Null => Some(Vec::new()),
        _ if strict => None,
        Value::Bool(_) | Value::Int(_) | Value::Float(_) => {
            Some(rhp_values::conv::to_string_bytes(&v))
        }
        Value::Object(_) => crate::zend::object_handlers::to_string(host, &v)?,
        _ => None,
    };
    s.ok_or_else(|| {
        Thrown::type_error(format!(
            "{fname}(): Argument #{n} (${pname}) must be of type string, {} given",
            crate::zend::zpp::zval_value_name(host, &v)
        ))
    })
}

/// zpp `d` for a parameter after the object (see [`long_param`]).
pub(crate) fn double_param(
    host: &mut dyn Host,
    v: Option<&Value>,
    n: usize,
    fname: &str,
    pname: &str,
) -> Result<f64, Thrown> {
    match v.map(crate::zend::zpp::deref) {
        None | Some(Value::Null) => Ok(0.0),
        Some(v) => crate::zend::zpp::double_value(host, &v, n, fname, pname),
    }
}

/// zpp `n` for a parameter after the object (see [`long_param`]).
pub(crate) fn number_param(
    host: &mut dyn Host,
    v: Option<&Value>,
    n: usize,
    fname: &str,
    pname: &str,
) -> Result<Value, Thrown> {
    match v.map(crate::zend::zpp::deref) {
        None | Some(Value::Null) => Ok(Value::Int(0)),
        Some(v) => crate::zend::zpp::number_value(host, &v, n, fname, pname),
    }
}

/// A C string for ICU: the bytes up to the first NUL, as C reads them.
pub(crate) fn c_string(s: &[u8]) -> std::ffi::CString {
    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    std::ffi::CString::new(&s[..end]).expect("no NUL")
}

/// The bytes of a C string ICU returned (empty for null).
pub(crate) fn c_bytes(p: *const std::ffi::c_char) -> Vec<u8> {
    if p.is_null() {
        return Vec::new();
    }
    // SAFETY: a NUL-terminated string from ICU.
    unsafe { CStr::from_ptr(p) }.to_bytes().to_vec()
}

/// `canonicalize_locale_string`.
pub(crate) fn canonicalize_locale_string(locale: &std::ffi::CStr) -> Option<std::ffi::CString> {
    let mut buf = [0 as std::ffi::c_char; sys::ULOC_FULLNAME_CAPACITY];
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a buffer of ULOC_FULLNAME_CAPACITY bytes.
    let len = unsafe {
        sys::uloc_canonicalize(
            locale.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as i32,
            &mut status,
        )
    };
    if sys::failure(status) || len <= 0 {
        return None;
    }
    // SAFETY: ICU terminated the string (it fit: no failure).
    Some(unsafe { CStr::from_ptr(buf.as_ptr()) }.to_owned())
}

/// `INTL_CHECK_LOCALE_LEN`: the global error for an over-long locale;
/// true when it is one.
pub(crate) fn locale_too_long(
    host: &mut dyn Host,
    fname: &str,
    len: usize,
) -> Result<bool, Thrown> {
    let max = sys::ULOC_FULLNAME_CAPACITY - 1;
    if len > max {
        error::set(
            host,
            fname,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            format!("Locale string too long, should be no longer than {max} characters"),
        )?;
        return Ok(true);
    }
    Ok(false)
}

/// Whether a locale of `len` bytes is too long (`INTL_CHECK_LOCALE_LEN`).
pub(crate) fn locale_too_long_len(len: usize) -> bool {
    len > sys::ULOC_FULLNAME_CAPACITY - 1
}

/// The UTF-16 units a fixed-size ICU getter writes, read with php's
/// grow-once pattern: `f(buf, cap, status)` returns the length.
pub(crate) fn icu_uchars(
    initial: usize,
    status: &mut UErrorCode,
    mut f: impl FnMut(*mut UChar, i32, &mut UErrorCode) -> i32,
) -> Vec<UChar> {
    let mut buf: Vec<UChar> = vec![0; initial];
    let mut len = f(buf.as_mut_ptr(), buf.len() as i32, status);
    if *status == sys::U_BUFFER_OVERFLOW_ERROR && len as usize >= initial {
        // one more unit, to avoid U_STRING_NOT_TERMINATED_WARNING
        let cap = len as usize + 1;
        *status = sys::U_ZERO_ERROR;
        buf = vec![0; cap];
        len = f(buf.as_mut_ptr(), cap as i32, status);
    }
    buf.truncate(len.clamp(0, buf.len() as i32) as usize);
    buf
}

/// php's `ZEND_HAS_OOM()`: an allocation crossed memory_limit, and the VM
/// raises the error once the builtin returns.
pub(crate) fn has_oom() -> bool {
    rhp_values::mem::state().attention()
}

/// `Z_PARAM_PATH`: a string that must not contain NUL bytes.
pub(crate) fn path_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<Vec<u8>, Thrown> {
    if args.len() <= i {
        return Ok(Vec::new());
    }
    let s = crate::zend::zpp::str_arg(host, args, i, fname, pname)?;
    if s.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{fname}(): Argument #{} (${pname}) must not contain any null bytes",
                i + 1
            ),
        ));
    }
    Ok(s)
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("intl_get_error_code", fn_intl_get_error_code),
            ("intl_get_error_message", fn_intl_get_error_message),
            ("intl_is_failure", fn_intl_is_failure),
            ("intl_error_name", fn_intl_error_name),
            ("grapheme_strlen", grapheme::fn_strlen),
            ("grapheme_strpos", grapheme::fn_strpos),
            ("grapheme_stripos", grapheme::fn_stripos),
            ("grapheme_strrpos", grapheme::fn_strrpos),
            ("grapheme_strripos", grapheme::fn_strripos),
            ("grapheme_substr", grapheme::fn_substr),
            ("grapheme_strstr", grapheme::fn_strstr),
            ("grapheme_stristr", grapheme::fn_stristr),
            ("grapheme_str_split", grapheme::fn_str_split),
            ("grapheme_extract", grapheme::fn_extract),
            ("grapheme_levenshtein", grapheme::fn_levenshtein),
            ("normalizer_normalize", normalizer::fn_normalize),
            ("normalizer_is_normalized", normalizer::fn_is_normalized),
            (
                "normalizer_get_raw_decomposition",
                normalizer::fn_get_raw_decomposition,
            ),
            ("idn_to_ascii", idn::fn_idn_to_ascii),
            ("idn_to_utf8", idn::fn_idn_to_utf8),
        ],
    );
    collator::register(m);
    formatter::register(m);
    locale::register(m);
    crate::register_helpers(
        m,
        &[
            ("__normalizer", normalizer::fn_method, 1, None),
            ("__collator", collator::fn_method, 2, None),
            ("__numfmt", formatter::fn_method, 2, None),
            ("__locale", locale::fn_method, 1, None),
            ("__resbundle", resourcebundle::fn_helper, 3, None),
            ("__translit", transliterator::fn_helper, 3, None),
            ("__intlchar", uchar::fn_helper, 3, None),
            ("__listfmt", listformatter::fn_helper, 3, None),
            ("__uconv", converter::fn_helper, 3, None),
            ("__spoof", spoofchecker::fn_helper, 3, None),
            ("__msgfmt", msgformat::fn_helper, 3, None),
            ("__datefmt", dateformat::fn_helper, 3, None),
            ("__datepatgen", dateformat::fn_dtpg_helper, 3, None),
            ("__intltz", timezone::fn_helper, 3, None),
            ("__intlcal", calendar::fn_helper, 3, None),
            ("__intlgregcal", calendar::fn_greg_helper, 3, None),
            ("__intliter", iterator::fn_helper, 3, None),
            ("__partsiter", iterator::fn_parts_helper, 3, None),
            ("__breakiter", breakiterator::fn_helper, 3, None),
            ("__rbbiter", breakiterator::fn_rbbi_helper, 3, None),
            ("__cpbiter", breakiterator::fn_cpbi_helper, 3, None),
        ],
    );
}

/// The object handlers of the intl classes with native state.
pub(crate) static CLASSES: &[crate::zend::object_handlers::ClassHandlers] = &[
    crate::zend::object_handlers::ClassHandlers {
        class: "Collator",
        handlers: &collator::HANDLERS,
        create_object: true,
    },
    crate::zend::object_handlers::ClassHandlers {
        class: "NumberFormatter",
        handlers: &formatter::HANDLERS,
        create_object: true,
    },
    crate::zend::object_handlers::ClassHandlers {
        class: "IntlListFormatter",
        handlers: &listformatter::HANDLERS,
        create_object: true,
    },
    crate::zend::object_handlers::ClassHandlers {
        class: "MessageFormatter",
        handlers: &msgformat::HANDLERS,
        create_object: true,
    },
    crate::zend::object_handlers::ClassHandlers {
        class: "IntlDateFormatter",
        handlers: &dateformat::HANDLERS,
        create_object: true,
    },
    crate::zend::object_handlers::ClassHandlers {
        class: "IntlDatePatternGenerator",
        handlers: &dateformat::DTPG_HANDLERS,
        create_object: true,
    },
    crate::zend::object_handlers::ClassHandlers {
        class: "ResourceBundle",
        handlers: &resourcebundle::HANDLERS,
        create_object: true,
    },
    crate::zend::object_handlers::ClassHandlers {
        class: "IntlTimeZone",
        handlers: &timezone::HANDLERS,
        create_object: true,
    },
    crate::zend::object_handlers::ClassHandlers {
        class: "IntlCalendar",
        handlers: &calendar::HANDLERS,
        create_object: true,
    },
    crate::zend::object_handlers::ClassHandlers {
        class: "IntlIterator",
        handlers: &iterator::HANDLERS,
        create_object: true,
    },
    crate::zend::object_handlers::ClassHandlers {
        class: "IntlBreakIterator",
        handlers: &breakiterator::HANDLERS,
        create_object: true,
    },
    crate::zend::object_handlers::ClassHandlers {
        class: "UConverter",
        handlers: &converter::HANDLERS,
        create_object: true,
    },
    crate::zend::object_handlers::ClassHandlers {
        class: "Transliterator",
        handlers: &transliterator::HANDLERS,
        create_object: true,
    },
    crate::zend::object_handlers::ClassHandlers {
        class: "Spoofchecker",
        handlers: &spoofchecker::HANDLERS,
        create_object: true,
    },
];

/// The generated classes before IntlException, in php_intl.c's MINIT
/// order, with their procedural aliases.
const CLASSES_BEFORE: &[(&common::ClassSpec, &[(&str, &str)])] = &[
    (&msgformat::SPEC, msgformat::ALIASES),
    (&dateformat::SPEC, dateformat::ALIASES),
    (&dateformat::DTPG_SPEC, &[]),
    (&resourcebundle::SPEC, resourcebundle::ALIASES),
    (&transliterator::SPEC, transliterator::ALIASES),
    (&timezone::SPEC, timezone::ALIASES),
    (&calendar::SPEC, calendar::ALIASES),
    (&calendar::GREG_SPEC, calendar::GREG_ALIASES),
    (&spoofchecker::SPEC, &[]),
];

/// The generated classes after IntlException.
const CLASSES_AFTER: &[(&common::ClassSpec, &[(&str, &str)])] = &[
    (&iterator::SPEC, &[]),
    (&breakiterator::SPEC, &[]),
    (&breakiterator::RBBI_SPEC, &[]),
    (&breakiterator::CPBI_SPEC, &[]),
    (&iterator::PARTS_SPEC, &[]),
    (&converter::SPEC, &[]),
    (&uchar::SPEC, &[]),
];

unsafe extern "C" {
    fn rhp_intl_info_version(which: i32) -> *const std::ffi::c_char;
}
sys::icu! {
    pub fn ucal_getTZDataVersion(status: *mut UErrorCode) -> *const std::ffi::c_char;
}

/// `PHP_MINFO_FUNCTION(intl)`'s rows (its ini entries follow).
pub(crate) fn info_rows() -> Vec<(&'static str, String)> {
    let v = |which| {
        // SAFETY: a static string or NULL.
        let p = unsafe { rhp_intl_info_version(which) };
        (!p.is_null()).then(|| String::from_utf8_lossy(&c_bytes(p)).into_owned())
    };
    let mut rows = vec![("Internationalization support", "enabled".to_string())];
    rows.push(("ICU version", v(0).unwrap_or_default()));
    if let Some(d) = v(1) {
        rows.push(("ICU Data version", d));
    }
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: ICU's static version string.
    let tz = unsafe { ucal_getTZDataVersion(&mut status) };
    if status == sys::U_ZERO_ERROR {
        rows.push((
            "ICU TZData version",
            String::from_utf8_lossy(&c_bytes(tz)).into_owned(),
        ));
    }
    rows.push(("ICU Unicode version", v(2).unwrap_or_default()));
    rows
}

/// The intl classes, in php_intl.c's MINIT order: their constants from
/// the linked ICU, their methods calling the procedural functions' code
/// (which names the method in errors).
pub fn prelude() -> &'static str {
    static PRELUDE: OnceLock<String> = OnceLock::new();
    PRELUDE.get_or_init(|| {
        let mut p = include_str!("prelude.php").to_string();
        for class in ["Collator", "NumberFormatter", "Locale"] {
            p = p.replace(
                &format!("    // %CONSTANTS {class}%\n"),
                &class_const_decls(class),
            );
        }
        for (marker, list) in [
            (
                "// %CLASSES AFTER NumberFormatter%\n",
                &[(&listformatter::SPEC, &[][..])][..],
            ),
            ("// %CLASSES BEFORE IntlException%\n", CLASSES_BEFORE),
            ("// %CLASSES AFTER IntlException%", CLASSES_AFTER),
        ] {
            let mut gen = String::new();
            for (spec, aliases) in list {
                gen.push_str(&common::class_php(spec));
                gen.push_str(&common::functions_php(spec.helper, aliases));
            }
            p = p.replace(marker, &gen);
        }
        p
    })
}

#[cfg(test)]
mod tests;
