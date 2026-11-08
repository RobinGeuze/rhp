//! `gen signatures`: builtin signatures from php-src's `*.stub.php`.
//!
//! php generates its arginfo from these stubs with build/gen_stub.php, and
//! Reflection reports that arginfo. This walks the stubs of the extensions
//! rhp provides with mago-syntax and writes one row per function, class and
//! method to crates/rhp-bytecode/src/signatures/table.rs:
//! - types in zend_type_to_string's canonical spelling;
//! - defaults printed the way gen_stub prints them (nikic/php-parser's
//!   pretty printer after name resolution), which is the string Reflection
//!   shows;
//! - by-ref, `@prefer-ref`, variadic, `@tentative-return-type`,
//!   `@frameless-function` arities, `#[\Deprecated]` (with a function's
//!   `since` and `message`), modifiers, parent and interfaces;
//! - the extension name php registers the item under;
//! - for a function parameter php's C body parses as any value, a resource
//!   or a stream (`zpp.rs`), that parse, so the VM's zpp pass can check
//!   past it.
//!
//! Declarations php only compiles on Windows or in debug builds are left out.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rhp_compiler::parse::cst::{
    Access, ArrayElement, Attribute, BinaryOperator, ClassLikeConstantSelector, ClassLikeMember,
    Expression, FunctionLikeParameterList, FunctionLikeReturnTypeHint, Hint, Identifier, Literal,
    LiteralStringKind, Method, Modifier, NamespaceBody, Sequence, Statement, UnaryPrefixOperator,
};

/// The stub directories of the extensions rhp provides, and the module name
/// php registers their functions and classes under (`zend_module_entry`'s
/// name; SAPI functions land in "standard", see php_module_startup).
const SOURCES: &[(&str, &str)] = &[
    ("Zend", "Core"),
    ("ext/bcmath", "bcmath"),
    ("ext/bz2", "bz2"),
    ("ext/calendar", "calendar"),
    ("ext/ctype", "ctype"),
    ("ext/curl", "curl"),
    ("ext/date", "date"),
    ("ext/dom", "dom"),
    ("ext/fileinfo", "fileinfo"),
    ("ext/filter", "filter"),
    ("ext/gd", "gd"),
    ("ext/gmp", "gmp"),
    ("ext/hash", "hash"),
    ("ext/iconv", "iconv"),
    ("ext/intl", "intl"),
    ("ext/json", "json"),
    ("ext/libxml", "libxml"),
    ("ext/mbstring", "mbstring"),
    ("ext/opcache", "Zend OPcache"),
    ("ext/openssl", "openssl"),
    ("ext/pcntl", "pcntl"),
    ("ext/pcre", "pcre"),
    ("ext/pdo", "PDO"),
    // Not sqlite_driver.stub.php: PDO_SQLite_Ext only carries the arginfo of
    // the deprecated driver methods php adds to PDO objects.
    ("ext/pdo_sqlite/pdo_sqlite.stub.php", "pdo_sqlite"),
    ("ext/phar", "Phar"),
    ("ext/posix", "posix"),
    ("ext/random", "random"),
    ("ext/readline", "readline"),
    ("ext/reflection", "Reflection"),
    ("ext/session", "session"),
    ("ext/simplexml", "SimpleXML"),
    ("ext/sockets", "sockets"),
    ("ext/sodium", "sodium"),
    ("ext/spl", "SPL"),
    ("ext/sqlite3", "sqlite3"),
    ("ext/standard", "standard"),
    ("ext/tokenizer", "tokenizer"),
    ("ext/xml", "xml"),
    ("ext/xmlreader", "xmlreader"),
    ("ext/xmlwriter", "xmlwriter"),
    ("ext/zip", "zip"),
    ("ext/zlib", "zlib"),
    ("sapi/cgi/cgi_main.stub.php", "standard"),
    ("sapi/cli/php_cli_process_title.stub.php", "standard"),
];

/// The generated file, relative to the workspace root.
const OUT: &str = "crates/rhp-bytecode/src/signatures/table.rs";

/// The php-src commit the table is generated from.
const PIN: &str = "tests/phpt/php-src.ref";

const USAGE: &str =
    "usage: rhp-difftest gen signatures [--php-src <dir>] [--out <file>] [--check]\n\
  --php-src   php-src checkout at tests/phpt/php-src.ref (default: php-src)\n\
  --out       output file (default: crates/rhp-bytecode/src/signatures/table.rs)\n\
  --check     fail if the output file differs instead of writing it\n";

