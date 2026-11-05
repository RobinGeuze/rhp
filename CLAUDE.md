# rhp

A PHP 8.5 engine in Rust. The reference is php 8.5.12: rhp must print what it
prints, byte for byte. Start with README.md, then docs/design/ (bytecode.md
first); tests/phpt/README.md and tests/apps/README.md describe the harnesses.

## Keep the README's tables current

README.md has three sections that describe the code: the crate list and the
PHP test support and application support tables. A change to what one of them
describes updates it in the same PR.

- **Crates**: a crate added, removed, renamed or given another job.
- **PHP test support**: generated from tests/phpt/known-passing.txt,
  ignored.txt and totals.txt. After changing those lists, run
  `cargo run -p rhp-difftest -- gen readme`. After changing
  tests/phpt/php-src.ref, run
  `cargo run -p rhp-difftest -- gen readme --php-src <php-src at that commit>`
  (all of its tests; it also rewrites totals.txt). Only the notes column is
  written by hand: update a note when what it says changes. `cargo test`
  (crates/rhp-difftest/tests/generated.rs) and CI's phpt job check the table.
- **Application support**: whenever a change makes an app work better or
  worse. Re-measure, and write down only what was checked:
  - Composer: `python3 tests/apps/composer_diff.py` (needs network access).
  - The fixture apps: `python3 tests/apps/apps.py setup <app>`, then
    `python3 tests/apps/apps.py lint <app>` (files that compile) and
    `python3 tests/apps/apps.py diff <app>` (requests answered like php-cgi),
    with `cargo build --release -p rhp-cli -p rhp-difftest` first. Also run
    the app's console (`bin/console`, `artisan`) under php and under
    `target/release/rhp`.

## Licences

Whenever third-party code is vendored, ported or linked, update
THIRD-PARTY-NOTICES and the crate's NOTICE file (crates/rhp-csrc/NOTICE) in
the same change. The policy is in docs/design/libraries.md ("Licences").

## Checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
RHP_PHP=php cargo test --workspace --all-targets --no-fail-fast
```
