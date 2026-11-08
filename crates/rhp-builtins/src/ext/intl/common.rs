//! What the intl classes added after Collator/NumberFormatter/Locale
//! share: their PHP declarations, generated from php's stub signatures,
//! and the call a generated method or function makes into Rust.
//!
//! Each class is declared from its `signatures` rows: modifiers, parent,
//! interfaces, constants (the shim's table) and methods. A method's body
//! passes its name, `func_num_args()`, `$this` and every parameter to the
//! class's helper (`__translit('create', func_num_args(), null, $id,
//! $direction)`), by-reference parameters wrapped as `[&$x]`. A procedural
//! alias does the same with its own name behind `fn:`. The VM has already
//! applied the stub's parameter types, so the helper sees php's
//! zend_parse_parameters results; `func_num_args()` stands for
//! `ZEND_NUM_ARGS()` (optional parameters without a default).

use std::fmt::Write as _;
use std::rc::Rc;

use rhp_bytecode::signatures::{self as sigs, Param};
use rhp_values::{ArrayKey, ObjectHandle, Value};

use crate::zend::object_handlers;
use crate::zend::zpp::deref;
use crate::{Host, Thrown};

/// One generated class: its name, its helper, and the PHP of what the
/// stub signatures do not carry (declared properties).
pub(super) struct ClassSpec {
    pub(super) name: &'static str,
    pub(super) helper: &'static str,
    pub(super) props: &'static str,
}

/// The generated functions whose php body does not reset the global intl
/// error before zend_parse_parameters (most do, through
/// `INTL_METHOD_INIT_VARS`): measured with php, by the error left after a
/// call that zpp refuses. Every other one is generated with
/// `#[\\RhpBeforeZpp]`, so the VM calls [`super::before_zpp`] before it
/// checks the arguments.
const KEEPS_ERROR: &[&str] = &[
    "IntlBreakIterator::getIterator",
    "IntlDateFormatter::formatObject",
    "IntlTimeZone::getIDForWindowsID",
    "IntlTimeZone::getWindowsID",
    "MessageFormatter::formatMessage",
    "MessageFormatter::getErrorCode",
    "MessageFormatter::getErrorMessage",
    "MessageFormatter::parseMessage",
    "ResourceBundle::get",
    "ResourceBundle::getIterator",
    "UConverter::convert",
    "UConverter::fromUCallback",
    "UConverter::getAliases",
    "UConverter::getAvailable",
    "UConverter::getDestinationEncoding",
    "UConverter::getDestinationType",
    "UConverter::getErrorCode",
    "UConverter::getErrorMessage",
    "UConverter::getSourceEncoding",
    "UConverter::getSourceType",
    "UConverter::getStandards",
    "UConverter::getSubstChars",
    "UConverter::reasonText",
    "UConverter::setDestinationEncoding",
    "UConverter::setSourceEncoding",
    "UConverter::setSubstChars",
    "UConverter::toUCallback",
    "UConverter::transcode",
    "datefmt_format_object",
    "intltz_get_id_for_windows_id",
    "intltz_get_windows_id",
    "msgfmt_format_message",
    "msgfmt_get_error_code",
    "msgfmt_get_error_message",
    "msgfmt_parse_message",
    "resourcebundle_get",
];

/// The prelude markers for a generated function: `#[\\RhpBeforeZpp]`
/// (see [`KEEPS_ERROR`]), and `#[\\RhpZppCallable]` when it takes a
/// callable, which php's zpp checks with its deprecations (IntlChar's
/// enumerations).
fn markers(name: &str, ps: &[Param]) -> String {
    let mut m = String::new();
    if !KEEPS_ERROR.contains(&name) {
        m.push_str("#[\\RhpBeforeZpp] ");
    }
    if ps.iter().any(|p| {
        p.ty.split('|')
            .any(|t| t.trim_start_matches('?') == "callable")
    }) {
        m.push_str("#[\\RhpZppCallable] ");
    }
    m
}