pub fn main(args: &[String]) -> ExitCode {
    let mut php_src = PathBuf::from("php-src");
    let mut out = PathBuf::from(OUT);
    let mut check = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--php-src" => match it.next() {
                Some(v) => php_src = PathBuf::from(v),
                None => return usage(),
            },
            "--out" => match it.next() {
                Some(v) => out = PathBuf::from(v),
                None => return usage(),
            },
            "--check" => check = true,
            _ => return usage(),
        }
    }
    if let Err(e) = check_tag(&php_src) {
        eprintln!("gen signatures: {e}");
        return ExitCode::FAILURE;
    }
    match generate(&php_src) {
        Ok(text) => super::write_or_check(&out, &text, check, "signatures"),
        Err(e) => {
            eprintln!("gen signatures: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ExitCode {
    eprint!("{USAGE}");
    ExitCode::from(2)
}

/// A git checkout must be at the commit tests/phpt/php-src.ref pins.
fn check_tag(php_src: &Path) -> Result<(), String> {
    if !php_src.join("Zend").is_dir() {
        return Err(format!("{} is not a php-src checkout", php_src.display()));
    }
    // From the repo root (CI, the documented use), else from this crate's
    // source location.
    let pin = std::fs::read_to_string(PIN).or_else(|_| {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(PIN),
        )
    });
    let Ok(pin) = pin else {
        return Err(format!("{PIN} not found: run from the rhp checkout"));
    };
    let Some(pin) = pin
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
    else {
        return Err(format!("{PIN} names no commit"));
    };
    let head = std::process::Command::new("git")
        .arg("-C")
        .arg(php_src)
        .args(["rev-parse", "HEAD"])
        .output();
    match head {
        Ok(o) if o.status.success() => {
            let head = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if head != pin {
                return Err(format!(
                    "{} is at {head}, tests/phpt/php-src.ref pins {pin}",
                    php_src.display()
                ));
            }
            Ok(())
        }
        // Not a git checkout (an unpacked tarball): trust the caller.
        _ => Ok(()),
    }
}

#[derive(Debug, Clone)]
struct Param {
    name: String,
    ty: String,
    default: Option<String>,
    by_ref: bool,
    prefer_ref: bool,
    variadic: bool,
    /// `#[\SensitiveParameter]`.
    sensitive: bool,
    /// What php's C body parses it as, when the VM can check that.
    parse: Option<zpp::Parse>,
}

#[derive(Debug)]
struct Func {
    name: String,
    /// The C function (`@alias` / `@implementation-alias`, else the name).
    imp: String,
    ext: &'static str,
    ret: String,
    /// `#[\Deprecated]`'s `since` and `message` (empty when not given).
    deprecated: Option<(String, String)>,
    /// `F_FRAMELESS << arity` for each `@frameless-function`.
    frameless: u32,
    params: Vec<Param>,
}

#[derive(Debug)]
struct ClassRow {
    name: String,
    ext: &'static str,
    kind: u8,
    flags: u32,
    parent: String,
    interfaces: Vec<String>,
}

#[derive(Debug)]
struct MethodRow {
    class: String,
    name: String,
    flags: u32,
    ret: String,
    params: Vec<Param>,
}

#[derive(Default)]
struct Out {
    funcs: BTreeMap<String, Func>,
    classes: BTreeMap<String, ClassRow>,
    methods: BTreeMap<(String, String), MethodRow>,
}

// Flags, mirrored by crates/rhp-bytecode/src/signatures/mod.rs.
const M_PUBLIC: u32 = 1;
const M_PROTECTED: u32 = 2;
const M_PRIVATE: u32 = 4;
const M_STATIC: u32 = 16;
const M_FINAL: u32 = 32;
const M_ABSTRACT: u32 = 64;
const M_TENTATIVE: u32 = 1 << 8;
const M_DEPRECATED: u32 = 1 << 9;
const F_FRAMELESS: u32 = 1 << 10;
const C_FINAL: u32 = 32;
const C_ABSTRACT: u32 = 64;
const C_READONLY: u32 = 65536;
const C_NO_DYNAMIC_PROPERTIES: u32 = 1 << 13;
const C_ALLOW_DYNAMIC_PROPERTIES: u32 = 1 << 15;
const C_NOT_SERIALIZABLE: u32 = 1 << 29;
const K_CLASS: u8 = 0;
const K_INTERFACE: u8 = 1;
const K_TRAIT: u8 = 2;
const K_ENUM: u8 = 3;

fn stub_files(php_src: &Path) -> Result<Vec<(PathBuf, &'static str)>, String> {
    let mut files = Vec::new();
    for &(rel, ext) in SOURCES {
        let p = php_src.join(rel);
        if p.is_file() {
            files.push((p, ext));
            continue;
        }
        if !p.is_dir() {
            return Err(format!("missing {}", p.display()));
        }
        let mut found = Vec::new();
        walk(&p, &mut found);
        if found.is_empty() {
            return Err(format!("no *.stub.php under {}", p.display()));
        }
        found.sort();
        files.extend(found.into_iter().map(|f| (f, ext)));
    }
    Ok(files)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n != "tests") {
                walk(&p, out);
            }
        } else if p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".stub.php"))
        {
            out.push(p);
        }
    }
}

fn generate(php_src: &Path) -> Result<String, String> {
    let mut out = Out::default();
    for (path, ext) in stub_files(php_src)? {
        let src = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let rel = path
            .strip_prefix(php_src)
            .unwrap_or(&path)
            .display()
            .to_string();
        parse_stub(&rel, &src, ext, &mut out).map_err(|e| format!("{rel}: {e}"))?;
    }
    let mut dirs: Vec<&str> = SOURCES.iter().map(|&(rel, _)| rel).collect();
    dirs.push("main");
    let parses = zpp::scan(php_src, &dirs)?;
    for fr in out.funcs.values_mut() {
        apply_parses(fr, &parses);
    }
    Ok(render(&out))
}

/// Record the C body's parse of a function's parameters, when it has one
/// entry per stub parameter: any value on any parameter, a resource or a
/// stream on an untyped one.
fn apply_parses(fr: &mut Func, parses: &zpp::Parses) {
    let Some(ps) = parses.get(&fr.imp) else {
        return;
    };
    if ps.len() != fr.params.len() {
        return;
    }
    for (p, &parse) in fr.params.iter_mut().zip(ps) {
        p.parse = match parse {
            zpp::Parse::Zval => Some(parse),
            zpp::Parse::Resource(_) | zpp::Parse::Stream(_) if p.ty.is_empty() => Some(parse),
            _ => None,
        };
    }
}

/// Per-file parse state.
struct File<'s> {
    src: &'s [u8],
    /// (start, end) offsets of each doc comment.
    docs: Vec<(u32, u32)>,
    /// Per line: is the line compiled on a non-debug unix build?
    live: Vec<bool>,
    ext: &'static str,
}

impl File<'_> {
    fn text(&self, a: u32, b: u32) -> &str {
        std::str::from_utf8(&self.src[a as usize..b as usize]).unwrap_or("")
    }

    /// The doc comment directly before offset `start`.
    fn doc(&self, start: u32) -> &str {
        for &(a, b) in self.docs.iter().rev() {
            if b > start {
                continue;
            }
            let gap = &self.src[b as usize..start as usize];
            if gap.iter().all(u8::is_ascii_whitespace) {
                return self.text(a, b);
            }
            break;
        }
        ""
    }

    fn live_at(&self, offset: u32) -> bool {
        let line = self.src[..offset as usize]
            .iter()
            .filter(|&&b| b == b'\n')
            .count();
        self.live.get(line).copied().unwrap_or(true)
    }
}

