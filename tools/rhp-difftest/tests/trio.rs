//! Differential regression gates for the gmp, mbstring and password extensions.
//! Each skips cleanly when no `php` binary is on PATH.

use rhp_difftest::sweeps;
use rhp_difftest::{php_available, Diff};

fn expect_match(name: &str, diff: Diff) {
    if let Some(why) = diff.failure() {
        panic!("[{name}] {why}");
    }
}

#[test]
fn gmp_matches_php() {
    if !php_available() {
        eprintln!("[gmp] no php — skipping");
        return;
    }
    // value-level only (object ids in var_dump differ: rhp doesn't recycle handles)
    for seed in [0xC0FFEE_u64, 7, 42] {
        expect_match("gmp", sweeps::gmp(seed, 80));
    }
}

/// libgmp's own results: the seeded Mersenne twister, primality, word
/// export and import.
#[test]
fn gmp_library_matches_php() {
    if !php_available() {
        eprintln!("[gmp] no php — skipping");
        return;
    }
    let src = br#"<?php
gmp_random_seed(42);
echo gmp_strval(gmp_random_bits(100)), " ", gmp_strval(gmp_random_range(0, "1000000000000000000000")), "\n";
gmp_random_seed(gmp_init("123456789012345678901234567890"));
echo gmp_strval(gmp_random_bits(65)), " ", gmp_strval(gmp_random_range(-5, 5)), "\n";
foreach (["97", "2462906046218231", "2462906046218233", "170141183460469231731687303715884105727", "-7"] as $n) {
    echo gmp_prob_prime($n), gmp_prob_prime($n, 1), " ";
}
echo "\n", gmp_strval(gmp_nextprime("1000000000000000000000")), "\n";
echo bin2hex(gmp_export("123456789012345678901234567890", 3, GMP_LSW_FIRST | GMP_BIG_ENDIAN)), "\n";
echo gmp_strval(gmp_import("\x01\x02\x03\x04", 2, GMP_MSW_FIRST | GMP_LITTLE_ENDIAN)), "\n";
echo serialize(gmp_init(-255)), " ", unserialize('C:3:"GMP":15:{s:2:"42";a:0:{}}'), "\n";
var_dump(gmp_perfect_power("-27"), gmp_kronecker(-3, 8), gmp_hamdist(-1, 1), gmp_popcount(-1), gmp_scan0(-1, 0));
echo gmp_strval(gmp_pow(3, 300), 62), "\n", (gmp_init(2) ** 100) >> 3, " ", pow(gmp_init(-3), 41), "\n";
"#;
    expect_match("gmp", rhp_difftest::diff_source(src));
}

/// Where libgmp would abort the process (php dies with SIGFPE), rhp throws.
#[test]
fn gmp_size_guards_throw() {
    let src = br#"<?php
$cases = [
    fn() => gmp_pow(2, PHP_INT_MAX),
    fn() => gmp_init(3) ** PHP_INT_MAX,
    fn() => gmp_init(1) << PHP_INT_MAX,
    fn() => gmp_fact(PHP_INT_MAX),
    fn() => gmp_binomial(gmp_pow(10, 30), PHP_INT_MAX),
];
foreach ($cases as $f) {
    try { $f(); echo "no error\n"; } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
}
"#;
    let out = rhp_difftest::run_rhp(src).expect("rhp runs");
    assert_eq!(
        String::from_utf8_lossy(&out),
        "gmp_pow(): Argument #2 ($exponent) is too large\nExponent is too large\n\
         Shift is too large\ngmp_fact(): Argument #1 ($num) is too large\n\
         gmp_binomial(): Argument #2 ($k) is too large\n"
    );
}

#[test]
fn mbstring_matches_php() {
    if !php_available() {
        eprintln!("[mbstring] no php — skipping");
        return;
    }
    for seed in [0xC0FFEE_u64, 7, 42] {
        expect_match("mbstring", sweeps::mbstring(seed, 100));
    }
}

#[test]
fn password_deterministic_surface_matches_php() {
    if !php_available() {
        eprintln!("[password] no php — skipping");
        return;
    }
    // password_algos / get_info / verify-known / needs_rehash are deterministic.
    expect_match("password", sweeps::password());
}

/// password_hash is random-salted, so prove the round-trip instead of byte
/// output: rhp's hash verifies in php, and php's hash verifies in rhp.
#[test]
fn password_cross_verifies_with_php() {
    if !php_available() {
        eprintln!("[password] no php — skipping");
        return;
    }
    use rhp_difftest::{run_php, run_rhp};

    // NB: hashes contain `$`, so they MUST sit in single-quoted PHP strings.
    for (pw, algo) in [
        ("hunter2", "PASSWORD_BCRYPT"),
        ("sukru-key", "PASSWORD_BCRYPT, ['cost' => 5]"),
        ("hunter2", "PASSWORD_ARGON2I"),
        (
            "sukru-key",
            "PASSWORD_ARGON2I, ['memory_cost' => 1024, 'time_cost' => 1]",
        ),
        ("hunter2", "PASSWORD_ARGON2ID"),
        (
            "sukru-key",
            "PASSWORD_ARGON2ID, ['memory_cost' => 2048, 'time_cost' => 2, 'threads' => 3]",
        ),
    ] {
        // rhp hashes -> php verifies
        let h = run_rhp(format!("<?php echo password_hash('{pw}', {algo});").as_bytes())
            .expect("rhp hash");
        let h = String::from_utf8_lossy(&h);
        let out = run_php(
            format!(
                "<?php var_dump(password_verify('{pw}','{h}'), password_verify('nope','{h}'));"
            )
            .as_bytes(),
        )
        .expect("php verify");
        assert_eq!(
            out, b"bool(true)\nbool(false)\n",
            "php could not verify rhp's hash for {pw:?}"
        );

        // php hashes -> rhp verifies
        let h = run_php(format!("<?php echo password_hash('{pw}', {algo});").as_bytes())
            .expect("php hash");
        let h = String::from_utf8_lossy(&h);
        let out = run_rhp(
            format!(
                "<?php var_dump(password_verify('{pw}','{h}'), password_verify('nope','{h}'));"
            )
            .as_bytes(),
        )
        .expect("rhp verify");
        assert_eq!(
            out, b"bool(true)\nbool(false)\n",
            "rhp could not verify php's hash for {pw:?}"
        );
    }
}
