use super::*;

/// Branches developed in parallel must not register the same builtin
/// twice: the later registration silently shadows the earlier one.
#[test]
fn no_duplicate_builtins() {
    let mut owner: HashMap<&'static str, &'static str> = HashMap::new();
    let mut dups = Vec::new();
    for md in modules() {
        let mut m = HashMap::new();
        (md.functions)(&mut m);
        for name in m.keys() {
            if let Some(prev) = owner.insert(name, md.name) {
                dups.push(format!("{name}: {prev} and {}", md.name));
            }
        }
    }
    dups.sort();
    assert!(dups.is_empty(), "duplicate builtins:\n{}", dups.join("\n"));
}

/// A module carries php's name for it: the modules and the loaded
/// extensions rhp reports agree, apart from the ones it deliberately
/// leaves unreported.
#[test]
fn modules_are_php_modules() {
    use crate::zend::builtin_functions::LOADED_EXTS;
    // Modules rhp has but does not report as loaded, until they are
    // complete enough that code testing `extension_loaded()` should use
    // them.
    const UNREPORTED_MODULES: &[&str] = &["intl", "readline", "sockets"];
    let names: Vec<&str> = modules().iter().map(|md| md.name).collect();
    let mut seen = std::collections::HashSet::new();
    for n in &names {
        assert!(seen.insert(n), "module {n} defined twice");
        assert!(
            LOADED_EXTS.contains(n) != UNREPORTED_MODULES.contains(n),
            "module {n}: in exactly one of LOADED_EXTS and UNREPORTED_MODULES"
        );
    }
    for e in LOADED_EXTS.iter().chain(UNREPORTED_MODULES) {
        assert!(names.contains(e), "{e} is not a module");
    }
}

/// Every constant has one definition. The engines load `constants()`
/// into a map, so a second definition silently overrides the first, and
/// a version bump made in one copy (version.rs) would not take effect.
#[test]
fn no_duplicate_constants() {
    let mut seen = std::collections::HashSet::new();
    let mut dups: Vec<&str> = constants()
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| !seen.insert(*name))
        .collect();
    dups.sort();
    assert!(dups.is_empty(), "constants defined twice: {dups:?}");
}

/// The golden lang/engine_helper_functions checks that user code can't
/// reach any engine helper (a builtin named `__…`), so its list must name
/// every one.
#[test]
fn engine_helper_golden_lists_every_helper() {
    let golden = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/golden/lang/engine_helper_functions.php"
    );
    let src = std::fs::read_to_string(golden).expect("read the golden");
    let list = src
        .split_once("$helpers = [")
        .and_then(|(_, rest)| rest.split_once("];"))
        .expect("the golden's $helpers list")
        .0;
    let mut listed: Vec<&str> = list
        .split(',')
        .map(|s| s.trim().trim_matches('\''))
        .filter(|s| !s.is_empty())
        .collect();
    listed.sort_unstable();
    let mut registered: Vec<&str> = registry()
        .keys()
        .copied()
        .filter(|n| n.starts_with("__"))
        .collect();
    registered.sort_unstable();
    assert_eq!(listed, registered);
}

/// Every builtin registered under a php name has a stub row, so its arity
/// is php's ([`crate::register`] takes it from there); the `__` engine
/// helpers have none.
#[test]
fn php_builtins_have_stubs() {
    let mut bad: Vec<_> = crate::registry()
        .keys()
        .filter(|name| {
            rhp_bytecode::signatures::function(name.as_bytes()).is_some() == name.starts_with("__")
        })
        .copied()
        .collect();
    bad.sort();
    assert!(bad.is_empty(), "{bad:?}");
}