/// The PHP declaration of a class.
pub(super) fn class_php(spec: &ClassSpec) -> String {
    let c = sigs::class(spec.name.as_bytes())
        .unwrap_or_else(|| panic!("no stub signature for class {}", spec.name));
    let mut o = String::new();
    if c.flags & sigs::CLASS_ABSTRACT != 0 {
        o.push_str("abstract ");
    }
    if c.flags & sigs::CLASS_FINAL != 0 {
        o.push_str("final ");
    }
    let kind = if c.kind == sigs::INTERFACE {
        "interface"
    } else {
        "class"
    };
    write!(o, "{kind} {}", c.name).unwrap();
    if !c.parent.is_empty() {
        write!(o, " extends {}", c.parent).unwrap();
    }
    if !c.interfaces.is_empty() {
        write!(o, " implements {}", c.interfaces.join(", ")).unwrap();
    }
    o.push_str("\n{\n");
    o.push_str(&super::class_const_decls(c.name));
    o.push_str(spec.props);
    // In the stub's order, as php lists them.
    let mut methods: Vec<_> = sigs::methods()
        .iter()
        .filter(|m| m.class == c.name)
        .collect();
    methods.sort_by_key(|m| m.order);
    for m in methods {
        let mut mods = String::new();
        if m.flags & sigs::ABSTRACT != 0 {
            mods.push_str("abstract ");
        }
        if m.flags & sigs::FINAL != 0 {
            mods.push_str("final ");
        }
        mods.push_str(if m.flags & sigs::PRIVATE != 0 {
            "private "
        } else if m.flags & sigs::PROTECTED != 0 {
            "protected "
        } else {
            "public "
        });
        let is_static = m.flags & sigs::STATIC != 0;
        if is_static {
            mods.push_str("static ");
        }
        let this = if is_static { "null" } else { "$this" };
        let body = body(spec.helper, &format!("'{}'", m.name), this, m.params, m.ret);
        writeln!(
            o,
            "    {}{mods}function {}({}){} {body}",
            markers(&format!("{}::{}", c.name, m.name), m.params),
            m.name,
            params(m.params, m.name),
            ret(m.ret, m.name)
        )
        .unwrap();
    }
    o.push_str("}\n");
    o
}

/// The PHP of the procedural functions `names`, each calling `helper` as
/// `fn:<name>`.
pub(super) fn functions_php(helper: &str, names: &[(&str, &str)]) -> String {
    let mut o = String::new();
    for (name, _) in names {
        let f = sigs::function(name.as_bytes())
            .unwrap_or_else(|| panic!("no stub signature for function {name}"));
        let mut body = body(helper, &format!("'fn:{name}'"), "null", f.params, f.ret);
        if let Some((since, message)) = sigs::deprecation(f) {
            let mut msg = format!("Function {}() is deprecated", f.name);
            if !since.is_empty() {
                write!(msg, " since {since}").unwrap();
            }
            if !message.is_empty() {
                write!(msg, ", {message}").unwrap();
            }
            let msg = msg.replace('\\', "\\\\").replace('\'', "\\'");
            body = format!("{{ __stdext_deprecated('{msg}'); {}", &body[2..]);
        }
        writeln!(
            o,
            "{}function {}({}){} {body}",
            markers(f.name, f.params),
            f.name,
            params(f.params, f.name),
            ret(f.ret, f.name)
        )
        .unwrap();
    }
    o
}

fn ret(ty: &str, name: &str) -> String {
    if ty.is_empty() || name == "__construct" {
        String::new()
    } else {
        format!(": {ty}")
    }
}

/// The functions whose php bodies count their arguments themselves
/// ("Too many arguments", or a deprecation before zpp's count check):
/// they take any number.
const COUNT_OWN_ARGS: &[&str] = &["intlgregcal_create_instance", "__construct", "set"];

fn params(ps: &[Param], name: &str) -> String {
    let mut out = Vec::new();
    for p in ps {
        let mut s = String::new();
        if name == "getPartsIterator" {
            // the stub says string; php's zpp takes an int
            s.push_str("int ");
        } else if !p.ty.is_empty() {
            s.push_str(zpp_type(p.ty));
            s.push(' ');
        }
        if p.by_ref() || p.prefer_ref() {
            s.push('&');
        }
        if p.variadic() {
            s.push_str("...");
        }
        s.push('$');
        s.push_str(p.name);
        if let Some(d) = p.default() {
            write!(s, " = {d}").unwrap();
        } else if p.flags & sigs::U != 0 {
            // UNKNOWN: optional with no default; func_num_args() tells.
            write!(s, " = {}", placeholder(p.ty)).unwrap();
        }
        out.push(s);
    }
    if COUNT_OWN_ARGS.contains(&name) && ps.len() == 6 {
        out.push("...$__more".into());
    }
    out.join(", ")
}

/// The type php's zpp checks where it is wider than the stub's
/// (`Z_PARAM_OBJ_OR_STR_OR_NULL` for a time zone).
fn zpp_type(ty: &str) -> &str {
    match ty {
        "IntlTimeZone|DateTimeZone|string|null" => "object|string|null",
        t => t,
    }
}

