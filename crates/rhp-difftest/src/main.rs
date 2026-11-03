//! `rhp-difftest` — run an extension's differential sweep against the real
//! `php` binary and print a per-feature verdict.
//!
//! Usage:
//!   rhp-difftest [hash]            # sweep the hash extension (default)
//!   rhp-difftest hash --seed 7 --count 64
//!   rhp-difftest hash --algo tiger128,3     # one algorithm
//!   rhp-difftest hash --strict              # unsupported algos count as failures
//!   rhp-difftest phpt --php-src ../php-src  # php-src .phpt corpus (see `phpt --help`)
//!   rhp-difftest phpt --php-src ../php-src --filter Zend/tests --mutate 20
//!   rhp-difftest gen parser --php-dist php-8.5.12 --check  # generated tables (see gen/)
//!   rhp-difftest lint vendor --check-known known-refused.txt  # compile-only scan
//!   rhp-difftest app symfony-7.4 --check-known tests/apps/symfony-7.4/known-passing-requests.txt
//!
//! Exit code is nonzero if any sweep produced a MISMATCH (or, with --strict,
//! any rhp-unsupported feature; for `phpt --check-known`, any regression;
//! for `lint --check-known`, a refused file not on the list).

use std::process::ExitCode;

use rhp_difftest::sweeps::{self, hash_algo, php_hash_algos};
use rhp_difftest::{default_byte_corpus, php_available, Diff};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("phpt") {
        return rhp_difftest::phpt::main(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("app") {
        return rhp_difftest::app::main(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("lint") {
        return rhp_difftest::lint::main(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("gen") {
        return rhp_difftest::gen::main(&args[1..]);
    }
    let ext = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "hash".to_string());
    let seed = flag_val(&args, "--seed")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0xC0FFEE);
    let count = flag_val(&args, "--count")
        .and_then(|v| v.parse().ok())
        .unwrap_or(48);
    let only = flag_val(&args, "--algo");
    let strict = args.iter().any(|a| a == "--strict");

    if args.iter().any(|a| a == "--dump") {
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
        "bcmath" => report(
            &format!("bcmath sweep — {count} operand rows, seed {seed:#x}"),
            "all bcmath operations byte-identical",
            sweeps::bcmath(seed, count),
        ),
        "ctype" => report(
            "ctype sweep — all ctype_* over every byte, int quirk, and strings",
            "all ctype_* byte-identical",
            sweeps::ctype(),
        ),
        "gmp" => simple_sweep("gmp", sweeps::gmp(seed, count)),
        "mbstring" => simple_sweep("mbstring", sweeps::mbstring(seed, count)),
        "password" => simple_sweep("password", sweeps::password()),
        other => {
            eprintln!(
                "rhp-difftest: unknown extension {other:?} (known: hash, bcmath, ctype, gmp, mbstring, password)"
            );
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
        let diff = hash_algo(algo, &corpus);
        match &diff {
            Diff::Match => {
                matched += 1;
                println!("  MATCH        {algo}");
            }
            Diff::Mismatch { .. } => {
                mismatched += 1;
                println!("  MISMATCH     {algo}");
                if let Some(d) = diff.first_difference() {
                    println!("      {d}");
                }
            }
            Diff::RhpErr(e) => {
                unsupported += 1;
                println!("  unsupported  {algo}   ({})", first_line(e));
            }
            Diff::PhpErr(e) => {
                errored += 1;
                println!("  PHP-ERROR    {algo}   ({})", first_line(e));
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

/// Single-snippet sweep reporter for extensions with one combined diff.
fn simple_sweep(name: &str, diff: Diff) -> ExitCode {
    report(
        &format!("{name} sweep"),
        &format!("all {name} output byte-identical"),
        diff,
    )
}

/// Print a one-snippet sweep's verdict: MATCH, or the first thing that went
/// wrong. A php failure (a bad corpus, not a rhp bug) exits 2.
fn report(header: &str, matched: &str, diff: Diff) -> ExitCode {
    println!("{header}\n");
    match diff.failure() {
        None => {
            println!("  MATCH  {matched}");
            ExitCode::SUCCESS
        }
        Some(why) => {
            println!("  {}", first_line(&why));
            if matches!(diff, Diff::PhpErr(_)) {
                ExitCode::from(2)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

fn flag_val(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}
