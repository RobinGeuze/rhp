//! `gen parser --php-dist <dir> [--check]`: the token ids and the Bison
//! tables of php's language and INI parsers, from the php release tarball
//! (`tools/php-dist.sha256` pins it). Only the tarball has Bison's output
//! (Zend/zend_language_parser.{c,h}, Zend/zend_ini_parser.c); the git tag
//! has the grammars alone.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use super::util::{finish, flag, read, repo_root};

const IDS: &str = "crates/rhp-builtins/src/ext/tokenizer/ids.rs";
const LANG: &str = "crates/rhp-builtins/src/zend/language_parser_tables.rs";
const INI: &str = "crates/rhp-builtins/src/zend/ini_parser_tables.rs";

/// The language parser's rules whose actions rhp runs (zend/language_parser.rs):
/// a name for each, and the rule as Bison prints it in yyparse's reduce switch.
const LANG_RULES: &[(&str, &str)] = &[
    (
        "TOP_HALT_COMPILER",
        "top_statement: \"'__halt_compiler'\" '(' ')' ';'",
    ),
    (
        "INNER_HALT_COMPILER",
        "inner_statement: \"'__halt_compiler'\" '(' ')' ';'",
    ),
    ("IDENTIFIER_SEMI_RESERVED", "identifier: semi_reserved"),
    (
        "TRAIT_ALIAS_RESERVED",
        "trait_alias: trait_method_reference \"'as'\" reserved_non_modifiers",
    ),
    (
        "CLASS_CONST_DECL_SEMI_RESERVED",
        "class_const_decl: semi_reserved '=' expr backup_doc_comment",
    ),
    ("CLASS_MODIFIERS_FIRST", "class_modifiers: class_modifier"),
    (
        "CLASS_MODIFIERS_ADD",
        "class_modifiers: class_modifiers class_modifier",
    ),
    (
        "ANON_CLASS_MODIFIERS_FIRST",
        "anonymous_class_modifiers: class_modifier",
    ),
    (
        "ANON_CLASS_MODIFIERS_ADD",
        "anonymous_class_modifiers: anonymous_class_modifiers class_modifier",
    ),
    (
        "MEMBER_MODIFIERS_FIRST",
        "non_empty_member_modifiers: member_modifier",
    ),
    (
        "MEMBER_MODIFIERS_ADD",
        "non_empty_member_modifiers: non_empty_member_modifiers member_modifier",
    ),
    (
        "CPP_MODIFIERS",
        "optional_cpp_modifiers: non_empty_member_modifiers",
    ),
    (
        "PROPERTY_MODIFIERS",
        "property_modifiers: non_empty_member_modifiers",
    ),
    (
        "METHOD_MODIFIERS",
        "method_modifiers: non_empty_member_modifiers",
    ),
    (
        "CLASS_CONST_MODIFIERS",
        "class_const_modifiers: non_empty_member_modifiers",
    ),
    (
        "PROPERTY_HOOK_MODIFIERS",
        "property_hook_modifiers: non_empty_member_modifiers",
    ),
    (
        "TRAIT_ALIAS_MODIFIER_IDENTIFIER",
        "trait_alias: trait_method_reference \"'as'\" member_modifier identifier",
    ),
    (
        "TRAIT_ALIAS_MODIFIER",
        "trait_alias: trait_method_reference \"'as'\" member_modifier",
    ),
];

/// Mid-rule actions (Bison numbers them `$@N`), found by the call they make.
const LANG_ACTIONS: &[(&str, &str)] = &[("DECLARE_ENCODING", "zend_handle_encoding_declaration(")];

pub fn main(args: &[String]) -> ExitCode {
    let check = args.iter().any(|a| a == "--check");
    finish("parser", check, files(args))
}

