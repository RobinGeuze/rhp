use super::Thrown;

#[test]
fn unsupported_message_appends_location_last() {
    let t = Thrown::unsupported("goto statement", "phase 2");
    assert_eq!(
        t.unsupported_message().unwrap(),
        "RHP_UNSUPPORTED: goto statement is not supported (phase 2)"
    );
    let t = t.with_location(|| "/p/a.php".to_string(), 12);
    assert_eq!(
        t.unsupported_message().unwrap(),
        "RHP_UNSUPPORTED: goto statement is not supported (phase 2) in /p/a.php on line 12"
    );
    // The innermost (first) location wins.
    let t = t.with_location(|| "/p/b.php".to_string(), 3);
    assert!(t
        .unsupported_message()
        .unwrap()
        .ends_with("a.php on line 12"));
    assert!(Thrown::Exit(0).unsupported_message().is_none());
    assert!(matches!(
        Thrown::Exit(1).with_location(String::new, 1),
        Thrown::Exit(1)
    ));
}