/// php builds the stubs' `#if` blocks through the C preprocessor. Only the
/// blocks a non-debug unix build drops matter here.
fn live_lines(src: &[u8]) -> Vec<bool> {
    #[derive(Clone, Copy, PartialEq)]
    enum C {
        Yes,
        No,
        Unknown,
    }
    let mut stack: Vec<C> = Vec::new();
    let mut live = Vec::new();
    for line in src.split(|&b| b == b'\n') {
        let t = String::from_utf8_lossy(line);
        let t = t.trim();
        if let Some(rest) = t.strip_prefix('#') {
            let rest = rest.trim_start();
            let word: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            let arg = rest[word.len()..].trim();
            let windows = |s: &str| s.contains("PHP_WIN32");
            match word.as_str() {
                "ifdef" => stack.push(if windows(arg) { C::No } else { C::Unknown }),
                "ifndef" => stack.push(if windows(arg) { C::Yes } else { C::Unknown }),
                "if" => {
                    let a: String = arg.chars().filter(|c| !c.is_whitespace()).collect();
                    let a = a.trim_start_matches('(').trim_end_matches(')');
                    stack.push(match a {
                        "defined(PHP_WIN32" | "PHP_WIN32" => C::No,
                        "!defined(PHP_WIN32" => C::Yes,
                        _ if a.starts_with("ZEND_DEBUG") => C::No,
                        _ => C::Unknown,
                    })
                }
                "elif" => {
                    if let Some(c) = stack.last_mut() {
                        *c = C::Unknown;
                    }
                }
                "else" => {
                    if let Some(c) = stack.last_mut() {
                        *c = match *c {
                            C::Yes => C::No,
                            C::No => C::Yes,
                            C::Unknown => C::Unknown,
                        };
                    }
                }
                "endif" => {
                    stack.pop();
                }
                _ => {}
            }
        }
        live.push(!stack.contains(&C::No));
    }
    live
}

fn parse_stub(rel: &str, src: &[u8], ext: &'static str, out: &mut Out) -> Result<(), String> {
    let arena = rhp_compiler::parse::Arena::new();
    let parsed = arena.parse(rel.as_bytes(), src);
    if parsed.has_parse_errors() {
        return Err(format!("parse error: {:?}", parsed.parse_error()));
    }
    let prog = parsed.program();
    // rhp's parser keeps no trivia: the doc comments come from the tokens.
    let docs = rhp_compiler::parse::tokens(src)
        .into_iter()
        .filter(|t| t.kind == rhp_compiler::parse::TokenKind::DocBlockComment)
        .map(|t| (t.start as u32, t.end as u32))
        .collect();
    let file = File {
        src,
        docs,
        live: live_lines(src),
        ext,
    };
    walk_stmts(&file, &prog.statements, "", out)
}

