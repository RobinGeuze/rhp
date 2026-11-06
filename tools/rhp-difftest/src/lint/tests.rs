use super::*;

#[test]
fn compiles_plain_php() {
    assert_eq!(lint_source("a.php", b"<?php echo 1 + 2;"), Ok(()));
    assert_eq!(lint_source("t.php", b"just inline html"), Ok(()));
}

#[test]
fn reports_parse_errors() {
    match lint_source("a.php", b"<?php function (") {
        Err(Refusal::Parse { .. }) => {}
        other => panic!("expected a parse refusal, got {other:?}"),
    }
}

#[test]
fn unsupported_refusals_carry_the_line_but_not_the_file() {
    // Spread arguments to an anonymous class are refused (deferred).
    let r = lint_source("/abs/dir/a.php", b"<?php\n\n$x = new class(...[1]) {};\n").unwrap_err();
    match &r {
        Refusal::Unsupported { line, .. } => assert_eq!(*line, Some(3)),
        other => panic!("expected an unsupported refusal, got {other:?}"),
    }
    let shown = r.to_string();
    assert!(shown.ends_with("(line 3)"), "{shown}");
    assert!(!shown.contains("/abs/dir"), "{shown}");
}

#[test]
fn known_list_takes_the_first_tab_field() {
    let dir = std::env::temp_dir().join(format!("rhp-lint-known-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let p = dir.join("known.txt");
    fs::write(
        &p,
        "# header\n\nvendor/a.php\t# unsupported x\nvendor/b.php\n",
    )
    .unwrap();
    let k = read_known(&p).unwrap();
    fs::remove_dir_all(&dir).unwrap();
    assert_eq!(
        k.into_iter().collect::<Vec<_>>(),
        vec!["vendor/a.php".to_string(), "vendor/b.php".to_string()]
    );
}

#[test]
fn collect_keys_by_root_name_and_skips_non_php() {
    let dir = std::env::temp_dir().join(format!("rhp-lint-collect-{}", std::process::id()));
    let root = dir.join("vendor");
    fs::create_dir_all(root.join("pkg/src")).unwrap();
    fs::write(root.join("pkg/src/A.php"), "<?php class A {}").unwrap();
    fs::write(root.join("pkg/README.md"), "not php").unwrap();
    fs::write(root.join("pkg/b.php"), "<?php").unwrap();
    let mut out = Vec::new();
    collect(&root, &mut out).unwrap();
    fs::remove_dir_all(&dir).unwrap();
    let mut keys: Vec<_> = out.into_iter().map(|e| e.key).collect();
    keys.sort();
    assert_eq!(keys, vec!["vendor/pkg/b.php", "vendor/pkg/src/A.php"]);
}
