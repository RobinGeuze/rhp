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

#[test]
fn bump_blocks_are_aligned_disjoint_and_kept() {
    use mago_allocator::alloc::Allocator;
    use std::alloc::Layout;
    let bump = bump::Bump::new();
    let mut blocks = Vec::new();
    // Small blocks of mixed alignment across several chunks, a block
    // with a chunk of its own, and zero-sized ones.
    for i in 0..5000usize {
        let (size, align) = match i % 5 {
            0 => (1, 1),
            1 => (24, 8),
            2 => (100, 16),
            3 => (0, 8),
            _ => (if i == 4 { 20_000 } else { 7 }, 4),
        };
        let layout = Layout::from_size_align(size, align).unwrap();
        let p = bump.allocate(layout).unwrap();
        let addr = p.cast::<u8>().as_ptr();
        assert_eq!(addr as usize % align, 0);
        assert_eq!(p.len(), size);
        // SAFETY: the block is `size` bytes, written once.
        unsafe { std::ptr::write_bytes(addr, i as u8, size) };
        blocks.push((addr, size, i as u8));
    }
    for &(addr, size, byte) in &blocks {
        // SAFETY: the blocks live as long as `bump`.
        let s = unsafe { std::slice::from_raw_parts(addr, size) };
        assert!(s.iter().all(|&b| b == byte));
    }
}

#[test]
fn parses_into_the_bump_arena() {
    let arena = Arena::new();
    let mut src = b"<?php\n".to_vec();
    for i in 0..2000 {
        src.extend_from_slice(format!("function f{i}($a) {{ return [$a, 'k' => $a + {i}]; }}\n").as_bytes());
    }
    let parsed = arena.parse(b"t.php", &src);
    assert!(!parsed.has_parse_errors());
    assert_eq!(parsed.program().statements.len(), 2001);
}
