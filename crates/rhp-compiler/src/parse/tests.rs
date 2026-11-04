use super::*;

#[test]
fn parses_echo() {
    let arena = Arena::new();
    let parsed = arena.parse(b"t.php", br#"<?php echo "hi", 1+2;"#);
    assert!(!parsed.has_parse_errors());
    assert!(!parsed.program().statements.is_empty());
}

#[test]
fn tokens_cover_source_with_exact_ranges() {
    let src = b"<?php $a = 10 <= 2.5; // c\necho 'x' . \"y\";";
    let toks = tokens(src);
    // Contiguous, gap-free cover of the whole source.
    assert_eq!(toks.first().unwrap().start, 0);
    assert_eq!(toks.last().unwrap().end, src.len());
    for w in toks.windows(2) {
        assert_eq!(w[0].end, w[1].start);
    }
    let text = |k: TokenKind| {
        let t = toks.iter().find(|t| t.kind == k).unwrap();
        &src[t.start..t.end]
    };
    assert_eq!(text(TokenKind::LiteralInteger), b"10");
    assert_eq!(text(TokenKind::LiteralFloat), b"2.5");
    assert_eq!(text(TokenKind::LessThanEqual), b"<=");
    assert_eq!(text(TokenKind::LiteralString), b"'x'");
    assert_eq!(text(TokenKind::Dot), b".");
}
