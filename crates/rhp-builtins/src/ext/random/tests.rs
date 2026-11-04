use super::*;

#[test]
fn mt_matches_php_seed_1() {
    // php: mt_srand(1); mt_rand() === 895547922
    let mut mt = Mt::seeded(1, MT_RAND_MT19937);
    assert_eq!(mt.next32() >> 1, 895547922);
}
