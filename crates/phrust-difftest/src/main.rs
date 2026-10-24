//! `phrust-difftest` — run an extension's differential sweep against the real
//! `php` binary and print a per-feature verdict.
//!
//! Usage:
//!   phrust-difftest [hash]            # sweep the hash extension (default)
//!   phrust-difftest hash --seed 7 --count 64
//!   phrust-difftest hash --algo tiger128,3     # one algorithm
//!   phrust-difftest hash --strict              # unsupported algos count as failures
//!
//! Exit code is nonzero if any sweep produced a MISMATCH (or, with --strict,
//! any phrust-unsupported feature).

use std::process::ExitCode;

use phrust_difftest::sweeps::{hash_algo, php_hash_algos};
use phrust_difftest::{default_byte_corpus, php_available, Diff};

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

    if !php_available() {
        eprintln!("phrust-difftest: no `php` binary on PATH — cannot run a differential sweep.");
        eprintln!("(set PHRUST_PHP to point at one, or install php 8.5.x)");
        return ExitCode::from(2);
    }

    match ext.as_str() {
        "hash" => sweep_hash(seed, count, only.as_deref(), strict),
        other => {
            eprintln!("phrust-difftest: unknown extension {other:?} (known: hash)");
            ExitCode::from(2)
        }
    }
}

fn sweep_hash(seed: u64, count: usize, only: Option<&str>, strict: bool) -> ExitCode {
    let algos = match php_hash_algos() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("phrust-difftest: could not query php hash_algos(): {e}");
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
            Diff::Mismatch { php, phrust } => {
                mismatched += 1;
                println!("  MISMATCH     {algo}");
                print_first_diff(&php, &phrust);
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

fn print_first_diff(php: &[u8], phrust: &[u8]) {
    let php_s = String::from_utf8_lossy(php);
    let phrust_s = String::from_utf8_lossy(phrust);
    for (i, (a, b)) in php_s.lines().zip(phrust_s.lines()).enumerate() {
        if a != b {
            println!("      input #{i}: php={a}  phrust={b}");
            return;
        }
    }
    println!("      (length differs: php {} B, phrust {} B)", php.len(), phrust.len());
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

fn flag_val(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1).cloned())
}
