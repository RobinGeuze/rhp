use super::*;

fn all(src: &str) -> Vec<String> {
    let n = count(src.as_bytes());
    mutants(src.as_bytes(), &mut Rng::new(1), n)
        .into_iter()
        .map(|m| String::from_utf8(m.source).unwrap())
        .collect()
}

#[test]
fn mutates_each_literal_and_operator_once_per_replacement() {
    let ms = all("<?php echo 1 + 2;");
    assert!(ms.contains(&"<?php echo 1 - 2;".to_string()));
    assert!(ms.contains(&"<?php echo PHP_INT_MAX + 2;".to_string()));
    assert!(ms.contains(&"<?php echo 1 + 3;".to_string())); // n + 1
    assert!(ms.contains(&"<?php echo (-1) + 2;".to_string())); // parenthesised
    assert!(ms.iter().all(|m| m != "<?php echo 1 + 2;")); // never the identity
}

#[test]
fn leaves_comments_strings_with_interpolation_and_html_alone() {
    let src = "<html>1 + 2</html><?php // 1 + 2\n$x = \"a $y + 1\"; /* < */";
    for m in all(src) {
        assert!(m.starts_with("<html>1 + 2</html><?php // 1 + 2\n"), "{m}");
        assert!(m.contains("/* < */"), "{m}");
    }
}

#[test]
fn skips_declare_and_jump_literals() {
    let ms = all("<?php declare(strict_types=1); while (1) { break 2; }");
    assert!(ms.iter().all(|m| m.contains("declare(strict_types=1)")));
    assert!(ms.iter().all(|m| m.contains("break 2;")));
    // ...but the `while (1)` literal is still mutated.
    assert!(ms.iter().any(|m| m.contains("while (0)")));
}

#[test]
fn deterministic_and_bounded() {
    let src = b"<?php $a = 3 < 4 && 'x' == \"y\"; echo $a ? 1.5 : null;";
    let a: Vec<_> = mutants(src, &mut Rng::new(9), 5)
        .into_iter()
        .map(|m| m.description)
        .collect();
    let b: Vec<_> = mutants(src, &mut Rng::new(9), 5)
        .into_iter()
        .map(|m| m.description)
        .collect();
    assert_eq!(a.len(), 5);
    assert_eq!(a, b);
}

#[test]
fn neighbours_only_for_decimal() {
    assert_eq!(int_neighbours(b"10"), vec!["11", "9"]);
    assert_eq!(int_neighbours(b"0"), vec!["1", "(-1)"]);
    assert_eq!(int_neighbours(b"1_000"), vec!["1001", "999"]);
    assert!(int_neighbours(b"0x1F").is_empty());
    assert!(int_neighbours(b"017").is_empty());
}
