use super::sys;

#[test]
fn sqlite3_is_linked_statically() {
    // PDO::ATTR_CLIENT_VERSION and phpinfo's "SQLite Library" come from
    // the library: it must be the release of the sqlite3.h rhp compiled
    // against.
    assert_eq!(sys::libversion(), env!("RHP_SQLITE_VERSION"));
    if env!("RHP_SQLITE3_LINK_MODE") != "static" {
        return;
    }
    let exe = std::env::current_exe().unwrap();
    let (tool, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("otool", &["-L"])
    } else {
        ("ldd", &[])
    };
    let Ok(out) = std::process::Command::new(tool)
        .args(args)
        .arg(&exe)
        .output()
    else {
        return;
    };
    let deps = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{tool} failed: {deps}");
    assert!(!deps.contains("libsqlite3"), "{deps}");
}

#[test]
fn sqlstate_descriptions_keep_the_first_entry() {
    use crate::ext::pdo::sqlstate::description;
    assert_eq!(description("HY000"), Some("General error"));
    assert_eq!(description("23000"), Some("Integrity constraint violation"));
    assert_eq!(description("ZZZZZ"), None);
}
