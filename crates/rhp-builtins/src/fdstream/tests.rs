use super::Dechunk;

fn dechunk(parts: &[&[u8]]) -> Vec<u8> {
    let mut d = Dechunk::default();
    parts.iter().flat_map(|p| d.feed(p.to_vec())).collect()
}

#[test]
fn dechunks_like_php() {
    let body: &[u8] = b"4\r\nWiki\r\n5;ext=1\r\npedia\r\n0\r\nTrailer: x\r\n\r\n";
    assert_eq!(dechunk(&[body]), b"Wikipedia");
    // the same body one byte per bucket
    let bytes: Vec<&[u8]> = body.chunks(1).collect();
    assert_eq!(dechunk(&bytes), b"Wikipedia");
    // LF-only line ends are accepted
    assert_eq!(dechunk(&[b"3\nabc\n0\n\n"]), b"abc");
    // not a chunk size: the rest passes through as it is
    assert_eq!(dechunk(&[b"zz\r\nraw"]), b"zz\r\nraw");
    // a bad chunk end switches to pass-through after the data
    assert_eq!(dechunk(&[b"2\r\nabX\r\nmore"]), b"abX\r\nmore");
}
