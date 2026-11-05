//! `gen readme [--php-src <dir>] [--check]`: the PHP test support table in
//! README.md, between its phpt-table markers. The passing tests are
//! tests/phpt/known-passing.txt; the totals are tests/phpt/totals.txt (the
//! .phpt files in php-src, per directory) minus tests/phpt/ignored.txt. The
//! notes column is written by hand, and each row keeps its note.
//!
//! With `--php-src <dir>`, a checkout at the commit in tests/phpt/php-src.ref,
//! it first recounts totals.txt from the tree (every `*.phpt` file, as the
//! phpt runner finds them). Do that whenever php-src.ref changes.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use super::util::{finish, flag, read, repo_root};

const README: &str = "README.md";
const TOTALS: &str = "tests/phpt/totals.txt";
const KNOWN: &str = "tests/phpt/known-passing.txt";
const IGNORED: &str = "tests/phpt/ignored.txt";

const START: &str =
    "<!-- phpt-table start: written by `rhp-difftest gen readme`; edit only the notes -->";
const END: &str = "<!-- phpt-table end -->";

/// Extensions rhp provides some functions of without registering the module
/// (get_loaded_extensions() leaves them out). They get their own row too.
const PARTIAL_EXTS: &[&str] = &["intl", "sockets"];

/// The SAPIs rhp implements (rhp-sapi), in table order.
const SAPIS: &[&str] = &["cli", "cgi"];

pub fn main(args: &[String]) -> ExitCode {
    let check = args.iter().any(|a| a == "--check");
    finish("readme", check, files(args))
}

fn files(args: &[String]) -> Result<Vec<(&'static str, String)>, String> {
    let root = repo_root();
    let known_txt = read(&root.join(KNOWN))?;
    let known = list(&known_txt);
    let ignored_txt = match std::fs::read_to_string(root.join(IGNORED)) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{IGNORED}: {e}")),
    };
    let ignored = list(&ignored_txt);
    let mut out = Vec::new();
    let totals = match flag(args, "--php-src") {
        Some(src) => {
            let totals = count(Path::new(src), &known)?;
            out.push((TOTALS, render_totals(&totals)));
            totals
        }
        None => parse_totals(&read(&root.join(TOTALS))?)?,
    };
    let readme = read(&root.join(README))?;
    let exts = implemented_exts();
    out.push((README, rewrite(&readme, &totals, &known, &ignored, &exts)?));
    Ok(out)
}

/// The first word of every line that is not blank or a `#` comment.
fn list(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_whitespace().next())
        .collect()
}

/// The directory a test is counted under: `ext/<name>` and `sapi/<name>`,
/// else the first path component (`Zend`, `tests`).
fn area(path: &str) -> &str {
    let n = if path.starts_with("ext/") || path.starts_with("sapi/") {
        2
    } else {
        1
    };
    match path.match_indices('/').nth(n - 1) {
        Some((i, _)) => &path[..i],
        None => path,
    }
}

/// The ext/ directories of the extensions rhp implements: the modules it
/// reports as loaded, plus [`PARTIAL_EXTS`].
fn implemented_exts() -> BTreeSet<String> {
    rhp_builtins::php_fns::loaded_extension_names()
        .iter()
        .map(|m| {
            let m = m.to_ascii_lowercase();
            m.strip_prefix("zend ").unwrap_or(&m).to_string()
        })
        .chain(PARTIAL_EXTS.iter().map(|e| e.to_string()))
        .collect()
}

/// The .phpt files under `src`, per area. Every known-passing test must be
/// one of them.
fn count(src: &Path, known: &[&str]) -> Result<BTreeMap<String, usize>, String> {
    let mut files = Vec::new();
    crate::phpt::find_phpt(src, Path::new(""), &mut files);
    if files.is_empty() {
        return Err(format!("no .phpt files under {}", src.display()));
    }
    let set: HashSet<&str> = files.iter().map(String::as_str).collect();
    let missing: Vec<&str> = known.iter().copied().filter(|k| !set.contains(k)).collect();
    if let Some(first) = missing.first() {
        return Err(format!(
            "{} known-passing test(s) are not in {}, e.g. {first}",
            missing.len(),
            src.display()
        ));
    }
    let mut totals = BTreeMap::new();
    for f in &files {
        *totals.entry(area(f).to_string()).or_insert(0) += 1;
    }
    Ok(totals)
}

