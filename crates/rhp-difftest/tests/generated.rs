//! The tables `rhp-difftest gen dumps` makes from the reference php are up
//! to date. Off the reference host (Homebrew php on macOS, rhp's release) the
//! generator skips. CI checks the php-src and tarball generators in ci.yml.

#[test]
fn dumps_match_the_reference_php() {
    let args = ["dumps".to_string(), "--check".to_string()];
    assert_eq!(
        rhp_difftest::gen::run(&args),
        Ok(true),
        "run `rhp-difftest gen dumps` on the reference host"
    );
}