fn files(args: &[String]) -> Result<Vec<(&'static str, String)>, String> {
    let dist =
        flag(args, "--php-dist").ok_or("gen parser needs --php-dist <extracted php-x.y.z>")?;
    let dist = Path::new(dist);
    check_version(dist)?;
    let zend = dist.join("Zend");
    let header = read(&zend.join("zend_language_parser.h"))?;
    let grammar = read(&zend.join("zend_language_parser.y"))?;
    let lang = read(&zend.join("zend_language_parser.c"))?;
    let ini = read(&zend.join("zend_ini_parser.c"))?;
    let ini_h = read(&zend.join("zend_ini_parser.h"))?;
    let kinds = token_kinds(&ini_h, "ini_tokentype")?;
    Ok(vec![
        (IDS, token_ids(&header, &grammar)?),
        (
            LANG,
            tables(
                &lang,
                "Zend/zend_language_parser.c",
                None,
                LANG_RULES,
                LANG_ACTIONS,
            )?,
        ),
        (
            INI,
            tables(&ini, "Zend/zend_ini_parser.c", Some(&kinds), &[], &[])?,
        ),
    ])
}

/// The tarball must be the release rhp reports and tools/php-dist.sha256 pins.
fn check_version(dist: &Path) -> Result<(), String> {
    let h = read(&dist.join("main/php_version.h"))?;
    let v = h
        .lines()
        .find_map(|l| l.strip_prefix("#define PHP_VERSION \""))
        .and_then(|l| l.strip_suffix('"'))
        .ok_or("main/php_version.h has no PHP_VERSION")?;
    let want = rhp_builtins::version::PHP_VERSION;
    if v != want {
        return Err(format!("the tarball is php {v}, rhp reports {want}"));
    }
    let pin = read(&repo_root().join("tools/php-dist.sha256"))?;
    let name = format!("php-{want}.tar.xz");
    if !pin
        .lines()
        .any(|l| l.split_whitespace().nth(1) == Some(name.as_str()))
    {
        return Err(format!("tools/php-dist.sha256 does not pin {name}"));
    }
    Ok(())
}

/// ids.rs: values from the Bison header, names and order from the grammar's
/// `%token` lines, read the way ext/tokenizer/tokenizer_data_gen.php reads
/// them (the last `T_` word of each line; not T_NOELSE or T_ERROR; plus
/// T_DOUBLE_COLON for T_PAAMAYIM_NEKUDOTAYIM).
fn token_ids(header: &str, grammar: &str) -> Result<String, String> {
    let value: BTreeMap<String, i64> = token_kinds(header, "zendtokentype")?.into_iter().collect();
    let mut names = Vec::new();
    for line in grammar.lines() {
        if !line.starts_with("%token") {
            continue;
        }
        let words: Vec<&str> = line
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|w| w.starts_with("T_"))
            .collect();
        if let Some(&w) = words.last() {
            if w != "T_NOELSE" && w != "T_ERROR" {
                names.push(w.to_string());
            }
        }
    }
    let mut all = Vec::new();
    for n in &names {
        let v = *value
            .get(n)
            .ok_or(format!("{n} is not in zend_language_parser.h"))?;
        all.push((n.clone(), v));
    }
    let paamayim = *value
        .get("T_PAAMAYIM_NEKUDOTAYIM")
        .ok_or("no T_PAAMAYIM_NEKUDOTAYIM")?;
    all.push(("T_DOUBLE_COLON".to_string(), paamayim));

    let mut o = String::new();
    o.push_str(
        "//! php's token ids (the `T_*` constants of ext/tokenizer) and `token_name()`.\n\
         //!\n\
         //! Generated by `rhp-difftest gen parser` from the php release tarball: the\n\
         //! values from Bison's Zend/zend_language_parser.h, the names and their order\n\
         //! from the grammar's `%token` lines, as ext/tokenizer/tokenizer_data_gen.php\n\
         //! reads them. Do not edit.\n\
         #![allow(dead_code)]\n\n",
    );
    for (n, v) in &all {
        writeln!(o, "pub const {n}: i64 = {v};").unwrap();
    }
    o.push_str("\n/// `(name, id)` for every T_* constant (both names for 402).\n");
    o.push_str("pub const ALL: &[(&str, i64)] = &[\n");
    for (n, v) in &all {
        writeln!(o, "    (\"{n}\", {v}),").unwrap();
    }
    o.push_str("];\n\n");
    o.push_str("/// `token_name()`: php reports 402 as T_DOUBLE_COLON.\n");
    o.push_str("pub fn token_name(id: i64) -> &'static str {\n    match id {\n");
    for (n, v) in all.iter().take(all.len() - 1) {
        let n = if n == "T_PAAMAYIM_NEKUDOTAYIM" {
            "T_DOUBLE_COLON"
        } else {
            n
        };
        writeln!(o, "        {v} => \"{n}\",").unwrap();
    }
    o.push_str("        _ => \"UNKNOWN\",\n    }\n}\n");
    Ok(o)
}