fn render_totals(totals: &BTreeMap<String, usize>) -> String {
    let mut o = String::from(
        "# The .phpt files in php-src at the commit in php-src.ref, per directory\n\
         # (ext/ and sapi/ per extension and SAPI): the totals of README.md's test\n\
         # table. Written by `rhp-difftest gen readme --php-src <php-src>`; rerun it\n\
         # whenever php-src.ref changes.\n",
    );
    for (area, n) in totals {
        writeln!(o, "{area} {n}").unwrap();
    }
    o
}

fn parse_totals(text: &str) -> Result<BTreeMap<String, usize>, String> {
    let mut totals = BTreeMap::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.split_whitespace();
        let (Some(area), Some(n), None) = (words.next(), words.next(), words.next()) else {
            return Err(format!("{TOTALS}: bad line {line:?}"));
        };
        let n = n
            .parse()
            .map_err(|_| format!("{TOTALS}: bad count in {line:?}"))?;
        totals.insert(area.to_string(), n);
    }
    Ok(totals)
}

/// `readme` with the table between the markers regenerated, keeping each
/// row's note.
fn rewrite(
    readme: &str,
    totals: &BTreeMap<String, usize>,
    known: &[&str],
    ignored: &[&str],
    exts: &BTreeSet<String>,
) -> Result<String, String> {
    let start = readme
        .find(START)
        .ok_or_else(|| format!("{README} has no line {START}"))?;
    let body = start + START.len();
    let end = body
        + readme[body..]
            .find(END)
            .ok_or_else(|| format!("{README} has no line {END} after the start marker"))?;
    let notes = old_notes(&readme[body..end]);
    let table = table(totals, known, ignored, exts, &notes)?;
    Ok(format!(
        "{}{START}\n{table}{}",
        &readme[..start],
        &readme[end..]
    ))
}

/// Each table row's note (its last column), by its first column.
fn old_notes(table: &str) -> HashMap<String, String> {
    let mut notes = HashMap::new();
    for line in table.lines().map(str::trim) {
        let Some(inner) = line.strip_prefix('|') else {
            continue;
        };
        let cells: Vec<&str> = inner.trim_end_matches('|').split('|').collect();
        if cells.len() >= 5 {
            let key = cells[0].trim();
            if key != "area" && !key.starts_with("---") {
                notes.insert(key.to_string(), cells[4..].join("|").trim().to_string());
            }
        }
    }
    notes
}

