//! Extension-specific differential sweeps built on the generic harness.
//!
//! A *sweep* generates a broad corpus of inputs for one extension's functions,
//! runs it through both engines, and reports a per-feature verdict. Today this
//! covers the `hash` extension; the same shape (generate → `diff_source` →
//! classify) is how any future reimplemented extension is driven.

use crate::{diff_source, php_bytes_array, php_bytes_literal, run_php, Diff};

/// Ask the real `php` which `hash()` algorithms it supports.
pub fn php_hash_algos() -> Result<Vec<String>, String> {
    let out = run_php(b"<?php foreach (hash_algos() as $a) echo $a, \"\\n\";")?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

/// PHP source that hashes every element of `corpus` with `algo` (raw output,
/// hex-encoded one per line). Identical text runs on both engines.
pub fn hash_algo_snippet(algo: &str, corpus: &[Vec<u8>]) -> String {
    // algo is a hash-algorithm name (ascii, may contain ',' or '/'): safe inside
    // a double-quoted PHP string with no escaping needed.
    format!(
        "<?php\n$in = {};\nforeach ($in as $d) echo bin2hex(hash(\"{}\", $d, true)), \"\\n\";\n",
        php_bytes_array(corpus),
        algo
    )
}

/// Run the `hash()` differential sweep for one algorithm over a corpus.
pub fn hash_algo(algo: &str, corpus: &[Vec<u8>]) -> Diff {
    diff_source(hash_algo_snippet(algo, corpus).as_bytes())
}

// ---------------------------------------------------------------------------
// bcmath
// ---------------------------------------------------------------------------

use crate::Rng;

/// A random bc-number string: optional sign, up to `int_max` integer digits and
/// up to `frac_max` fractional digits (sometimes an integer).
fn bc_number(rng: &mut Rng, int_max: usize, frac_max: usize) -> String {
    let neg = rng.below(2) == 0;
    let ni = 1 + rng.below(int_max);
    let nf = rng.below(frac_max + 1);
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    for k in 0..ni {
        // avoid a leading zero run that is the whole integer part being "0..."
        let d = if k == 0 {
            1 + rng.below(9)
        } else {
            rng.below(10)
        };
        s.push((b'0' + d as u8) as char);
    }
    if nf > 0 {
        s.push('.');
        for _ in 0..nf {
            s.push((b'0' + rng.below(10) as u8) as char);
        }
    }
    s
}

/// A PHP snippet exercising the two-operand bcmath ops over generated operands
/// and scales, plus the unary ops. Deterministic from `seed`.
pub fn bcmath_snippet(seed: u64, count: usize) -> String {
    let mut rng = Rng::new(seed);
    let mut lines = String::from("<?php\n");
    for _ in 0..count {
        let a = bc_number(&mut rng, 12, 6);
        let b = bc_number(&mut rng, 8, 6);
        let scale = rng.below(10);
        // guard divide/mod by zero (php throws; we test those separately)
        let b_nonzero = if b.trim_start_matches(['-', '0', '.']).is_empty() {
            "1".to_string()
        } else {
            b.clone()
        };
        lines.push_str(&format!("echo bcadd(\"{a}\",\"{b}\",{scale}),\"|\";\n"));
        lines.push_str(&format!("echo bcsub(\"{a}\",\"{b}\",{scale}),\"|\";\n"));
        lines.push_str(&format!("echo bcmul(\"{a}\",\"{b}\",{scale}),\"|\";\n"));
        lines.push_str(&format!(
            "echo bcdiv(\"{a}\",\"{b_nonzero}\",{scale}),\"|\";\n"
        ));
        lines.push_str(&format!(
            "echo bcmod(\"{a}\",\"{b_nonzero}\",{scale}),\"|\";\n"
        ));
        lines.push_str(&format!("echo bccomp(\"{a}\",\"{b}\",{scale}),\"|\";\n"));
        lines.push_str(&format!(
            "echo implode(\",\",bcdivmod(\"{a}\",\"{b_nonzero}\",{scale})),\"|\";\n"
        ));
        lines.push_str(&format!("echo bcfloor(\"{a}\"),\"|\";\n"));
        lines.push_str(&format!("echo bcceil(\"{a}\"),\"|\";\n"));
        lines.push_str(&format!("echo bcround(\"{a}\",{}),\"|\";\n", rng.below(6)));
        // small non-negative exponent for bcpow; non-negative base for bcsqrt
        let e = rng.below(12);
        lines.push_str(&format!("echo bcpow(\"{a}\",\"{e}\",{scale}),\"|\";\n"));
        let pos = a.trim_start_matches('-').to_string();
        lines.push_str(&format!("echo bcsqrt(\"{pos}\",{scale}),\"|\";\n"));
        lines.push_str("echo \"\\n\";\n");
    }
    lines
}

/// Run the whole bcmath differential sweep as one snippet.
pub fn bcmath(seed: u64, count: usize) -> Diff {
    diff_source(bcmath_snippet(seed, count).as_bytes())
}

// ---------------------------------------------------------------------------
// ctype
// ---------------------------------------------------------------------------

/// The 11 `ctype_*` functions.
pub const CTYPE_FUNCS: &[&str] = &[
    "ctype_alnum",
    "ctype_alpha",
    "ctype_cntrl",
    "ctype_digit",
    "ctype_graph",
    "ctype_lower",
    "ctype_print",
    "ctype_punct",
    "ctype_space",
    "ctype_upper",
    "ctype_xdigit",
];

/// PHP snippet that runs every `ctype_*` over every single byte (0..=255), every
/// integer argument quirk, and a few multi-char strings.
pub fn ctype_snippet() -> String {
    let mut s = String::from("<?php\n$fns = [");
    for f in CTYPE_FUNCS {
        s.push_str(&format!("\"{f}\","));
    }
    s.push_str("];\n");
    // single bytes, integer args (the -128..=256 char-code quirk), strings
    s.push_str(
        "foreach ($fns as $f) {\n\
           for ($i = 0; $i < 256; $i++) echo (int)$f(chr($i));\n\
           for ($n = -130; $n <= 260; $n++) echo (int)$f($n);\n\
           foreach ([\"\", \"abc\", \"ABC123\", \"  \\t\", \"a1!\", \"\\xe9\\xff\"] as $str) echo (int)$f($str);\n\
           echo \"\\n\";\n\
         }\n",
    );
    s
}

/// Run the ctype differential sweep.
pub fn ctype() -> Diff {
    diff_source(ctype_snippet().as_bytes())
}

// ---------------------------------------------------------------------------
// gmp (value-level; object ids in var_dump are intentionally not tested —
// rhp does not recycle object handles like php does)
// ---------------------------------------------------------------------------

/// A random big-integer decimal string (optional sign, up to `digits` digits).
fn gmp_int(rng: &mut Rng, digits: usize) -> String {
    let neg = rng.below(4) == 0;
    let n = 1 + rng.below(digits);
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    for k in 0..n {
        let d = if k == 0 {
            1 + rng.below(9)
        } else {
            rng.below(10)
        };
        s.push((b'0' + d as u8) as char);
    }
    s
}

/// PHP snippet over generated big integers, printing gmp results via gmp_strval
/// / gmp_cmp / gmp_sign (never var_dump — object ids differ by design).
pub fn gmp_snippet(seed: u64, count: usize) -> String {
    let mut rng = Rng::new(seed);
    let mut s = String::from("<?php\n");
    for _ in 0..count {
        let a = gmp_int(&mut rng, 40);
        let b = gmp_int(&mut rng, 20);
        let bz = if b.trim_start_matches('-') == "0" {
            "1".to_string()
        } else {
            b.clone()
        };
        let e = rng.below(16);
        s.push_str(&format!(
            "echo gmp_strval(gmp_add(\"{a}\",\"{b}\")),\"|\";\n"
        ));
        s.push_str(&format!(
            "echo gmp_strval(gmp_sub(\"{a}\",\"{b}\")),\"|\";\n"
        ));
        s.push_str(&format!(
            "echo gmp_strval(gmp_mul(\"{a}\",\"{b}\")),\"|\";\n"
        ));
        s.push_str(&format!(
            "echo gmp_strval(gmp_div_q(\"{a}\",\"{bz}\")),\"|\";\n"
        ));
        s.push_str(&format!(
            "echo gmp_strval(gmp_div_r(\"{a}\",\"{bz}\")),\"|\";\n"
        ));
        s.push_str(&format!(
            "echo gmp_strval(gmp_mod(\"{a}\",\"{bz}\")),\"|\";\n"
        ));
        s.push_str(&format!("echo gmp_strval(gmp_pow(\"{b}\",{e})),\"|\";\n"));
        s.push_str(&format!(
            "echo gmp_strval(gmp_gcd(\"{a}\",\"{b}\")),\"|\";\n"
        ));
        s.push_str(&format!("echo gmp_strval(gmp_abs(\"{a}\")),\"|\";\n"));
        s.push_str(&format!("echo gmp_strval(gmp_neg(\"{a}\")),\"|\";\n"));
        s.push_str(&format!(
            "echo gmp_strval(gmp_and(\"{a}\",\"{b}\")),\"|\";\n"
        ));
        s.push_str(&format!(
            "echo gmp_strval(gmp_or(\"{a}\",\"{b}\")),\"|\";\n"
        ));
        s.push_str(&format!(
            "echo gmp_strval(gmp_xor(\"{a}\",\"{b}\")),\"|\";\n"
        ));
        s.push_str(&format!("echo gmp_cmp(\"{a}\",\"{b}\"),gmp_sign(\"{a}\"),gmp_popcount(gmp_abs(\"{a}\")),\"|\";\n"));
        s.push_str(&format!(
            "echo gmp_strval(gmp_add(\"{a}\",\"{b}\"),16),\"|\";\n"
        ));
        s.push_str("echo \"\\n\";\n");
    }
    s
}

/// Run the gmp differential sweep.
pub fn gmp(seed: u64, count: usize) -> Diff {
    diff_source(gmp_snippet(seed, count).as_bytes())
}

// ---------------------------------------------------------------------------
// mbstring (UTF-8 value-level)
// ---------------------------------------------------------------------------

/// A random UTF-8 string mixing ASCII, Latin-1 accents, CJK and astral emoji.
fn utf8_string(rng: &mut Rng) -> String {
    const POOL: &[char] = &[
        'a', 'Z', '0', ' ', '!', 'é', 'ß', 'Ü', 'ñ', 'Ω', 'Я', '日', '本', '語', '✓', '€', '😀',
        '🎉', '𝔘',
    ];
    let n = rng.below(12);
    (0..n).map(|_| POOL[rng.below(POOL.len())]).collect()
}

/// PHP snippet exercising the common mb_* functions over generated UTF-8 strings.
pub fn mbstring_snippet(seed: u64, count: usize) -> String {
    let mut rng = Rng::new(seed);
    let mut s = String::from("<?php\n");
    for _ in 0..count {
        let a = utf8_string(&mut rng);
        let needle = utf8_string(&mut rng);
        let start = rng.below(6);
        let len = rng.below(6);
        // embed as \xNN escapes so the source is pure-ascii and binary-safe
        let la = php_bytes_literal(a.as_bytes());
        let ln = php_bytes_literal(needle.as_bytes());
        s.push_str(&format!("echo mb_strlen({la}),\"|\";\n"));
        s.push_str(&format!("echo mb_substr({la},{start},{len}),\"|\";\n"));
        s.push_str(&format!("echo mb_strtoupper({la}),\"|\";\n"));
        s.push_str(&format!("echo mb_strtolower({la}),\"|\";\n"));
        s.push_str(&format!(
            "echo mb_convert_case({la}, MB_CASE_TITLE),\"|\";\n"
        ));
        s.push_str(&format!("echo implode(\",\", mb_str_split({la})),\"|\";\n"));
        s.push_str(&format!(
            "echo var_export(mb_strpos({la},{ln}),true),\"|\";\n"
        ));
        s.push_str(&format!("echo mb_strwidth({la}),\"|\";\n"));
        // mb_ord throws on an empty string in php 8; guard so the snippet never fatals
        s.push_str(&format!(
            "echo (mb_strlen({la})>0 ? var_export(mb_ord({la}),true) : \"-\"),\"|\";\n"
        ));
        s.push_str("echo \"\\n\";\n");
    }
    s
}

/// Run the mbstring differential sweep.
pub fn mbstring(seed: u64, count: usize) -> Diff {
    diff_source(mbstring_snippet(seed, count).as_bytes())
}

// ---------------------------------------------------------------------------
// password (deterministic surface; hashing itself is random-salted so it is
// cross-verified separately, not byte-compared)
// ---------------------------------------------------------------------------

/// PHP snippet over the deterministic password_* surface: algos, get_info and
/// verify over fixed known hashes, needs_rehash.
pub fn password_snippet() -> String {
    let known = [
        // (password, bcrypt $2y$ hash of it)
        (
            "$2y$10$N9qo8uLOickgx2ZMRZoMye1VdLk3fsv36cQ.Rl4bQ2N0J1x8i9Ym",
            "secret",
        ),
        (
            "$2y$12$GhvMmNVjRW29ulnudl.LbuAnUtN/LRfe1oxpvczonrdgw2fFbMWDG",
            "password",
        ),
    ];
    let mut s = String::from("<?php\n");
    s.push_str("echo implode(\",\", password_algos()), \"\\n\";\n");
    for (h, pw) in known {
        s.push_str(&format!("print_r(password_get_info(\"{h}\"));\n"));
        s.push_str(&format!(
            "var_dump(password_verify(\"{pw}\",\"{h}\"), password_verify(\"wrong\",\"{h}\"));\n"
        ));
        s.push_str(&format!(
            "var_dump(password_needs_rehash(\"{h}\", PASSWORD_BCRYPT, [\"cost\"=>10]));\n"
        ));
    }
    s
}

/// Run the deterministic password_* differential sweep.
pub fn password() -> Diff {
    diff_source(password_snippet().as_bytes())
}
