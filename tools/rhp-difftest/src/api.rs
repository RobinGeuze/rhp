//! `rhp-difftest api`: the surface of the internal classes against php's.
//!
//! The same Reflection dump runs under `php -n` and on rhp: for every
//! internal class, interface, trait and enum, one row for the class (kind,
//! modifiers, parent, interfaces, extension) and one per method, property
//! and constant it declares itself (modifiers, property type and default,
//! constant value). Signatures are checked by tests/signatures.rs.
//!
//! Only classes of extensions both engines load are compared (and PDO's
//! driver constants of drivers both load). A class one side lacks is one
//! row; otherwise the member rows are compared as sets. The result is a
//! sorted list of `- row` (php has it, rhp does not) and `+ row` (rhp has
//! it, php does not).
//!
//! `--check-known FILE` fails on a difference not in FILE and lists the
//! rows of FILE that no longer differ (for a class that was compared), to
//! take off the list; `--write-known FILE` rewrites it.

use std::collections::{BTreeMap, BTreeSet};
use std::process::ExitCode;

use crate::{php_available, run_php, run_rhp};

const USAGE: &str = "usage: rhp-difftest api [--check-known FILE | --write-known FILE]\n\
\x20 --check-known  fail on a difference not in FILE, or a row of FILE that is fixed\n\
\x20 --write-known  rewrite FILE with the current differences\n";

/// The list kept in the repository.
pub const KNOWN: &str = "tests/api/known-differences.txt";

/// Prints `X ext` per loaded extension, then the rows, then `END`.
pub const DUMP: &str = r#"<?php
foreach (get_loaded_extensions() as $e) { echo "X ", strtolower($e), "\n"; }
function api_mods(int $m): string {
    return implode(' ', Reflection::getModifierNames($m));
}
function api_val($v): string {
    if (is_object($v)) { return 'object(' . get_class($v) . ')'; }
    if (is_float($v) && is_nan($v)) { return 'NAN'; }
    return str_replace(["\n", "\r"], ['\n', '\r'], var_export($v, true));
}
$all = array_merge(get_declared_classes(), get_declared_interfaces(), get_declared_traits());
sort($all);
foreach ($all as $c) {
    $r = new ReflectionClass($c);
    if (!$r->isInternal()) { continue; }
    $n = $r->getName();
    $ext = strtolower((string) $r->getExtensionName());
    $kind = $r->isEnum() ? 'enum' : ($r->isInterface() ? 'interface' : ($r->isTrait() ? 'trait' : 'class'));
    $p = $r->getParentClass();
    $ifs = $r->getInterfaceNames();
    sort($ifs);
    echo "$ext\t$n\tC $n $kind", ($m = api_mods($r->getModifiers())) === '' ? '' : " $m",
        $p ? ' extends ' . $p->getName() : '', $ifs ? ' implements ' . implode(',', $ifs) : '', "\n";
    foreach ($r->getMethods() as $m) {
        if ($m->getDeclaringClass()->getName() !== $n) { continue; }
        echo "$ext\t$n\tM $n::", $m->getName(), ' ', api_mods($m->getModifiers()), "\n";
    }
    foreach ($r->getProperties() as $pr) {
        if ($pr->getDeclaringClass()->getName() !== $n) { continue; }
        $t = $pr->getType();
        echo "$ext\t$n\tP $n::\$", $pr->getName(), ' ', api_mods($pr->getModifiers()),
            $t === null ? '' : " $t",
            $pr->hasDefaultValue() && $pr->getDefaultValue() !== null ? ' = ' . api_val($pr->getDefaultValue()) : '', "\n";
    }
    foreach ($r->getReflectionConstants() as $k) {
        if ($k->getDeclaringClass()->getName() !== $n) { continue; }
        $v = $k->isEnumCase() ? 'case' : api_val($k->getValue());
        echo "$ext\t$n\tK $n::", $k->getName(), ' ', api_mods($k->getModifiers()), " = $v\n";
    }
}
echo "END\n";
"#;

/// One engine's dump: its extensions, and per class its extension and rows.
struct Surface {
    exts: BTreeSet<String>,
    classes: BTreeMap<String, (String, BTreeSet<String>)>,
}

fn parse(out: &[u8], who: &str) -> Result<Surface, String> {
    let text = String::from_utf8_lossy(out);
    if !text.lines().any(|l| l == "END") {
        let tail: Vec<&str> = text.lines().rev().take(5).collect();
        return Err(format!("{who}'s dump stopped early; last lines: {tail:?}"));
    }
    let mut s = Surface {
        exts: BTreeSet::new(),
        classes: BTreeMap::new(),
    };
    for l in text.lines() {
        if let Some(e) = l.strip_prefix("X ") {
            s.exts.insert(e.to_string());
            continue;
        }
        let mut w = l.splitn(3, '\t');
        let (Some(ext), Some(class), Some(row)) = (w.next(), w.next(), w.next()) else {
            continue;
        };
        s.classes
            .entry(class.to_string())
            .or_insert_with(|| (ext.to_string(), BTreeSet::new()))
            .1
            .insert(row.to_string());
    }
    Ok(s)
}

/// The differences, and the classes that were compared.
pub struct Report {
    pub diffs: BTreeSet<String>,
    pub compared: BTreeSet<String>,
}