fn table(
    totals: &BTreeMap<String, usize>,
    known: &[&str],
    ignored: &[&str],
    exts: &BTreeSet<String>,
    notes: &HashMap<String, String>,
) -> Result<String, String> {
    let mut total = totals.clone();
    for t in ignored {
        let n = total.get_mut(area(t)).filter(|n| **n > 0).ok_or_else(|| {
            format!("{IGNORED} lists {t}, but {TOTALS} has no test left under its directory")
        })?;
        *n -= 1;
    }
    let ignored: HashSet<&str> = ignored.iter().copied().collect();
    let mut pass: HashMap<&str, usize> = HashMap::new();
    for t in known {
        if ignored.contains(t) {
            return Err(format!("{t} is in both {KNOWN} and {IGNORED}"));
        }
        if !total.contains_key(area(t)) {
            return Err(format!("{KNOWN} lists {t}, a directory {TOTALS} lacks"));
        }
        *pass.entry(area(t)).or_insert(0) += 1;
    }

    // Candidate rows in table order: a label and the areas it adds up. An
    // area goes in the first row that names it, and a row left without tests
    // is dropped.
    let one = |a: &str| (a.to_string(), vec![a.to_string()]);
    let under = |prefix: &str| -> Vec<String> {
        totals
            .keys()
            .filter(|a| a.starts_with(prefix))
            .cloned()
            .collect()
    };
    let mut candidates = vec![("Zend/tests".to_string(), vec!["Zend".to_string()])];
    candidates.push(one("tests"));
    candidates.extend(SAPIS.iter().map(|s| one(&format!("sapi/{s}"))));
    candidates.push(one("ext/standard"));
    for a in totals.keys() {
        if a.strip_prefix("ext/").is_some_and(|e| exts.contains(e)) {
            candidates.push(one(a));
        }
    }
    candidates.push(("other ext/".to_string(), under("ext/")));
    candidates.push(("other sapi/".to_string(), under("sapi/")));
    candidates.extend(totals.keys().map(|a| one(a)));
    let mut used = HashSet::new();
    let mut rows = Vec::new();
    for (label, areas) in candidates {
        let areas: Vec<String> = areas
            .into_iter()
            .filter(|a| total.get(a).is_some_and(|n| *n > 0) && used.insert(a.clone()))
            .collect();
        if !areas.is_empty() {
            rows.push((label, areas));
        }
    }
    let others: Vec<String> = rows
        .iter()
        .filter(|(label, _)| label.starts_with("other "))
        .map(|(label, areas)| {
            let names: Vec<&str> = areas
                .iter()
                .map(|a| a.split_once('/').map_or(a.as_str(), |(_, n)| n))
                .collect();
            format!("{label}: {}", names.join(", "))
        })
        .collect();

    let mut o = String::from("| area | passing | total | % | notes |\n|---|---:|---:|---:|---|\n");
    let row = |o: &mut String, label: &str, p: usize, n: usize, bold: bool| {
        let b = if bold { "**" } else { "" };
        let note = notes.get(label).map(String::as_str).unwrap_or("");
        let note = if note.is_empty() {
            " |".to_string()
        } else {
            format!(" {note} |")
        };
        writeln!(
            o,
            "| {label} | {b}{}{b} | {b}{}{b} | {b}{:.1}%{b} |{note}",
            thousands(p),
            thousands(n),
            p as f64 * 100.0 / n as f64
        )
        .unwrap();
    };
    for (label, areas) in &rows {
        let p = areas
            .iter()
            .map(|a| pass.get(a.as_str()).copied().unwrap_or(0))
            .sum();
        let n = areas.iter().map(|a| total[a]).sum();
        row(&mut o, label, p, n, false);
    }
    let n: usize = total.values().sum();
    row(&mut o, "**total**", known.len(), n, true);
    if !others.is_empty() {
        writeln!(
            o,
            "\nNot implemented, so counted under {}.",
            others.join("; ")
        )
        .unwrap();
    }
    Ok(o)
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut o = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            o.push(',');
        }
        o.push(c);
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn areas() {
        assert_eq!(area("Zend/tests/a/b.phpt"), "Zend");
        assert_eq!(area("tests/lang/x.phpt"), "tests");
        assert_eq!(area("ext/intl/uchar/tests/x.phpt"), "ext/intl");
        assert_eq!(area("sapi/cli/tests/x.phpt"), "sapi/cli");
    }

    #[test]
    fn numbers() {
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(7044), "7,044");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn table_keeps_notes_and_groups_the_rest() {
        let totals = parse_totals(
            "# c\nZend 4\ntests 2\next/standard 3\next/json 2\next/gd 5\nsapi/cli 2\nsapi/fpm 1\n",
        )
        .unwrap();
        let known = list("# k\nZend/tests/a.phpt\next/json/tests/a.phpt\next/gd/tests/a.phpt\n");
        let ignored = list("ext/gd/tests/b.phpt opcode dump\n");
        let exts: BTreeSet<String> = ["standard", "json"].iter().map(|s| s.to_string()).collect();
        let readme = format!(
            "intro\n{START}\n| area | passing | total | % | notes |\n|---|\n\
             | ext/json | 9 | 9 | 100.0% | a note |\n| other ext/ | 0 | 1 | 0% | gd: none |\n{END}\nrest\n"
        );
        let got = rewrite(&readme, &totals, &known, &ignored, &exts).unwrap();
        let want = format!(
            "intro\n{START}\n| area | passing | total | % | notes |\n|---|---:|---:|---:|---|\n\
             | Zend/tests | 1 | 4 | 25.0% | |\n\
             | tests | 0 | 2 | 0.0% | |\n\
             | sapi/cli | 0 | 2 | 0.0% | |\n\
             | ext/standard | 0 | 3 | 0.0% | |\n\
             | ext/json | 1 | 2 | 50.0% | a note |\n\
             | other ext/ | 1 | 4 | 25.0% | gd: none |\n\
             | other sapi/ | 0 | 1 | 0.0% | |\n\
             | **total** | **3** | **18** | **16.7%** | |\n\
             \nNot implemented, so counted under other ext/: gd; other sapi/: fpm.\n{END}\nrest\n"
        );
        assert_eq!(got, want);
        // Idempotent.
        assert_eq!(
            rewrite(&got, &totals, &known, &ignored, &exts).unwrap(),
            got
        );
    }

    #[test]
    fn inconsistent_lists_are_errors() {
        let totals = parse_totals("Zend 1\n").unwrap();
        let exts = BTreeSet::new();
        let readme = format!("{START}\n{END}\n");
        let known = ["Zend/tests/a.phpt"];
        assert!(rewrite(&readme, &totals, &known, &known, &exts).is_err());
        assert!(rewrite(&readme, &totals, &["ext/x/tests/a.phpt"], &[], &exts).is_err());
        assert!(rewrite("no markers", &totals, &known, &[], &exts).is_err());
    }
}
