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
- `src/parser/internal/escape.rs` (new), `literal.rs`, `string.rs`: string
  escapes are decoded by a port of php's zend_scan_escape_string instead of
  mago_syntax_core's `parse_literal_string_in`: `\X` is a hex escape, and
  `\"` / `` \` `` decode only inside their own quotes (a heredoc keeps both).
  Symfony Yaml's heredoc fixtures, WordPress kses' `\X1C` data.
- Memory (rhp parses a 1.1 MB file into 37 MB of arena instead of 177 MB):
  - `src/cst/cst/expression.rs`, `src/cst/cst/statement.rs`: every
    `Expression` and `Statement` variant holds `&'arena Node` (the spans of
    `Expression::Error` and `Statement::Noop` too), so both enums are 16
    bytes and each node takes its own size in the arena instead of the
    largest variant's (376 / 312 bytes). Match ergonomics see through the
    references, so code that matches on the CST mostly compiles unchanged;
    the parser allocates each node (`self.arena.alloc(..)`). A candidate to
    offer upstream (oxc's AST does the same).
  - `src/parser/internal/alloc.rs` (`new_vec`, `sequence`,
    `token_separated`, `slice`), `src/parser/stream.rs`,
    `src/parser/mod.rs`: lists are collected in heap vectors and copied into
    the arena at their final length. Growing a vector inside the bump arena
    left every outgrown buffer and the spare capacity behind.
  - `src/settings.rs` `ParserSettings::discard_trivia`,
    `src/parser/stream.rs`: skip collecting comments and whitespace
    (rhp finds doc comments from the tokens).

To update: copy the new release's `src/` and `Cargo.toml` (without the
`[[test]]`, `[[bench]]` and `[dev-dependencies]` entries) over this
directory, reapply the changes above, and bump the version here and in
THIRD-PARTY-NOTICES.
