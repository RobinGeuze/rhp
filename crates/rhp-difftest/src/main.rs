//! `rhp-difftest` — run an extension's differential sweep against the real
//! `php` binary and print a per-feature verdict.
//!
//! Usage:
//!   rhp-difftest [hash]            # sweep the hash extension (default)
//!   rhp-difftest hash --seed 7 --count 64
//!   rhp-difftest hash --algo tiger128,3     # one algorithm
//!   rhp-difftest hash --strict              # unsupported algos count as failures
//!
//! Exit code is nonzero if any sweep produced a MISMATCH (or, with --strict,
//! any rhp-unsupported feature).

use std::process::ExitCode;

use rhp_difftest::sweeps::{hash_algo, php_hash_algos};
use rhp_difftest::{default_byte_corpus, php_available, Diff};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ext = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "hash".to_string());
    let seed = flag_val(&args, "--seed").and_then(|v| v.parse().ok()).unwrap_or(0xC0FFEE);
    let count = flag_val(&args, "--count").and_then(|v| v.parse().ok()).unwrap_or(48);
    let only = flag_val(&args, "--algo");
    let strict = args.iter().any(|a| a == "--strict");

    if args.iter().any(|a| a == "--dump") {
        use rhp_difftest::sweeps;
        let src = match ext.as_str() {
            "gmp" => sweeps::gmp_snippet(seed, count),
            "mbstring" => sweeps::mbstring_snippet(seed, count),
            "password" => sweeps::password_snippet(),
            "bcmath" => sweeps::bcmath_snippet(seed, count),
            "ctype" => sweeps::ctype_snippet(),
            _ => String::new(),
        };
        print!("{src}");
        return ExitCode::SUCCESS;
    }

    if !php_available() {
        eprintln!("rhp-difftest: no `php` binary on PATH — cannot run a differential sweep.");
        eprintln!("(set RHP_PHP to point at one, or install php 8.5.x)");
        return ExitCode::from(2);
    }

    match ext.as_str() {
        "hash" => sweep_hash(seed, count, only.as_deref(), strict),
        "bcmath" => sweep_bcmath(seed, count),
        "ctype" => sweep_ctype(),
        "gmp" => simple_sweep("gmp", rhp_difftest::sweeps::gmp(seed, count)),
        "mbstring" => simple_sweep("mbstring", rhp_difftest::sweeps::mbstring(seed, count)),
        "password" => simple_sweep("password", rhp_difftest::sweeps::password()),
        other => {
            eprintln!(
                "rhp-difftest: unknown extension {other:?} (known: hash, bcmath, ctype, gmp, mbstring, password)"
            );
            ExitCode::from(2)
        }
    }
}

fn sweep_bcmath(seed: u64, count: usize) -> ExitCode {
    use rhp_difftest::sweeps::bcmath;
    println!("bcmath sweep — {count} operand rows, seed {seed:#x}\n");
    match bcmath(seed, count) {
        Diff::Match => {
            println!("  MATCH  all bcmath operations byte-identical");
            ExitCode::SUCCESS
        }
        Diff::Mismatch { php, rhp } => {
            println!("  MISMATCH");
            let (p, q) = (String::from_utf8_lossy(&php), String::from_utf8_lossy(&rhp));
            for (i, (a, b)) in p.lines().zip(q.lines()).enumerate() {
                if a != b {
                    println!("    row {i}:\n      php   : {a}\n      rhp: {b}");
                    break;
                }
            }
            ExitCode::FAILURE
        }
        Diff::PhrustErr(e) => {
            println!("  rhp error: {}", first_line(&e));
            ExitCode::FAILURE
        }
        Diff::PhpErr(e) => {
            println!("  php error: {}", first_line(&e));
            ExitCode::from(2)
        }
    }
}

fn sweep_hash(seed: u64, count: usize, only: Option<&str>, strict: bool) -> ExitCode {
    let algos = match php_hash_algos() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("rhp-difftest: could not query php hash_algos(): {e}");
            return ExitCode::from(2);
        }
    };
    let corpus = default_byte_corpus(seed, count);
    println!(
        "hash sweep — {} php algorithm(s), {} inputs/algo, seed {:#x}\n",
        algos.len(),
        corpus.len(),
        seed
    );

    let (mut matched, mut mismatched, mut unsupported, mut errored) = (0u32, 0u32, 0u32, 0u32);
    for algo in &algos {
        if let Some(f) = only {
            if algo != f {
                continue;
            }
        }
        match hash_algo(algo, &corpus) {
            Diff::Match => {
                matched += 1;
                println!("  MATCH        {algo}");
            }
            Diff::Mismatch { php, rhp } => {
                mismatched += 1;
                println!("  MISMATCH     {algo}");
                print_first_diff(&php, &rhp);
            }
            Diff::PhrustErr(e) => {
                unsupported += 1;
                println!("  unsupported  {algo}   ({})", first_line(&e));
            }
            Diff::PhpErr(e) => {
                errored += 1;
                println!("  PHP-ERROR    {algo}   ({})", first_line(&e));
            }
        }
    }

    println!(
        "\n{matched} match, {mismatched} mismatch, {unsupported} unsupported, {errored} php-error"
    );
    let bad = mismatched + errored + if strict { unsupported } else { 0 };
    if bad == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Generic single-snippet sweep reporter for extensions with one combined diff.
fn simple_sweep(name: &str, diff: Diff) -> ExitCode {
    println!("{name} sweep\n");
    match diff {
        Diff::Match => {
            println!("  MATCH  all {name} output byte-identical");
            ExitCode::SUCCESS
        }
        Diff::Mismatch { php, rhp } => {
            println!("  MISMATCH");
            print_first_diff(&php, &rhp);
            ExitCode::FAILURE
        }
        Diff::PhrustErr(e) => {
            println!("  rhp error: {}", first_line(&e));
            ExitCode::FAILURE
        }
        Diff::PhpErr(e) => {
            println!("  php error: {}", first_line(&e));
            ExitCode::from(2)
        }
    }
}

fn sweep_ctype() -> ExitCode {
    use rhp_difftest::sweeps::ctype;
    println!("ctype sweep — all ctype_* over every byte, int quirk, and strings\n");
    match ctype() {
        Diff::Match => {
            println!("  MATCH  all ctype_* byte-identical");
            ExitCode::SUCCESS
        }
        Diff::Mismatch { php, rhp } => {
            println!("  MISMATCH");
            print_first_diff(&php, &rhp);
            ExitCode::FAILURE
        }
        Diff::PhrustErr(e) => {
            println!("  rhp error: {}", first_line(&e));
            ExitCode::FAILURE
        }
        Diff::PhpErr(e) => {
            println!("  php error: {}", first_line(&e));
            ExitCode::from(2)
        }
    }
}

fn print_first_diff(php: &[u8], rhp: &[u8]) {
    let php_s = String::from_utf8_lossy(php);
    let rhp_s = String::from_utf8_lossy(rhp);
    for (i, (a, b)) in php_s.lines().zip(rhp_s.lines()).enumerate() {
        if a != b {
            println!("      input #{i}: php={a}  rhp={b}");
            return;
        }
    }
    println!("      (length differs: php {} B, rhp {} B)", php.len(), rhp.len());
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

fn flag_val(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1).cloned())
}
