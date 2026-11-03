use super::*;

#[test]
fn tables_are_sorted_for_lookup() {
    let lc = |s: &str| s.to_ascii_lowercase();
    assert!(functions()
        .windows(2)
        .all(|w| lc(w[0].name) < lc(w[1].name)));
    assert!(classes().windows(2).all(|w| lc(w[0].name) < lc(w[1].name)));
    assert!(methods()
        .windows(2)
        .all(|w| (lc(w[0].class), lc(w[0].name)) < (lc(w[1].class), lc(w[1].name))));
}

/// Callback functions whose php body requires more arguments than their
/// arginfo (`array_udiff(): expects at least 2 arguments`); rhp's
/// registration carries that minimum.
const BODY_MIN: &[&str] = &[
    "array_diff_uassoc",
    "array_diff_ukey",
    "array_intersect_uassoc",
    "array_intersect_ukey",
    "array_udiff",
    "array_udiff_assoc",
    "array_udiff_uassoc",
    "array_uintersect",
    "array_uintersect_assoc",
    "array_uintersect_uassoc",
];

/// Parameters php declares that the body does not implement yet: the
/// registration's maximum stops before them, so such an argument is refused
/// (RHP_UNSUPPORTED) instead of ignored.
///
/// None at present: a body that cannot honour a parameter for some values
/// refuses those values itself (bcround's RoundingMode, a non-empty
/// stream_context_create $params).
const NOT_IMPLEMENTED: &[(&str, usize)] = &[];

/// A registration's arity is php's, except for the parameters listed in
/// [`NOT_IMPLEMENTED`]: a lower maximum would refuse arguments php takes, a
/// higher minimum would reject calls php accepts. That each body reads all
/// of its registered parameters is not checked here; a body that ignores
/// one must lower its maximum and be listed.
#[test]
fn registry_arity_matches_stubs() {
    let mut bad = Vec::new();
    let mut names: Vec<_> = crate::registry().keys().copied().collect();
    names.sort();
    for name in names {
        let b = &crate::registry()[name];
        let Some(f) = function(name.as_bytes()) else {
            continue;
        };
        let min_ok = b.min_args == f.min_args() || BODY_MIN.contains(&name);
        let max_ok = match NOT_IMPLEMENTED.iter().find(|(n, _)| *n == name) {
            Some(&(_, implemented)) => {
                b.max_args == Some(implemented) && f.max_args().is_some_and(|m| m > implemented)
            }
            None => b.max_args == f.max_args(),
        };
        if !min_ok || !max_ok {
            bad.push(format!(
                "{name}: registered {}..{:?}, stub {}..{:?}",
                b.min_args,
                b.max_args,
                f.min_args(),
                f.max_args()
            ));
        }
    }
    assert!(bad.is_empty(), "{}:\n{}", bad.len(), bad.join("\n"));
}