/// Runs the dump on both engines and compares.
pub fn compare() -> Result<Report, String> {
    let php = parse(&run_php(DUMP.as_bytes())?, "php")?;
    let rhp = parse(&run_rhp(DUMP.as_bytes())?, "rhp")?;
    let mut diffs = BTreeSet::new();
    let mut compared = BTreeSet::new();
    let both = |ext: &str| php.exts.contains(ext) && rhp.exts.contains(ext);
    for (class, (ext, rows)) in &php.classes {
        if !both(ext) {
            continue;
        }
        compared.insert(class.clone());
        let rows: BTreeSet<String> = rows
            .iter()
            .filter(|r| driver_const_loaded(r, &php.exts) && driver_const_loaded(r, &rhp.exts))
            .cloned()
            .collect();
        let rows = &rows;
        match rhp.classes.get(class) {
            None => {
                diffs.insert(format!("- {}", class_row(rows)));
            }
            Some((_, got)) => {
                let got: BTreeSet<String> = got
                    .iter()
                    .filter(|r| driver_const_loaded(r, &php.exts))
                    .cloned()
                    .collect();
                let got = &got;
                for r in rows.difference(got) {
                    diffs.insert(format!("- {r}"));
                }
                for r in got.difference(rows) {
                    diffs.insert(format!("+ {r}"));
                }
            }
        }
    }
    for (class, (ext, rows)) in &rhp.classes {
        if both(ext) && !php.classes.contains_key(class) {
            compared.insert(class.clone());
            diffs.insert(format!("+ {}", class_row(rows)));
        }
    }
    Ok(Report { diffs, compared })
}

/// PDO's `PDO::MYSQL_…` style constants come from the driver extension
/// (pdo_mysql…), not from PDO: a row for a driver an engine does not load
/// is left out.
fn driver_const_loaded(row: &str, exts: &BTreeSet<String>) -> bool {
    const DRIVERS: &[(&str, &str)] = &[
        ("DBLIB_", "pdo_dblib"),
        ("FB_", "pdo_firebird"),
        ("MYSQL_", "pdo_mysql"),
        ("OCI_", "pdo_oci"),
        ("ODBC_", "pdo_odbc"),
        ("PGSQL_", "pdo_pgsql"),
        ("SQLITE_", "pdo_sqlite"),
    ];
    let Some(name) = row.strip_prefix("K PDO::") else {
        return true;
    };
    DRIVERS
        .iter()
        .find(|(p, _)| name.starts_with(p))
        .is_none_or(|(_, ext)| exts.contains(*ext))
}

fn class_row(rows: &BTreeSet<String>) -> &str {
    rows.iter()
        .find(|r| r.starts_with("C "))
        .map(String::as_str)
        .unwrap_or("C ?")
}

/// The class a diff row is about.
fn row_class(row: &str) -> &str {
    let rest = row.get(4..).unwrap_or("");
    let end = rest.find([' ', ':']).unwrap_or(rest.len());
    &rest[..end]
}

/// Rows of `known` that are new (not listed) and fixed (listed, compared,
/// no longer different).
pub fn check(report: &Report, known: &[String]) -> (Vec<String>, Vec<String>) {
    let known: BTreeSet<&str> = known.iter().map(String::as_str).collect();
    let new = report
        .diffs
        .iter()
        .filter(|d| !known.contains(d.as_str()))
        .cloned()
        .collect();
    let fixed = known
        .iter()
        .filter(|k| report.compared.contains(row_class(k)) && !report.diffs.contains(**k))
        .map(|k| k.to_string())
        .collect();
    (new, fixed)
}

const HEADER: &str = "\
# Internal-class surface rows where rhp differs from php 8.5
# (`rhp-difftest api`, tools/rhp-difftest/src/api.rs): `- row` php has and
# rhp lacks, `+ row` rhp has and php lacks. Only shrink this list (`--check-known`
# names the FIXED rows); rewrite it with
# `cargo run -p rhp-difftest -- api --write-known tests/api/known-differences.txt`.
";

pub fn main(args: &[String]) -> ExitCode {
    let (mode, file) = match args {
        [] => ("print", None),
        [m, f] if m == "--check-known" || m == "--write-known" => (m.as_str(), Some(f)),
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    if !php_available() {
        eprintln!("rhp-difftest api: no php");
        return ExitCode::from(2);
    }
    let report = match compare() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("rhp-difftest api: {e}");
            return ExitCode::FAILURE;
        }
    };
    match (mode, file) {
        ("--write-known", Some(f)) => {
            let mut text = HEADER.to_string();
            for d in &report.diffs {
                text.push_str(d);
                text.push('\n');
            }
            if let Err(e) = std::fs::write(f, text) {
                eprintln!("rhp-difftest api: {f}: {e}");
                return ExitCode::FAILURE;
            }
            println!("{} differences written to {f}", report.diffs.len());
            ExitCode::SUCCESS
        }
        ("--check-known", Some(f)) => {
            let known = match crate::read_known(std::path::Path::new(f)) {
                Ok(k) => k,
                Err(e) => {
                    eprintln!("rhp-difftest api: {f}: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let (new, fixed) = check(&report, &known);
            for n in &new {
                println!("NEW   {n}");
            }
            for x in &fixed {
                println!("FIXED {x}");
            }
            println!(
                "{} classes compared, {} differences, {} new, {} fixed",
                report.compared.len(),
                report.diffs.len(),
                new.len(),
                fixed.len()
            );
            if new.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        _ => {
            for d in &report.diffs {
                println!("{d}");
            }
            eprintln!(
                "{} classes compared, {} differences",
                report.compared.len(),
                report.diffs.len()
            );
            ExitCode::SUCCESS
        }
    }
}
