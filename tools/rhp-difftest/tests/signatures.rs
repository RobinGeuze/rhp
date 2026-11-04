//! The generated signature table (crates/rhp-bytecode/src/signatures)
//! against `php -n` Reflection, for every function, class and method rhp
//! implements: parameter names, types, defaults, by-ref / prefer-ref,
//! variadics, return types (tentative ones too), modifiers, parents and
//! extension names. A second pass compares rhp's own Reflection output with
//! php's for the same items, which checks that the engine reads the table.
//!
//! Skips unless `php` is the release rhp targets.

use rhp_bytecode::signatures::{self, Param, Signature};
use rhp_difftest::{php_available, run_php, run_rhp};

/// Lists what rhp implements: `F name`, `C name`, `M class name`.
const INVENTORY: &str = r#"<?php
foreach (get_defined_functions()['internal'] as $f) { echo "F $f\n"; }
foreach (array_merge(get_declared_classes(), get_declared_interfaces(), get_declared_traits()) as $c) {
    $r = new ReflectionClass($c);
    if (!$r->isInternal()) { continue; }
    echo "C ", $r->getName(), "\n";
    foreach ($r->getMethods() as $m) {
        if ($m->getDeclaringClass()->getName() === $r->getName()) { echo "M ", $r->getName(), " ", $m->getName(), "\n"; }
    }
}
"#;

/// Prints one row per item of `$items` in the same format as [`render`].
/// With `$values`, each default's evaluated value is appended (what
/// getDefaultValue / getDefaultValueConstantName report).
const DUMP: &str = r#"
function sig_params(ReflectionFunctionAbstract $f, bool $values): string {
    $o = [];
    foreach ($f->getParameters() as $p) {
        $s = '';
        if ($p->isPassedByReference()) { $s .= $p->canBePassedByValue() ? '&?' : '&'; }
        if ($p->isVariadic()) { $s .= '...'; }
        $s .= '$' . $p->getName();
        if ($p->hasType()) { $s .= ': ' . $p->getType(); }
        if ($p->isOptional() && !$p->isVariadic()) {
            $t = (string) $p;
            $needle = '$' . $p->getName() . ' = ';
            $at = strpos($t, $needle);
            $d = $at === false ? '?' : substr($t, $at + strlen($needle), -2);
            $s .= ' = ' . ($d === '<default>' ? 'UNKNOWN' : $d);
            if ($values && $p->isDefaultValueAvailable()) {
                try {
                    $s .= $p->isDefaultValueConstant()
                        ? ' {const ' . $p->getDefaultValueConstantName() . '}'
                        : ' {' . str_replace("\n", '\n', var_export($p->getDefaultValue(), true)) . '}';
                } catch (Throwable $e) {
                    $s .= ' {' . get_class($e) . '}';
                }
            }
        }
        $o[] = $s;
    }
    return implode(', ', $o);
}
function sig_ret(ReflectionFunctionAbstract $f): string {
    if ($f->hasReturnType()) { return ': ' . $f->getReturnType(); }
    if ($f->hasTentativeReturnType()) { return ': tentative ' . $f->getTentativeReturnType(); }
    return '';
}
foreach ($items as [$kind, $a, $b]) {
    if ($kind === 'F') {
        if (!function_exists($a)) { continue; }
        $f = new ReflectionFunction($a);
        echo 'F ', $f->getName(), ' [', $f->getExtensionName(), ']', $f->isDeprecated() ? ' deprecated' : '',
            sig_ret($f), ' (', sig_params($f, $values), ")\n";
    } elseif ($kind === 'C') {
        if (!class_exists($a, false) && !interface_exists($a, false) && !trait_exists($a, false)) { continue; }
        $c = new ReflectionClass($a);
        $kind = $c->isEnum() ? 'enum' : ($c->isInterface() ? 'interface' : ($c->isTrait() ? 'trait' : 'class'));
        $parent = $c->getParentClass();
        echo 'C ', $c->getName(), ' [', $c->getExtensionName(), '] ', $kind, ' ', $c->getModifiers(),
            $parent ? ' extends ' . $parent->getName() : '', "\n";
        echo 'I ', $c->getName(), ' ', implode(',', $c->getInterfaceNames()), "\n";
    } else {
        if (!method_exists($a, $b)) { continue; }
        $m = new ReflectionMethod($a, $b);
        if (strcasecmp($m->getDeclaringClass()->getName(), $a) !== 0) { continue; }
        echo 'M ', $m->getDeclaringClass()->getName(), '::', $m->getName(), ' ', $m->getModifiers() & 0x77,
            $m->isDeprecated() ? ' deprecated' : '', sig_ret($m), ' (', sig_params($m, $values), ")\n";
    }
}
echo "END\n";
"#;

