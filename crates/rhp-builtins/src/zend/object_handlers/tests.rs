use super::*;

#[test]
fn registry_lookup_ignores_case() {
    let gmp = registered(b"gmp").expect("GMP registers handlers");
    assert!(std::ptr::eq(gmp, registered(b"GMP").unwrap()));
    assert!(gmp.handlers.do_operation.is_some());
    assert!(matches!(
        registered(b"XMLPARSER").map(|t| t.handlers.clone_obj),
        Some(CloneObj::None)
    ));
    assert!(!registered(b"directory").unwrap().create_object);
    assert!(registered(b"stdClass").is_none());
    assert!(registered(b"").is_none());
}
