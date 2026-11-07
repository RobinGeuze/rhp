# rhp's changes to mago-syntax 1.59.0

Vendored from crates.io (`cargo` source of mago-syntax 1.59.0, `src/` and
`Cargo.toml`; tests and benches left out). The root Cargo.toml patches
crates.io's mago-syntax with this copy. Each change is marked `rhp:` in the
source. Drop a change once upstream mago lexes the case like php.

- `src/lexer/mod.rs`, `follows_object_operator`: after `->` / `?->` (and
  whitespace) `__halt_compiler` is a property name (php's scanner is in
  ST_LOOKING_FOR_PROPERTY, where any label is T_STRING), not the start of
  `__halt_compiler();`. Zend/tests/grammar/semi_reserved_003.phpt.
- `src/lexer/mod.rs`, `matches_start_of_*_document`: a heredoc / nowdoc
  opener ends at `\r` alone too (php's NEWLINE is `\r`, `\n` or `\r\n`).
  Zend/tests/heredoc_nowdoc/bug79934.phpt.

To update: copy the new release's `src/` and `Cargo.toml` (without the
`[[test]]`, `[[bench]]` and `[dev-dependencies]` entries) over this
directory, reapply the changes above, and bump the version here and in
THIRD-PARTY-NOTICES.