fn params(ps: &[Param]) -> String {
    ps.iter()
        .map(|p| {
            let mut s = String::new();
            if p.prefer_ref() {
                s.push_str("&?");
            } else if p.by_ref() {
                s.push('&');
            }
            if p.variadic() {
                s.push_str("...");
            }
            s.push('$');
            s.push_str(p.name);
            if !p.ty.is_empty() {
                s.push_str(": ");
                s.push_str(p.ty);
            }
            if !p.variadic() {
                if let Some(d) = p.default() {
                    s.push_str(" = ");
                    s.push_str(d);
                } else if p.flags & signatures::U != 0 {
                    s.push_str(" = UNKNOWN");
                }
            }
            s
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn ret(r: &str, tentative: bool) -> String {
    match (r.is_empty(), tentative) {
        (true, _) => String::new(),
        (false, true) => format!(": tentative {r}"),
        (false, false) => format!(": {r}"),
    }
}

/// The table's row for one inventory item, `None` when it has none.
fn render(kind: &str, a: &str, b: &str) -> Option<String> {
    match kind {
        "F" => {
            let f = signatures::function(a.as_bytes())?;
            Some(format!(
                "F {} [{}]{}{} ({})",
                f.name,
                f.ext,
                if f.deprecated() { " deprecated" } else { "" },
                ret(f.ret, false),
                params(f.params)
            ))
        }
        "C" => {
            let c = signatures::class(a.as_bytes())?;
            let kind = ["class", "interface", "trait", "enum"][c.kind as usize];
            let parent = if c.parent.is_empty() {
                String::new()
            } else {
                format!(" extends {}", c.parent)
            };
            Some(format!(
                "C {} [{}] {kind} {}{parent}",
                c.name,
                c.ext,
                c.flags & signatures::CLASS_MODIFIERS
            ))
        }
        _ => {
            let m = signatures::method(a.as_bytes(), b.as_bytes())?;
            Some(format!(
                "M {}::{} {}{}{} ({})",
                m.class,
                m.name,
                m.flags & 0x77,
                if m.deprecated() { " deprecated" } else { "" },
                ret(m.ret, m.tentative()),
                params(m.params)
            ))
        }
    }
}

/// `php` is the release rhp targets.
fn reference_php() -> bool {
    if !php_available() {
        eprintln!("[signatures] no php — skipping");
        return false;
    }
    let v = run_php(b"<?php echo PHP_VERSION;").unwrap_or_default();
    if v != rhp_builtins::PHP_VERSION.as_bytes() {
        eprintln!(
            "[signatures] php is {}, not {} — skipping",
            String::from_utf8_lossy(&v),
            rhp_builtins::PHP_VERSION
        );
        return false;
    }
    true
}

/// rhp's inventory as `(kind, a, b)`.
fn inventory() -> Vec<(String, String, String)> {
    let out = run_rhp(INVENTORY.as_bytes()).expect("rhp inventory");
    String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|l| {
            let mut w = l.splitn(3, ' ');
            let k = w.next()?.to_string();
            let a = w.next()?.to_string();
            let b = w.next().unwrap_or("").to_string();
            Some((k, a, b))
        })
        .collect()
}

fn dump_script(items: &[(String, String, String)], values: bool) -> String {
    let mut s = String::from("<?php\n$values = ");
    s.push_str(if values { "true" } else { "false" });
    s.push_str(";\n$items = [\n");
    for (k, a, b) in items {
        s.push_str(&format!("['{k}', '{}', '{b}'],\n", a.replace('\\', "\\\\")));
    }
    s.push_str("];\n");
    s.push_str(DUMP);
    s
}

#[test]
fn table_matches_php_reflection() {
    if !reference_php() {
        return;
    }
    let items = inventory();
    assert!(items.len() > 1000, "inventory too small: {}", items.len());
    let php = run_php(dump_script(&items, false).as_bytes()).expect("php dump");
    let php = String::from_utf8_lossy(&php);
    assert!(
        php.lines().any(|l| l == "END"),
        "php's Reflection dump stopped early"
    );
    let mut php_rows = std::collections::HashMap::new();
    for l in php.lines() {
        let key = match l.get(..2) {
            Some("F ") | Some("C ") | Some("I ") => {
                l.split(' ').nth(1).unwrap_or("").to_ascii_lowercase()
            }
            Some("M ") => l.split(' ').nth(1).unwrap_or("").to_ascii_lowercase(),
            _ => continue,
        };
        php_rows.insert((l[..1].to_string(), key), l.to_string());
    }
    let mut bad = Vec::new();
    let mut checked = 0;
    for (k, a, b) in &items {
        let key = if k == "M" {
            format!("{a}::{b}").to_ascii_lowercase()
        } else {
            a.to_ascii_lowercase()
        };
        let Some(want) = php_rows.get(&(k.clone(), key.clone())) else {
            continue; // php lacks it: an rhp-only helper or extension
        };
        let Some(got) = render(k, a, b) else {
            bad.push(format!("missing from the table: {want}"));
            continue;
        };
        checked += 1;
        if &got != want {
            bad.push(format!("php:   {want}\ntable: {got}"));
        }
        if k == "C" {
            // Declared interfaces are among the ones php reports.
            let c = signatures::class(a.as_bytes()).unwrap();
            let all = php_rows
                .get(&("I".to_string(), key))
                .map(|l| l.splitn(3, ' ').nth(2).unwrap_or("").to_string())
                .unwrap_or_default();
            let all: Vec<&str> = all.split(',').collect();
            for i in c.interfaces {
                if !all.iter().any(|x| x.eq_ignore_ascii_case(i)) {
                    bad.push(format!("{}: php does not report interface {i}", c.name));
                }
            }
        }
    }
    eprintln!("[signatures] {checked} rows checked against php");
    assert!(
        bad.is_empty(),
        "{} mismatches:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// rhp's Reflection of a builtin function or a built-in class's method
/// prints php's row; a class reports php's extension. (Class shapes,
/// modifiers and interface lists come from rhp's prelude declarations, not
/// from the table.)
#[test]
fn rhp_reflection_matches_php() {
    if !reference_php() {
        return;
    }
    let items = inventory();
    let script = dump_script(&items, true);
    let php = run_php(script.as_bytes()).expect("php dump");
    let rhp = run_rhp(script.as_bytes()).expect("rhp dump");
    let php = String::from_utf8_lossy(&php);
    let rhp = String::from_utf8_lossy(&rhp);
    // A dump that stopped early would drop the remaining rows silently.
    for (who, out) in [("php", &php), ("rhp", &rhp)] {
        let tail: Vec<&str> = out.lines().rev().take(5).collect();
        assert!(
            out.lines().any(|l| l == "END"),
            "{who}'s Reflection dump stopped early; last lines: {tail:?}"
        );
    }
    // Rows keyed by kind and name; a class row is compared up to its
    // extension.
    let key = |l: &str| {
        let mut w = l.splitn(3, ' ');
        let k = w.next().unwrap_or("").to_string();
        let n = w.next().unwrap_or("").to_ascii_lowercase();
        (k, n)
    };
    let cmp = |l: &str| -> Option<String> {
        match l.get(..2) {
            Some("F ") | Some("M ") => Some(l.to_string()),
            Some("C ") => Some(l.split(']').next().unwrap_or("").to_string()),
            _ => None,
        }
    };
    let php_rows: std::collections::HashMap<_, _> = php
        .lines()
        .filter_map(|l| Some((key(l), cmp(l)?)))
        .collect();
    let mut bad = Vec::new();
    let mut checked = 0;
    for l in rhp.lines() {
        let Some(got) = cmp(l) else { continue };
        let Some(want) = php_rows.get(&key(l)) else {
            continue;
        };
        checked += 1;
        if *want != got {
            bad.push(format!("php: {want}\nrhp: {got}"));
        }
    }
    eprintln!("[signatures] {checked} rhp Reflection rows checked against php");
    assert!(
        bad.is_empty(),
        "{} mismatches:\n{}",
        bad.len(),
        bad.join("\n")
    );
}
