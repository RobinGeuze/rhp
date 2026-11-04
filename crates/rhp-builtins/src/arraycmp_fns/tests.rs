use super::*;

#[test]
fn zend_sort_matches_std_order() {
    for n in [0usize, 1, 2, 3, 5, 6, 7, 16, 17, 40, 1100] {
        let mut v: Vec<i64> = (0..n as i64).map(|i| (i * 7919) % 101).collect();
        let mut want = v.clone();
        want.sort();
        zend_sort(&mut v, &mut |a, b| (*a - *b).signum() as i32);
        assert_eq!(v, want, "n={n}");
    }
}