/// The entries of a Bison header's token-kind enum, in order.
fn token_kinds(header: &str, enum_name: &str) -> Result<Vec<(String, i64)>, String> {
    let body = header
        .split(&format!("enum {enum_name}"))
        .nth(1)
        .and_then(|s| s.split("};").next())
        .ok_or(format!("no enum {enum_name}"))?;
    let mut out = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        let Some((name, rest)) = line.split_once(" = ") else {
            continue;
        };
        let num: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        let num: i64 = num.parse().map_err(|_| format!("bad enum line {line:?}"))?;
        out.push((name.trim().to_string(), num));
    }
    Ok(out)
}

/// The tables Bison's yacc.c skeleton drives, as a `bison::Tables`, the
/// token kinds (above `error` and `invalid token`) when given, and the
/// numbers of `rules` and `actions`.
fn tables(
    c: &str,
    what: &str,
    kinds: Option<&[(String, i64)]>,
    rules: &[(&str, &str)],
    actions: &[(&str, &str)],
) -> Result<String, String> {
    if !c.contains("#define YYBISON_VERSION") || !c.contains("#define YYSKELETON_NAME \"yacc.c\"") {
        return Err(format!("{what}: not a Bison yacc.c parser"));
    }
    let version = define_str(c, "YYBISON_VERSION")?;
    let mut o = String::new();
    writeln!(
        o,
        "//! The tables of {what}, generated by Bison {version}.\n\
         //!\n\
         //! Copied by `rhp-difftest gen parser` from the php release tarball. Do not\n\
         //! edit. Bison's output is covered by its skeleton exception (see\n\
         //! THIRD-PARTY-NOTICES).\n"
    )
    .unwrap();
    o.push_str("use super::bison::Tables;\n\n");
    if let Some(kinds) = kinds {
        for (n, v) in kinds.iter().filter(|(_, v)| *v > 257) {
            writeln!(o, "pub const {n}: i32 = {v};").unwrap();
        }
        o.push('\n');
    }
    if !rules.is_empty() {
        o.push_str(
            "/// The rules whose actions rhp runs, by their number in yyparse's\n\
             /// reduce switch.\n\
             pub mod rule {\n",
        );
        for (name, text) in rules {
            writeln!(
                o,
                "    pub const {name}: usize = {};",
                rule_number(c, text)?
            )
            .unwrap();
        }
        for (name, call) in actions {
            writeln!(
                o,
                "    pub const {name}: usize = {};",
                action_rule(c, call)?
            )
            .unwrap();
        }
        o.push_str("}\n\n");
    }
    o.push_str("pub static TABLES: Tables = Tables {\n");
    for (field, name) in [
        ("final_state", "YYFINAL"),
        ("last", "YYLAST"),
        ("ntokens", "YYNTOKENS"),
        ("maxutok", "YYMAXUTOK"),
        ("pact_ninf", "YYPACT_NINF"),
        ("table_ninf", "YYTABLE_NINF"),
        ("maxdepth", "YYMAXDEPTH"),
    ] {
        writeln!(o, "    {field}: {},", define_int(c, name)?).unwrap();
    }
    for field in [
        "translate",
        "pact",
        "defact",
        "pgoto",
        "defgoto",
        "table",
        "check",
        "r1",
        "r2",
    ] {
        writeln!(o, "    {field}: &{},", field.to_uppercase()).unwrap();
    }
    o.push_str("    tname: &TNAME,\n};\n");
    for (name, ty) in [
        ("translate", "u8"),
        ("pact", "i16"),
        ("defact", "i16"),
        ("pgoto", "i16"),
        ("defgoto", "i16"),
        ("table", "i16"),
        ("check", "i16"),
        ("r1", "i16"),
        ("r2", "u8"),
    ] {
        let v = int_array(c, &format!("yy{name}"))?;
        writeln!(
            o,
            "\nstatic {}: [{ty}; {}] = [",
            name.to_uppercase(),
            v.len()
        )
        .unwrap();
        for chunk in v.chunks(16) {
            let row: Vec<String> = chunk.iter().map(i64::to_string).collect();
            writeln!(o, "    {},", row.join(", ")).unwrap();
        }
        o.push_str("];\n");
    }
    let names = tname(c)?;
    let symbols = define_int(c, "YYNTOKENS")? + define_int(c, "YYNNTS")?;
    if names.len() as i64 != symbols {
        return Err(format!(
            "{what}: {} names for {symbols} symbols",
            names.len()
        ));
    }
    writeln!(o, "\nstatic TNAME: [&str; {}] = [", names.len()).unwrap();
    for n in names {
        writeln!(o, "    {n:?},").unwrap();
    }
    o.push_str("];\n");
    Ok(o)
}

