//! How a `Diff` reports where php and rhp part ways (no php needed).

use rhp_difftest::{Diff, FirstDifference};

fn mismatch(php: &str, rhp: &str) -> Diff {
    Diff::Mismatch {
        php: php.into(),
        rhp: rhp.into(),
    }
}

#[test]
fn first_difference_names_the_row() {
    let d = mismatch("a\nb\nc\n", "a\nB\nc\n");
    assert_eq!(
        d.first_difference(),
        Some(FirstDifference::Row {
            index: 1,
            php: "b".into(),
            rhp: "B".into(),
        })
    );
    assert_eq!(d.failure().unwrap(), "MISMATCH: row 1: php=b rhp=B");
}

#[test]
fn first_difference_falls_back_to_length() {
    let d = mismatch("a\nb\n", "a\n");
    assert_eq!(
        d.first_difference(),
        Some(FirstDifference::Length { php: 4, rhp: 2 })
    );
    assert_eq!(
        d.failure().unwrap(),
        "MISMATCH: length differs (php 4 B, rhp 2 B)"
    );
}

#[test]
fn failure_covers_every_outcome() {
    assert_eq!(Diff::Match.failure(), None);
    assert_eq!(Diff::Match.first_difference(), None);
    assert_eq!(
        Diff::RhpErr("rhp: panicked".into()).failure().unwrap(),
        "rhp error: rhp: panicked"
    );
    assert_eq!(
        Diff::PhpErr("php exited 255".into()).failure().unwrap(),
        "php error: php exited 255"
    );
}