/// A default of the parameter's type for an UNKNOWN default.
fn placeholder(ty: &str) -> &'static str {
    if sigs::type_nullable(ty) {
        return "null";
    }
    match ty.split('|').next().unwrap_or("") {
        "int" => "0",
        "float" => "0.0",
        "string" => "''",
        "bool" => "false",
        "array" => "[]",
        "" | "mixed" => "null",
        other => panic!("no placeholder default for type {other}"),
    }
}

fn body(helper: &str, name: &str, this: &str, ps: &[Param], ret: &str) -> String {
    let mut args = vec![name.to_string(), "func_num_args()".into(), this.into()];
    let more = ps.len() == 6
        && COUNT_OWN_ARGS
            .iter()
            .any(|n| name.trim_matches('\'') == *n || name.trim_matches('\'') == format!("fn:{n}"));
    for p in ps {
        args.push(if p.variadic() {
            format!("...${}", p.name)
        } else if p.by_ref() || p.prefer_ref() {
            format!("[&${}]", p.name)
        } else {
            format!("${}", p.name)
        });
    }
    if more {
        args.push("...$__more".into());
    }
    let call = format!("{helper}({})", args.join(", "));
    if ret == "void" || ret.is_empty() {
        format!("{{ {call}; }}")
    } else {
        format!("{{ return {call}; }}")
    }
}

/// A helper call: who called (php's name for errors), how many arguments
/// php's zpp saw, and the arguments after the object.
pub(super) struct Call<'a> {
    /// `Class::method` or the function's name.
    pub(super) fname: String,
    /// The method's name.
    pub(super) name: &'a str,
    /// A procedural call of an instance method: its object was argument
    /// #1, so the others number from #2.
    pub(super) shifted: bool,
    /// The object (`$this` or a procedural call's first argument).
    pub(super) this: Option<Value>,
    pub(super) argc: usize,
    pub(super) args: &'a [Value],
}