/// The number of the rule Bison prints as `case N: /* <text>  */`.
fn rule_number(c: &str, text: &str) -> Result<usize, String> {
    let comment = format!(": /* {text}  */");
    c.lines()
        .find_map(|l| {
            let n = l
                .trim_start()
                .strip_prefix("case ")?
                .strip_suffix(&comment)?;
            n.parse().ok()
        })
        .ok_or(format!("no rule {text:?} in yyparse"))
}

/// The number of the one rule whose action contains `call`.
fn action_rule(c: &str, call: &str) -> Result<usize, String> {
    let mut case = None;
    let mut found = Vec::new();
    for l in c.lines() {
        if let Some(rest) = l.trim_start().strip_prefix("case ") {
            case = rest.split(':').next().and_then(|n| n.parse::<usize>().ok());
        }
        if l.contains(call) {
            found.extend(case);
        }
    }
    match found[..] {
        [n] => Ok(n),
        _ => Err(format!("{} actions call {call:?}, not one", found.len())),
    }
}

fn define_line<'a>(c: &'a str, name: &str) -> Result<&'a str, String> {
    c.lines()
        .find_map(|l| {
            let l = l.trim_start_matches('#').trim_start();
            let rest = l.strip_prefix("define ")?.trim_start().strip_prefix(name)?;
            rest.starts_with([' ', '\t']).then(|| rest.trim())
        })
        .ok_or(format!("no #define {name}"))
}

fn define_int(c: &str, name: &str) -> Result<i64, String> {
    let v = define_line(c, name)?;
    v.trim_matches(|c| c == '(' || c == ')')
        .parse()
        .map_err(|_| format!("#define {name} {v}: not a number"))
}

fn define_str<'a>(c: &'a str, name: &str) -> Result<&'a str, String> {
    Ok(define_line(c, name)?.trim_matches('"'))
}

/// The body of `static const <type> <name>[] =\n{ … };`.
fn array_body<'a>(c: &'a str, name: &str) -> Result<&'a str, String> {
    let pat = format!(" {name}[] =");
    let start = c.find(&pat).ok_or(format!("no array {name}"))?;
    let rest = &c[start..];
    let open = rest.find('{').ok_or(format!("{name}: no '{{'"))?;
    let close = rest.find("};").ok_or(format!("{name}: no '}};'"))?;
    Ok(&rest[open + 1..close])
}

fn int_array(c: &str, name: &str) -> Result<Vec<i64>, String> {
    array_body(c, name)?
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().map_err(|_| format!("{name}: bad entry {s:?}")))
        .collect()
}

/// yytname, each entry's C string literals unescaped and joined (Bison
/// splits `??` into `"?" "?"` to dodge trigraphs).
fn tname(c: &str) -> Result<Vec<String>, String> {
    let body = array_body(c, "yytname")?;
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    let mut it = body.chars().peekable();
    while let Some(ch) = it.next() {
        match ch {
            '"' => {
                let s = cur.get_or_insert_with(String::new);
                loop {
                    match it.next().ok_or("yytname: unterminated string")? {
                        '"' => break,
                        '\\' => match it.next().ok_or("yytname: bad escape")? {
                            '\\' => s.push('\\'),
                            '"' => s.push('"'),
                            '\'' => s.push('\''),
                            e => return Err(format!("yytname: unexpected escape \\{e}")),
                        },
                        o => s.push(o),
                    }
                }
            }
            ',' => out.extend(cur.take()),
            // YY_NULLPTR ends the list
            'Y' => break,
            _ => {}
        }
    }
    out.extend(cur.take());
    Ok(out)
}
