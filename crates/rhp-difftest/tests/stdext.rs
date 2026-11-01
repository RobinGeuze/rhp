//! Differential sweeps for the tooling-facing stdlib: the seeded Mt19937
//! engine (mt_rand / rand / shuffle / array_rand / str_shuffle), the
//! array_multisort / array_change_key_case family, fnmatch and SPL heaps.
//! Each generates PHP from a deterministic RNG, runs it through `php -n` and
//! the rhp VM, and requires identical output. Skips cleanly without `php`.

use rhp_difftest::{diff_source, php_available, Diff, Rng};

fn expect_match(name: &str, src: &str) {
    match diff_source(src.as_bytes()) {
        Diff::Match => {}
        Diff::Mismatch { php, rhp } => {
            let (p, q) = (String::from_utf8_lossy(&php), String::from_utf8_lossy(&rhp));
            let first = p
                .lines()
                .zip(q.lines())
                .find(|(a, b)| a != b)
                .map(|(a, b)| format!("php={a} rhp={b}"))
                .unwrap_or_else(|| "length differs".to_string());
            panic!("[{name}] MISMATCH: {first}\n--- source ---\n{src}");
        }
        Diff::PhrustErr(e) => panic!("[{name}] rhp error: {e}\n{src}"),
        Diff::PhpErr(e) => panic!("[{name}] php error: {e}\n{src}"),
    }
}

/// A PHP array literal of small random scalars (ints, numeric and plain strings).
fn scalar_array(rng: &mut Rng, n: usize) -> String {
    let items: Vec<String> = (0..n)
        .map(|_| match rng.below(4) {
            0 => format!("{}", rng.below(20) as i64 - 5),
            1 => format!("'{}'", rng.below(30)),
            2 => format!(
                "'{}'",
                ["a", "B", "c", "aa", "Zed", "10", "9"][rng.below(7)]
            ),
            _ => format!("{}.5", rng.below(9)),
        })
        .collect();
    format!("[{}]", items.join(", "))
}

#[test]
fn mt_engine_matches_php() {
    if !php_available() {
        eprintln!("[stdext] no php — skipping");
        return;
    }
    let mut src = String::from("<?php\n");
    for seed in [0i64, 1, 2, 42, 1234, -7, 2147483647, 4294967296 + 5] {
        src.push_str(&format!(
            "mt_srand({seed}); echo mt_rand(), ' ', mt_rand(0, 9), ' ', mt_rand(-1000, 1000), ' ', rand(50, 10), ' ', mt_rand(0, PHP_INT_MAX), ' ', mt_rand(PHP_INT_MIN, PHP_INT_MAX), \"\\n\";\n\
             mt_srand({seed}); $a = range(1, 12); shuffle($a); echo implode(',', $a), ' ', str_shuffle('abcdefghij'), ' ', json_encode(array_rand(array_flip(range(10, 30)), 5)), ' ', json_encode(array_rand(['x' => 1, 'y' => 2, 'z' => 3])), \"\\n\";\n"
        ));
    }
    expect_match("mt", &src);
}

#[test]
fn array_multisort_matches_php() {
    if !php_available() {
        eprintln!("[stdext] no php — skipping");
        return;
    }
    let mut rng = Rng::new(0x5EED);
    let flags = [
        "SORT_REGULAR",
        "SORT_NUMERIC",
        "SORT_STRING",
        "SORT_NATURAL",
        "SORT_STRING | SORT_FLAG_CASE",
    ];
    let mut src = String::from("<?php\n");
    for _ in 0..40 {
        let n = 1 + rng.below(7);
        let a = scalar_array(&mut rng, n);
        let b = scalar_array(&mut rng, n);
        let order = if rng.below(2) == 0 {
            "SORT_ASC"
        } else {
            "SORT_DESC"
        };
        let flag = flags[rng.below(flags.len())];
        src.push_str(&format!(
            "$a = {a}; $b = {b}; array_multisort($a, {order}, {flag}, $b); echo json_encode([$a, $b]), \"\\n\";\n\
             echo json_encode(array_change_key_case(array_flip(array_map('strval', {a})), CASE_UPPER)), \"\\n\";\n"
        ));
    }
    expect_match("multisort", &src);
}

#[test]
fn fnmatch_matches_php() {
    if !php_available() {
        eprintln!("[stdext] no php — skipping");
        return;
    }
    let patterns = [
        "*",
        "*.php",
        "a?c",
        "[a-c]*",
        "[!a-c]*",
        "src/*",
        "src/*/x.php",
        "*/x.php",
        "\\*",
        ".*",
        "*[0-9]",
        "a*b*c",
        "",
    ];
    let names = [
        "a.php",
        "abc",
        "src/a/x.php",
        "src/x.php",
        ".hidden",
        "b9",
        "*",
        "aXbYc",
        "",
        "dir/.x.php",
    ];
    let mut src = String::from("<?php\n");
    for p in patterns {
        for n in names {
            src.push_str(&format!(
                "echo (int) fnmatch('{p}', '{n}'), (int) fnmatch('{p}', '{n}', FNM_PATHNAME), (int) fnmatch('{p}', '{n}', FNM_PERIOD), (int) fnmatch('{p}', '{n}', FNM_PATHNAME | FNM_PERIOD | FNM_NOESCAPE), ' ';\n",
                p = p.replace('\\', "\\\\")
            ));
        }
        src.push_str("echo \"\\n\";\n");
    }
    expect_match("fnmatch", &src);
}

#[test]
fn spl_heaps_match_php() {
    if !php_available() {
        eprintln!("[stdext] no php — skipping");
        return;
    }
    let mut rng = Rng::new(0xBEEF);
    let mut src = String::from("<?php\n");
    for _ in 0..30 {
        let n = rng.below(12);
        let vals: Vec<String> = (0..n).map(|_| format!("{}", rng.below(6))).collect();
        let prios: Vec<String> = (0..n).map(|_| format!("{}", rng.below(4))).collect();
        src.push_str(&format!(
            "$v = [{v}]; $p = [{p}];\n\
             $h = new SplMinHeap; foreach ($v as $i => $x) $h->insert([$x, $i]); $o = []; foreach ($h as $e) $o[] = implode(':', $e); echo implode(' ', $o), \"\\n\";\n\
             $h = new SplMaxHeap; foreach ($v as $i => $x) $h->insert([$x, $i]); $o = []; while ($h->valid()) {{ $o[] = implode(':', $h->extract()); }} echo implode(' ', $o), \"\\n\";\n\
             $q = new SplPriorityQueue; foreach ($v as $i => $x) $q->insert(\"$x/$i\", $p[$i]); $o = []; while (!$q->isEmpty()) $o[] = $q->extract(); echo implode(' ', $o), \"\\n\";\n",
            v = vals.join(", "),
            p = prios.join(", ")
        ));
    }
    expect_match("heaps", &src);
}