fn walk_stmts(
    f: &File,
    stmts: &Sequence<Statement>,
    ns: &str,
    out: &mut Out,
) -> Result<(), String> {
    let mut ns = ns.to_string();
    for s in stmts.iter() {
        match s {
            Statement::Namespace(n) => {
                let name = n
                    .name
                    .as_ref()
                    .map(|i| String::from_utf8_lossy(ident_bytes(i)).into_owned())
                    .unwrap_or_default();
                match &n.body {
                    NamespaceBody::BraceDelimited(b) => walk_stmts(f, &b.statements, &name, out)?,
                    NamespaceBody::Implicit(b) => {
                        ns = name;
                        walk_stmts(f, &b.statements, &ns, out)?;
                    }
                }
            }
            Statement::Function(func) => {
                let start =
                    attrs_start(&func.attribute_lists).unwrap_or(func.function.span.start.offset);
                if !f.live_at(func.function.span.start.offset) {
                    continue;
                }
                let doc = f.doc(start);
                let name = qualify(&ns, &String::from_utf8_lossy(func.name.value));
                let params =
                    params(&func.parameter_list, doc, &ns).map_err(|e| format!("{name}: {e}"))?;
                // Namespaced functions are ZEND_NS_FUNCTIONs: no C name.
                let imp = doc_tags(doc)
                    .into_iter()
                    .find(|(t, _)| t == "alias" || t == "implementation-alias")
                    .map_or_else(|| name.clone(), |(_, v)| v);
                let row = Func {
                    name: name.clone(),
                    imp: if ns.is_empty() { imp } else { String::new() },
                    ext: f.ext,
                    ret: ret_type(&func.return_type_hint, &ns),
                    deprecated: deprecated_attr(&func.attribute_lists)
                        .map_err(|e| format!("{name}: {e}"))?,
                    frameless: frameless_arities(doc).map_err(|e| format!("{name}: {e}"))?,
                    params,
                };
                if out.funcs.insert(name.to_ascii_lowercase(), row).is_some() {
                    return Err(format!("function {name} declared twice"));
                }
            }
            Statement::Class(c) => {
                if !f.live_at(c.class.span.start.offset) {
                    continue;
                }
                let name = qualify(&ns, &String::from_utf8_lossy(c.name.value));
                let mut flags = 0;
                for m in c.modifiers.iter() {
                    match m {
                        Modifier::Final(_) => flags |= C_FINAL,
                        Modifier::Abstract(_) => flags |= C_ABSTRACT,
                        Modifier::Readonly(_) => flags |= C_READONLY,
                        _ => {}
                    }
                }
                // gen_stub's ClassInfo::getFlagsByPhpVersion.
                let start = attrs_start(&c.attribute_lists)
                    .or_else(|| c.modifiers.first().map(modifier_start))
                    .unwrap_or(c.class.span.start.offset);
                for (tag, _) in doc_tags(f.doc(start)) {
                    match tag.as_str() {
                        "strict-properties" => flags |= C_NO_DYNAMIC_PROPERTIES,
                        "not-serializable" => flags |= C_NOT_SERIALIZABLE,
                        _ => {}
                    }
                }
                if has_attr(&c.attribute_lists, "AllowDynamicProperties") {
                    flags |= C_ALLOW_DYNAMIC_PROPERTIES;
                }
                let parent = c
                    .extends
                    .as_ref()
                    .and_then(|e| e.types.iter().next())
                    .map(|i| resolve_class(i, &ns))
                    .unwrap_or_default();
                let ifaces = c
                    .implements
                    .as_ref()
                    .map(|i| i.types.iter().map(|t| resolve_class(t, &ns)).collect())
                    .unwrap_or_default();
                class(
                    f, out, &ns, name, K_CLASS, flags, parent, ifaces, &c.members,
                )?;
            }
            Statement::Interface(c) => {
                if !f.live_at(c.interface.span.start.offset) {
                    continue;
                }
                let name = qualify(&ns, &String::from_utf8_lossy(c.name.value));
                let ifaces = c
                    .extends
                    .as_ref()
                    .map(|i| i.types.iter().map(|t| resolve_class(t, &ns)).collect())
                    .unwrap_or_default();
                class(
                    f,
                    out,
                    &ns,
                    name,
                    K_INTERFACE,
                    0,
                    String::new(),
                    ifaces,
                    &c.members,
                )?;
            }
            Statement::Trait(c) => {
                if !f.live_at(c.r#trait.span.start.offset) {
                    continue;
                }
                let name = qualify(&ns, &String::from_utf8_lossy(c.name.value));
                class(
                    f,
                    out,
                    &ns,
                    name,
                    K_TRAIT,
                    0,
                    String::new(),
                    Vec::new(),
                    &c.members,
                )?;
            }
            Statement::Enum(c) => {
                if !f.live_at(c.r#enum.span.start.offset) {
                    continue;
                }
                let name = qualify(&ns, &String::from_utf8_lossy(c.name.value));
                let ifaces = c
                    .implements
                    .as_ref()
                    .map(|i| i.types.iter().map(|t| resolve_class(t, &ns)).collect())
                    .unwrap_or_default();
                // Internal enums are not final (zend_register_internal_enum).
                class(
                    f,
                    out,
                    &ns,
                    name,
                    K_ENUM,
                    0,
                    String::new(),
                    ifaces,
                    &c.members,
                )?;
            }
            _ => {}
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn class(
    f: &File,
    out: &mut Out,
    ns: &str,
    name: String,
    kind: u8,
    flags: u32,
    parent: String,
    interfaces: Vec<String>,
    members: &Sequence<ClassLikeMember>,
) -> Result<(), String> {
    for m in members.iter() {
        if let ClassLikeMember::Method(m) = m {
            if !f.live_at(m.function.span.start.offset) {
                continue;
            }
            let row = method(f, ns, &name, kind, m)?;
            let key = (name.to_ascii_lowercase(), row.name.to_ascii_lowercase());
            if out.methods.insert(key, row).is_some() {
                return Err(format!(
                    "method {name}::{} declared twice",
                    m.name.value.escape_ascii()
                ));
            }
        }
    }
    let row = ClassRow {
        name: name.clone(),
        ext: f.ext,
        kind,
        flags,
        parent,
        interfaces,
    };
    if out.classes.insert(name.to_ascii_lowercase(), row).is_some() {
        return Err(format!("class {name} declared twice"));
    }
    Ok(())
}

fn method(f: &File, ns: &str, class: &str, kind: u8, m: &Method) -> Result<MethodRow, String> {
    let start = attrs_start(&m.attribute_lists)
        .or_else(|| m.modifiers.first().map(modifier_start))
        .unwrap_or(m.function.span.start.offset);
    let doc = f.doc(start);
    let name = String::from_utf8_lossy(m.name.value).into_owned();
    let mut flags = 0;
    for md in m.modifiers.iter() {
        flags |= match md {
            Modifier::Public(_) => M_PUBLIC,
            Modifier::Protected(_) => M_PROTECTED,
            Modifier::Private(_) => M_PRIVATE,
            Modifier::Static(_) => M_STATIC,
            Modifier::Final(_) => M_FINAL,
            Modifier::Abstract(_) => M_ABSTRACT,
            _ => 0,
        };
    }
    if flags & (M_PUBLIC | M_PROTECTED | M_PRIVATE) == 0 {
        flags |= M_PUBLIC;
    }
    // gen_stub: a class method without a body (DateInterval::__serialize)
    // is still a concrete method; only interfaces' methods are implied
    // abstract.
    if kind == K_INTERFACE {
        flags |= M_ABSTRACT;
    }
    if doc_tags(doc)
        .iter()
        .any(|(t, _)| t == "tentative-return-type")
    {
        flags |= M_TENTATIVE;
    }
    if has_attr(&m.attribute_lists, "Deprecated") {
        flags |= M_DEPRECATED;
    }
    let params = params(&m.parameter_list, doc, ns).map_err(|e| format!("{class}::{name}: {e}"))?;
    Ok(MethodRow {
        class: class.to_string(),
        name,
        flags,
        ret: ret_type(&m.return_type_hint, ns),
        params,
    })
}

fn modifier_start(m: &Modifier) -> u32 {
    match m {
        Modifier::Static(k)
        | Modifier::Final(k)
        | Modifier::Abstract(k)
        | Modifier::Readonly(k)
        | Modifier::Public(k)
        | Modifier::PublicSet(k)
        | Modifier::Protected(k)
        | Modifier::ProtectedSet(k)
        | Modifier::Private(k)
        | Modifier::PrivateSet(k) => k.span.start.offset,
    }
}

fn attrs_start(lists: &Sequence<rhp_compiler::parse::cst::AttributeList>) -> Option<u32> {
    lists.first().map(|l| l.hash_left_bracket.start.offset)
}

fn has_attr(lists: &Sequence<rhp_compiler::parse::cst::AttributeList>, name: &str) -> bool {
    lists.iter().any(|l| {
        l.attributes.iter().any(|a: &Attribute| {
            let n = ident_bytes(&a.name);
            let n = n.strip_prefix(b"\\").unwrap_or(n);
            n.eq_ignore_ascii_case(name.as_bytes())
        })
    })
}

/// The `since` and `message` of a `#[\Deprecated]` attribute (literal
/// strings, named or in Deprecated::__construct's order), if it has one.
fn deprecated_attr(
    lists: &Sequence<rhp_compiler::parse::cst::AttributeList>,
) -> Result<Option<(String, String)>, String> {
    use rhp_compiler::parse::cst::PartialArgument;
    let Some(a) = lists.iter().flat_map(|l| l.attributes.iter()).find(|a| {
        let n = ident_bytes(&a.name);
        n.strip_prefix(b"\\")
            .unwrap_or(n)
            .eq_ignore_ascii_case(b"Deprecated")
    }) else {
        return Ok(None);
    };
    let mut vals = [String::new(), String::new()];
    let args = a.argument_list.iter().flat_map(|l| l.arguments.iter());
    for (pos, arg) in args.enumerate() {
        let (slot, value) = match arg {
            PartialArgument::Positional(p) if pos < 2 => (pos, p.value),
            PartialArgument::Named(n) if n.name.value == b"message" => (0, n.value),
            PartialArgument::Named(n) if n.name.value == b"since" => (1, n.value),
            _ => return Err("unexpected #[\\Deprecated] argument".into()),
        };
        let Expression::Literal(Literal::String(s)) = value else {
            return Err("#[\\Deprecated] argument is no string literal".into());
        };
        let v = s.value.ok_or("undecodable string literal")?;
        vals[slot] = String::from_utf8_lossy(v).into_owned();
    }
    let [message, since] = vals;
    Ok(Some((since, message)))
}

/// The `@frameless-function {"arity": N}` tags of a doc comment, as
/// `F_FRAMELESS << N` bits (zend's arities are 0 to 3).
fn frameless_arities(doc: &str) -> Result<u32, String> {
    let mut bits = 0;
    for (_, v) in doc_tags(doc)
        .iter()
        .filter(|(t, _)| t == "frameless-function")
    {
        let arity = v
            .split_once("\"arity\":")
            .and_then(|(_, n)| n.trim().trim_end_matches('}').trim().parse::<u32>().ok())
            .filter(|&n| n <= 3)
            .ok_or_else(|| format!("bad @frameless-function {v}"))?;
        bits |= F_FRAMELESS << arity;
    }
    Ok(bits)
}

/// `@tag value` pairs of a doc comment.
fn doc_tags(doc: &str) -> Vec<(String, String)> {
    let mut v = Vec::new();
    for line in doc.lines() {
        let l = line
            .trim()
            .trim_start_matches("/**")
            .trim_start_matches('*')
            .trim();
        let l = l.trim_end_matches("*/").trim();
        if let Some(rest) = l.strip_prefix('@') {
            let (tag, val) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            v.push((tag.to_string(), val.trim().to_string()));
        }
    }
    v
}

fn params(list: &FunctionLikeParameterList, doc: &str, ns: &str) -> Result<Vec<Param>, String> {
    let prefer: BTreeSet<String> = doc_tags(doc)
        .into_iter()
        .filter(|(t, _)| t == "prefer-ref")
        .filter_map(|(_, v)| {
            v.strip_prefix('$')
                .map(|v| v.split_whitespace().next().unwrap_or("").to_string())
        })
        .collect();
    let mut out = Vec::new();
    for p in list.parameters.iter() {
        let name = String::from_utf8_lossy(&p.variable.name[1..]).into_owned();
        let ty = p
            .hint
            .as_ref()
            .map(|h| type_string(h, ns))
            .unwrap_or_default();
        // `= UNKNOWN` (optional without a default value) is kept as "".
        let default = match &p.default_value {
            Some(d) => {
                let s = print_expr(d.value, ns).map_err(|e| format!("${name}: {e}"))?;
                Some(if s == "UNKNOWN" { String::new() } else { s })
            }
            None => None,
        };
        let prefer_ref = prefer.contains(&name);
        out.push(Param {
            name,
            ty,
            default,
            by_ref: p.ampersand.is_some() || prefer_ref,
            prefer_ref,
            variadic: p.ellipsis.is_some(),
            sensitive: p.attribute_lists.iter().any(|l| {
                l.attributes.iter().any(|a| {
                    let n = String::from_utf8_lossy(ident_bytes(&a.name)).into_owned();
                    n.trim_start_matches('\\') == "SensitiveParameter"
                })
            }),
            parse: None,
        });
    }
    Ok(out)
}

fn ret_type(h: &Option<FunctionLikeReturnTypeHint>, ns: &str) -> String {
    h.as_ref()
        .map(|h| type_string(&h.hint, ns))
        .unwrap_or_default()
}

fn ident_bytes<'a>(i: &Identifier<'a>) -> &'a [u8] {
    match i {
        Identifier::Local(l) => l.value,
        Identifier::Qualified(q) => q.value,
        Identifier::FullyQualified(q) => q.value,
    }
}

fn qualify(ns: &str, name: &str) -> String {
    if ns.is_empty() {
        name.to_string()
    } else {
        format!("{ns}\\{name}")
    }
}

/// php-parser's NameResolver for a class name.
fn resolve_class(i: &Identifier, ns: &str) -> String {
    let raw = String::from_utf8_lossy(ident_bytes(i)).into_owned();
    match i {
        Identifier::FullyQualified(_) => raw.trim_start_matches('\\').to_string(),
        _ => {
            let lc = raw.to_ascii_lowercase();
            if matches!(lc.as_str(), "self" | "static" | "parent") {
                raw
            } else {
                qualify(ns, &raw)
            }
        }
    }
}

/// php-parser's NameResolver for a constant name: unqualified names stay as
/// written (php falls back to the global constant at run time).
fn resolve_const(i: &Identifier, ns: &str) -> String {
    let raw = String::from_utf8_lossy(ident_bytes(i)).into_owned();
    match i {
        Identifier::FullyQualified(_) => raw.trim_start_matches('\\').to_string(),
        Identifier::Qualified(_) => qualify(ns, &raw),
        Identifier::Local(_) => raw,
    }
}

// Type masks, in zend_type_to_string's order.
const T_STATIC: u32 = 1;
const T_CALLABLE: u32 = 2;
const T_OBJECT: u32 = 4;
const T_ARRAY: u32 = 8;
const T_STRING: u32 = 16;
const T_INT: u32 = 32;
const T_FLOAT: u32 = 64;
const T_FALSE: u32 = 128;
const T_TRUE: u32 = 256;
const T_VOID: u32 = 512;
const T_NEVER: u32 = 1024;
const T_NULL: u32 = 2048;
const T_MIXED: u32 = 4096;

#[derive(Default)]
struct TypeAcc {
    /// Class names and `(A&B)` groups, in declaration order.
    classes: Vec<String>,
    mask: u32,
    intersection: bool,
}

fn collect_type(h: &Hint, ns: &str, acc: &mut TypeAcc) {
    match h {
        Hint::Identifier(i) => acc.classes.push(resolve_class(i, ns)),
        Hint::Parenthesized(p) => collect_type(p.hint, ns, acc),
        Hint::Nullable(n) => {
            acc.mask |= T_NULL;
            collect_type(n.hint, ns, acc);
        }
        Hint::Union(u) => {
            collect_type(u.left, ns, acc);
            collect_type(u.right, ns, acc);
        }
        Hint::Intersection(_) => {
            let mut parts = Vec::new();
            flatten_intersection(h, ns, &mut parts);
            acc.classes.push(parts.join("&"));
            acc.intersection = true;
        }
        Hint::Null(_) => acc.mask |= T_NULL,
        Hint::True(_) => acc.mask |= T_TRUE,
        Hint::False(_) => acc.mask |= T_FALSE,
        Hint::Array(_) => acc.mask |= T_ARRAY,
        Hint::Callable(_) => acc.mask |= T_CALLABLE,
        Hint::Static(_) => acc.mask |= T_STATIC,
        Hint::Self_(_) => acc.classes.push("self".into()),
        Hint::Parent(_) => acc.classes.push("parent".into()),
        Hint::Void(_) => acc.mask |= T_VOID,
        Hint::Never(_) => acc.mask |= T_NEVER,
        Hint::Float(_) => acc.mask |= T_FLOAT,
        Hint::Bool(_) => acc.mask |= T_FALSE | T_TRUE,
        Hint::Integer(_) => acc.mask |= T_INT,
        Hint::String(_) => acc.mask |= T_STRING,
        Hint::Object(_) => acc.mask |= T_OBJECT,
        Hint::Mixed(_) => acc.mask |= T_MIXED,
        // gen_stub: iterable is Traversable|array.
        Hint::Iterable(_) => {
            acc.classes.push("Traversable".into());
            acc.mask |= T_ARRAY;
        }
    }
}

fn flatten_intersection(h: &Hint, ns: &str, out: &mut Vec<String>) {
    match h {
        Hint::Intersection(i) => {
            flatten_intersection(i.left, ns, out);
            flatten_intersection(i.right, ns, out);
        }
        Hint::Parenthesized(p) => flatten_intersection(p.hint, ns, out),
        Hint::Identifier(i) => out.push(resolve_class(i, ns)),
        _ => out.push("?".into()),
    }
}

/// zend_type_to_string of the type gen_stub builds from a hint.
fn type_string(h: &Hint, ns: &str) -> String {
    let mut acc = TypeAcc::default();
    collect_type(h, ns, &mut acc);
    if acc.mask & T_MIXED != 0 {
        return "mixed".into();
    }
    let pure_intersection = acc.intersection && acc.classes.len() == 1 && acc.mask == 0;
    let mut parts: Vec<String> = acc
        .classes
        .iter()
        .map(|c| {
            if c.contains('&') && !pure_intersection {
                format!("({c})")
            } else {
                c.clone()
            }
        })
        .collect();
    let m = acc.mask;
    for (bit, n) in [
        (T_STATIC, "static"),
        (T_CALLABLE, "callable"),
        (T_OBJECT, "object"),
        (T_ARRAY, "array"),
        (T_STRING, "string"),
        (T_INT, "int"),
        (T_FLOAT, "float"),
    ] {
        if m & bit != 0 {
            parts.push(n.into());
        }
    }
    if m & (T_FALSE | T_TRUE) == T_FALSE | T_TRUE {
        parts.push("bool".into());
    } else if m & T_FALSE != 0 {
        parts.push("false".into());
    } else if m & T_TRUE != 0 {
        parts.push("true".into());
    }
    if m & T_VOID != 0 {
        parts.push("void".into());
    }
    if m & T_NEVER != 0 {
        parts.push("never".into());
    }
    if m & T_NULL != 0 {
        if parts.is_empty() {
            return "null".into();
        }
        if parts.len() == 1 && !parts[0].contains('&') {
            return format!("?{}", parts[0]);
        }
        parts.push("null".into());
    }
    parts.join("|")
}

// php-parser precedences (Standard printer), higher binds tighter.
fn bin_op(op: &BinaryOperator) -> Option<(&'static str, i32)> {
    Some(match op {
        BinaryOperator::Exponentiation(_) => ("**", 250),
        BinaryOperator::Multiplication(_) => ("*", 210),
        BinaryOperator::Division(_) => ("/", 210),
        BinaryOperator::Modulo(_) => ("%", 210),
        BinaryOperator::Addition(_) => ("+", 200),
        BinaryOperator::Subtraction(_) => ("-", 200),
        BinaryOperator::StringConcat(_) => (".", 185),
        BinaryOperator::LeftShift(_) => ("<<", 190),
        BinaryOperator::RightShift(_) => (">>", 190),
        BinaryOperator::BitwiseAnd(_) => ("&", 140),
        BinaryOperator::BitwiseXor(_) => ("^", 130),
        BinaryOperator::BitwiseOr(_) => ("|", 120),
        _ => return None,
    })
}

fn expr_prec(e: &Expression) -> i32 {
    match e {
        Expression::Binary(b) => bin_op(&b.operator).map_or(1000, |(_, p)| p),
        Expression::Parenthesized(p) => expr_prec(p.expression),
        Expression::UnaryPrefix(_) => 240,
        _ => 1000,
    }
}

/// gen_stub's default-value string: php-parser's Standard pretty printer.
fn print_expr(e: &Expression, ns: &str) -> Result<String, String> {
    Ok(match e {
        Expression::Parenthesized(p) => print_expr(p.expression, ns)?,
        Expression::Literal(l) => match l {
            Literal::String(s) => {
                let v = s.value.ok_or("undecodable string literal")?;
                match s.kind {
                    LiteralStringKind::SingleQuoted => single_quoted(v),
                    LiteralStringKind::DoubleQuoted => double_quoted(v),
                }
            }
            Literal::Integer(i) => print_int(i.raw, i.value.ok_or("integer overflow")?),
            Literal::Float(x) => print_float(x.value.into_inner()),
            Literal::True(k) | Literal::False(k) | Literal::Null(k) => {
                String::from_utf8_lossy(k.value).into_owned()
            }
        },
        Expression::ConstantAccess(c) => resolve_const(&c.name, ns),
        Expression::Identifier(i) => resolve_const(i, ns),
        Expression::Access(Access::ClassConstant(cc)) => {
            let class = match cc.class {
                Expression::Identifier(i) => resolve_class(i, ns),
                Expression::Self_(k) | Expression::Static(k) | Expression::Parent(k) => {
                    String::from_utf8_lossy(k.value).into_owned()
                }
                _ => return Err("unsupported class constant class".into()),
            };
            let name = match &cc.constant {
                ClassLikeConstantSelector::Identifier(i) => {
                    String::from_utf8_lossy(i.value).into_owned()
                }
                _ => return Err("unsupported class constant name".into()),
            };
            format!("{class}::{name}")
        }
        Expression::UnaryPrefix(u) => {
            let op = match u.operator {
                UnaryPrefixOperator::Negation(_) => "-",
                UnaryPrefixOperator::Plus(_) => "+",
                UnaryPrefixOperator::BitwiseNot(_) => "~",
                UnaryPrefixOperator::Not(_) => "!",
                _ => return Err("unsupported unary operator".into()),
            };
            let inner = print_expr(u.operand, ns)?;
            let nested = matches!(u.operand, Expression::UnaryPrefix(_)) && inner.starts_with(op);
            if expr_prec(u.operand) < 240 || nested {
                format!("{op}({inner})")
            } else {
                format!("{op}{inner}")
            }
        }
        Expression::Binary(b) => {
            let (op, prec) = bin_op(&b.operator).ok_or("unsupported binary operator")?;
            let mut l = print_expr(b.lhs, ns)?;
            let mut r = print_expr(b.rhs, ns)?;
            // Left-associative: parenthesize a looser lhs, or a rhs that is
            // not strictly tighter (** is right-associative).
            let right_assoc = op == "**";
            let (lp, rp) = (expr_prec(b.lhs), expr_prec(b.rhs));
            if lp < prec || (right_assoc && lp == prec) {
                l = format!("({l})");
            }
            if rp < prec || (!right_assoc && rp == prec) {
                r = format!("({r})");
            }
            format!("{l} {op} {r}")
        }
        Expression::Array(a) => format!("[{}]", print_elems(a.elements.iter(), ns)?),
        Expression::LegacyArray(a) => format!("array({})", print_elems(a.elements.iter(), ns)?),
        _ => return Err("unsupported default expression".into()),
    })
}

fn print_elems<'a, I>(elems: I, ns: &str) -> Result<String, String>
where
    I: Iterator<Item = &'a ArrayElement<'a>>,
{
    let mut v = Vec::new();
    for e in elems {
        v.push(match e {
            ArrayElement::KeyValue(kv) => format!(
                "{} => {}",
                print_expr(kv.key, ns)?,
                print_expr(kv.value, ns)?
            ),
            ArrayElement::Value(x) => print_expr(x.value, ns)?,
            ArrayElement::Variadic(x) => format!("...{}", print_expr(x.value, ns)?),
            ArrayElement::Missing(_) => String::new(),
        });
    }
    Ok(v.join(", "))
}

/// pSingleQuotedString: `'` and a `\` before `'`, `\` or the end are escaped.
fn single_quoted(v: &[u8]) -> String {
    let mut s = String::from("'");
    for (i, &b) in v.iter().enumerate() {
        let next = v.get(i + 1).copied();
        if b == b'\'' || (b == b'\\' && matches!(next, Some(b'\'') | Some(b'\\') | None)) {
            s.push('\\');
        }
        s.push(b as char);
    }
    s.push('\'');
    s
}

/// escapeString(value, '"'): addcslashes "\n\r\t\f\v$\"\\", then control
/// bytes as \xHH. (The stubs' strings are ASCII.)
fn double_quoted(v: &[u8]) -> String {
    let mut s = String::from("\"");
    for &b in v {
        match b {
            b'\n' => s.push_str("\\n"),
            b'\r' => s.push_str("\\r"),
            b'\t' => s.push_str("\\t"),
            0x0c => s.push_str("\\f"),
            0x0b => s.push_str("\\v"),
            b'$' => s.push_str("\\$"),
            b'"' => s.push_str("\\\""),
            b'\\' => s.push_str("\\\\"),
            0x00..=0x08 | 0x0e..=0x1f => s.push_str(&format!("\\x{b:02x}")),
            _ => s.push(b as char),
        }
    }
    s.push('"');
    s
}

/// pScalar_Int: the literal's base is kept, digits come from the value.
fn print_int(raw: &[u8], v: u64) -> String {
    let raw = String::from_utf8_lossy(raw).to_ascii_lowercase();
    if raw.starts_with("0x") {
        format!("0x{v:x}")
    } else if raw.starts_with("0b") {
        format!("0b{v:b}")
    } else if raw.len() > 1 && raw.starts_with('0') {
        format!("0{v:o}")
    } else {
        v.to_string()
    }
}

/// pScalar_Float: %.16G, or %.17G when that does not round-trip, with `.0`
/// added to an integral result.
fn print_float(v: f64) -> String {
    let mut s = fmt_g(v, 16);
    if s.parse::<f64>().ok() != Some(v) {
        s = fmt_g(v, 17);
    }
    if s.trim_start_matches('-')
        .bytes()
        .all(|b| b.is_ascii_digit())
    {
        s.push_str(".0");
    }
    s
}

/// C's `%.{p}G`.
fn fmt_g(v: f64, p: usize) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let e = format!("{:.*e}", p - 1, v);
    let (mant, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    if exp < -4 || exp >= p as i32 {
        let mant = strip_zeros(mant);
        format!("{mant}E{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs())
    } else {
        let decimals = (p as i32 - 1 - exp).max(0) as usize;
        strip_zeros(&format!("{v:.decimals$}"))
    }
}

fn strip_zeros(s: &str) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

/// A Rust string literal.
fn lit(s: &str) -> String {
    format!("{s:?}")
}

fn render_params(ps: &[Param]) -> String {
    let rows: Vec<String> = ps
        .iter()
        .map(|p| {
            let mut fl = Vec::new();
            if p.prefer_ref {
                fl.push("PR");
            } else if p.by_ref {
                fl.push("R");
            }
            if p.variadic {
                fl.push("V");
            }
            if p.default.as_deref() == Some("") {
                fl.push("U");
            }
            if p.sensitive {
                fl.push("SENS");
            }
            match p.parse {
                Some(zpp::Parse::Zval) => fl.push("ZV"),
                Some(zpp::Parse::Resource(n)) => {
                    fl.extend(["RS"].into_iter().chain(n.then_some("NUL")))
                }
                Some(zpp::Parse::Stream(n)) => {
                    fl.extend(["ST"].into_iter().chain(n.then_some("NUL")))
                }
                Some(zpp::Parse::Other) | None => {}
            }
            let fl = if fl.is_empty() {
                "0".to_string()
            } else {
                fl.join("|")
            };
            let d = match &p.default {
                Some(d) if !d.is_empty() => lit(d),
                _ => "\"\"".into(),
            };
            format!("p({}, {}, {d}, {fl})", lit(&p.name), lit(&p.ty))
        })
        .collect();
    format!("&[{}]", rows.join(", "))
}

fn render(out: &Out) -> String {
    let mut s = String::new();
    s.push_str(
        "// @generated by `rhp-difftest gen signatures` from php-src's *.stub.php at\n\
         // tests/phpt/php-src.ref. Do not edit; regenerate instead.\n\n\
         use super::{c, f, m, p, Class, Func, Method, NUL, PR, R, RS, SENS, ST, U, V, ZV};\n\n",
    );
    s.push_str(
        "/// Functions, sorted by lowercase name: name, extension, flags, return type, params.\n",
    );
    s.push_str("#[rustfmt::skip]\npub static FUNCTIONS: &[Func] = &[\n");
    for fr in out.funcs.values() {
        let flags = if fr.deprecated.is_some() {
            M_DEPRECATED
        } else {
            0
        } | fr.frameless;
        s.push_str(&format!(
            "f({}, {}, {flags}, {}, {}),\n",
            lit(&fr.name),
            lit(fr.ext),
            lit(&fr.ret),
            render_params(&fr.params)
        ));
    }
    s.push_str("];\n\n");
    s.push_str(
        "/// `#[\\Deprecated]` functions, sorted by lowercase name: name, since, message.\n",
    );
    s.push_str("#[rustfmt::skip]\npub static DEPRECATIONS: &[(&str, &str, &str)] = &[\n");
    for fr in out.funcs.values() {
        if let Some((since, message)) = &fr.deprecated {
            s.push_str(&format!(
                "({}, {}, {}),\n",
                lit(&fr.name),
                lit(since),
                lit(message)
            ));
        }
    }
    s.push_str("];\n\n");
    s.push_str("/// Classes, sorted by lowercase name: name, extension, kind, flags, parent, interfaces.\n");
    s.push_str("#[rustfmt::skip]\npub static CLASSES: &[Class] = &[\n");
    for c in out.classes.values() {
        let ifaces: Vec<String> = c.interfaces.iter().map(|i| lit(i)).collect();
        s.push_str(&format!(
            "c({}, {}, {}, {}, {}, &[{}]),\n",
            lit(&c.name),
            lit(c.ext),
            c.kind,
            c.flags,
            lit(&c.parent),
            ifaces.join(", ")
        ));
    }
    s.push_str("];\n\n");
    s.push_str(
        "/// Methods, sorted by lowercase class then method name: class, name, flags, return type, params.\n",
    );
    s.push_str("#[rustfmt::skip]\npub static METHODS: &[Method] = &[\n");
    for mr in out.methods.values() {
        s.push_str(&format!(
            "m({}, {}, {}, {}, {}),\n",
            lit(&mr.class),
            lit(&mr.name),
            mr.flags,
            lit(&mr.ret),
            render_params(&mr.params)
        ));
    }
    s.push_str("];\n");
    s
}

mod zpp;

#[cfg(test)]
mod tests;
