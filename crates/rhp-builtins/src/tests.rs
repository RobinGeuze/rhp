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