impl<'a> Call<'a> {
    /// Unpacks a helper's arguments: `(method | "fn:<name>", argc, $this,
    /// ...args)`, with `aliases` naming each function's method.
    pub(super) fn new(
        class: &str,
        args: &'a [Value],
        aliases: &'static [(&'static str, &'static str)],
    ) -> Call<'a> {
        let what =
            String::from_utf8_lossy(&rhp_values::conv::to_string_bytes(&args[0])).into_owned();
        let argc = match args[1] {
            Value::Int(n) => n as usize,
            _ => 0,
        };
        let this = match deref(&args[2]) {
            Value::Null => None,
            v => Some(v),
        };
        let rest = &args[3..];
        if let Some(f) = what.strip_prefix("fn:") {
            let &(fname, name) = aliases
                .iter()
                .find(|(n, _)| *n == f)
                .unwrap_or_else(|| panic!("no alias for {f}"));
            let is_static = sigs::method(class.as_bytes(), name.as_bytes())
                .is_some_and(|m| m.flags & sigs::STATIC != 0);
            if !is_static {
                return Call {
                    fname: fname.to_string(),
                    name,
                    shifted: true,
                    this: rest.first().map(deref),
                    argc: argc.saturating_sub(1),
                    args: &rest[1.min(rest.len())..],
                };
            }
            return Call {
                fname: fname.to_string(),
                name,
                shifted: false,
                this: None,
                argc,
                args: rest,
            };
        }
        let name = sigs::method(class.as_bytes(), what.as_bytes())
            .map(|m| m.name)
            .unwrap_or_else(|| panic!("no stub signature for {class}::{what}"));
        Call {
            fname: format!("{class}::{name}"),
            name,
            shifted: false,
            this,
            argc,
            args: rest,
        }
    }

    /// Whether this runs as a method (`hasThis()`).
    pub(super) fn is_method(&self) -> bool {
        !self.shifted && self.this.is_some()
    }

    /// php's argument number of parameter `i` (after the object).
    pub(super) fn argno(&self, i: usize) -> usize {
        i + if self.shifted { 2 } else { 1 }
    }

    /// Parameter `i` when the caller passed it.
    pub(super) fn arg(&self, i: usize) -> Option<Value> {
        (i < self.argc)
            .then(|| self.args.get(i).map(deref))
            .flatten()
    }

    pub(super) fn long(&self, i: usize, default: i64) -> i64 {
        match self.arg(i) {
            Some(Value::Int(n)) => n,
            Some(Value::Bool(b)) => b as i64,
            Some(Value::Float(f)) => f as i64,
            _ => default,
        }
    }

    pub(super) fn opt_long(&self, i: usize) -> Option<i64> {
        match self.arg(i) {
            Some(Value::Int(n)) => Some(n),
            _ => None,
        }
    }

    pub(super) fn double(&self, i: usize, default: f64) -> f64 {
        match self.arg(i) {
            Some(Value::Float(f)) => f,
            Some(Value::Int(n)) => n as f64,
            _ => default,
        }
    }

    pub(super) fn bool(&self, i: usize, default: bool) -> bool {
        match self.arg(i) {
            Some(Value::Bool(b)) => b,
            Some(Value::Int(n)) => n != 0,
            _ => default,
        }
    }

    pub(super) fn bytes(&self, i: usize) -> Vec<u8> {
        match self.arg(i) {
            Some(Value::Str(s)) => s.as_bytes().to_vec(),
            _ => Vec::new(),
        }
    }

    pub(super) fn opt_bytes(&self, i: usize) -> Option<Vec<u8>> {
        match self.arg(i) {
            Some(Value::Str(s)) => Some(s.as_bytes().to_vec()),
            _ => None,
        }
    }

    /// A by-reference parameter's reference (`[&$x]`), when passed.
    pub(super) fn reference(&self, i: usize) -> Option<Value> {
        if i >= self.argc {
            return None;
        }
        match self.args.get(i)? {
            r @ Value::Ref(_) => Some(r.clone()),
            Value::Array(a) => match a.get(&ArrayKey::Int(0)) {
                Some(r @ Value::Ref(_)) => Some(r.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// `zend_argument_value_error(i, msg)`.
    pub(super) fn value_error(&self, i: usize, msg: &str) -> Thrown {
        let pname = self.param_name(i);
        Thrown::error(
            "ValueError",
            format!(
                "{}(): Argument #{} (${pname}) {msg}",
                self.fname,
                self.argno(i)
            ),
        )
    }

    /// `zend_argument_type_error(i, msg)`.
    pub(super) fn type_error(&self, i: usize, msg: &str) -> Thrown {
        let pname = self.param_name(i);
        Thrown::type_error(format!(
            "{}(): Argument #{} (${pname}) {msg}",
            self.fname,
            self.argno(i)
        ))
    }

    /// The stub's name of parameter `i` (after the object).
    pub(super) fn param_name(&self, i: usize) -> &'static str {
        let class = self.fname.split("::").next().unwrap_or("");
        let ps = if self.fname.contains("::") {
            sigs::method(class.as_bytes(), self.name.as_bytes()).map(|m| m.params)
        } else {
            sigs::function(self.fname.as_bytes()).map(|f| f.params)
        };
        let skip = usize::from(self.shifted);
        ps.and_then(|ps| ps.get(i + skip)).map_or("", |p| p.name)
    }
}

/// Stores into a by-reference parameter (`ZEND_TRY_ASSIGN_REF_*`).
pub(super) fn assign_ref(host: &mut dyn Host, r: &Value, v: Value) -> Result<(), Thrown> {
    if let Value::Ref(rc) = r {
        host.assign_ref(rc, v)?;
    }
    Ok(())
}

/// The native state of an object of an intl class, created on first use.
pub(super) fn state<T: Default + 'static>(h: &ObjectHandle) -> Rc<T> {
    if let Some(o) = object_handlers::payload_of::<T>(h) {
        return o;
    }
    let o = Rc::new(T::default());
    object_handlers::set_payload(h, o.clone());
    o
}

/// [`state`] of a value holding the object.
pub(super) fn state_of<T: Default + 'static>(v: &Value) -> Option<Rc<T>> {
    object_handlers::object_of(v).map(|h| state::<T>(&h))
}

/// The UTF-8 of an ICU string, or `None` (and the status) on failure.
pub(super) fn to_utf8(u: &[super::UChar], status: &mut super::UErrorCode) -> Option<Vec<u8>> {
    match super::utf16_to_utf8(u) {
        Ok(s) => Some(s),
        Err(e) => {
            *status = e;
            None
        }
    }
}

/// The UTF-16 of a PHP string, or `None` (and the status) on failure.
pub(super) fn to_utf16(s: &[u8], status: &mut super::UErrorCode) -> Option<Vec<super::UChar>> {
    match super::utf8_to_utf16(s) {
        Ok(u) => Some(u),
        Err(e) => {
            *status = e;
            None
        }
    }
}

/// `add_assoc_*` for a string key.
pub(super) fn put(a: &mut rhp_values::PhpArray, k: &str, v: Value) {
    a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
}
