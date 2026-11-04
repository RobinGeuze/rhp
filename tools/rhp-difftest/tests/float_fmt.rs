//! Shortest round-trip float output (serialize_precision -1) against php:
//! var_export, json_encode, serialize and var_dump over random-bit doubles,
//! exact decimal ties, powers of two with their neighbours and subnormals.
//! Skips without `php`.

use rhp_difftest::{diff_source, php_available, Rng};

/// Doubles whose shortest digits are hard to get right.
fn corpus() -> Vec<f64> {
    let mut rng = Rng::new(0xF10A7);
    let mut v = Vec::new();
    // Random bit patterns.
    while v.len() < 6000 {
        let f = f64::from_bits(rng.next_u64());
        if f.is_finite() {
            v.push(f);
        }
    }
    // n + k/8 between 2^46 and 2^56: shortest digits end in an exact tie.
    for e in 46..=56 {
        for _ in 0..60 {
            let n = (1u64 << e) + rng.next_u64() % (1u64 << e);
            for k in [1, 3, 5, 7] {
                v.push(n as f64 + k as f64 / 8.0);
            }
        }
    }
    // Every power of two and its neighbours, built from bits: powi gives 0.0
    // below 2^-1023.
    for e in -1074i32..=1023 {
        let p = if e >= -1022 {
            f64::from_bits(((e + 1023) as u64) << 52)
        } else {
            f64::from_bits(1u64 << (e + 1074))
        };
        v.push(p);
        v.push(f64::from_bits(p.to_bits() + 1));
        if p.to_bits() > 1 {
            v.push(f64::from_bits(p.to_bits() - 1));
        }
    }
    // Small subnormals and k * 2^-m.
    for i in 1..300u64 {
        v.push(f64::from_bits(i));
    }
    for m in 1..60 {
        for k in [1, 3, 5, 7, 9, 11, 13, 15] {
            v.push(k as f64 / 2f64.powi(m));
        }
    }
    // Large integers near 2^53 and above.
    for _ in 0..500 {
        v.push((rng.next_u64() >> rng.below(12)) as f64);
    }
    v
}

#[test]
fn shortest_float_output_matches_php() {
    if !php_available() {
        eprintln!("[float_fmt] no php — skipping");
        return;
    }
    let values = corpus();
    for chunk in values.chunks(2500) {
        // 17 significant digits read back as the same double in both engines.
        let lits: Vec<String> = chunk.iter().map(|f| format!("{f:.16e}")).collect();
        let src = format!(
            "<?php\nforeach ([{}] as $f) {{ echo var_export($f, true), ' ', json_encode($f), ' ', serialize($f), ' '; var_dump($f); }}\n",
            lits.join(",")
        );
        if let Some(why) = diff_source(src.as_bytes()).failure() {
            panic!("[float_fmt] {why}");
        }
    }
}
