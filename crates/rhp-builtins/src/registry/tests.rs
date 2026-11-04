use super::*;

/// Branches developed in parallel must not register the same builtin
/// twice: the later registration silently shadows the earlier one.
#[test]
fn no_duplicate_builtins() {
    let mut owner: HashMap<&'static str, &'static str> = HashMap::new();
    let mut dups = Vec::new();
    for (module, register) in builtin_modules() {
        let mut m = HashMap::new();
        register(&mut m);
        for name in m.keys() {
            if let Some(prev) = owner.insert(name, module) {
                dups.push(format!("{name}: {prev} and {module}"));
            }
        }
    }
    dups.sort();
    assert!(dups.is_empty(), "duplicate builtins:\n{}", dups.join("\n"));
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
