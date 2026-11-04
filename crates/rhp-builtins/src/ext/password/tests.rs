use super::*;

#[test]
fn base64_matches_php() {
    assert_eq!(base64(b""), b"");
    assert_eq!(base64(b"f"), b"Zg==");
    assert_eq!(base64(b"fo"), b"Zm8=");
    assert_eq!(base64(b"foo"), b"Zm9v");
    assert_eq!(base64(&[0xfb, 0xff]), b"+/8=");
}

#[test]
fn argon2_params_like_sscanf() {
    let d = (65536, 4, 1);
    assert_eq!(
        argon2_params(b"$argon2i$v=19$m=1024,t=2,p=3$x$y", d),
        (1024, 2, 3)
    );
    assert_eq!(
        argon2_params(b"$argon2id$v=19$m=1024", (0, 0, 0)),
        (1024, 0, 0)
    );
    assert_eq!(argon2_params(b"$argon2i$m=1024,t=2,p=3$", d), d);
    assert_eq!(argon2_params(b"$argon2i$", d), d);
}

#[test]
fn ident() {
    assert_eq!(extract_ident(b"$2y$10$"), Some(&b"2y"[..]));
    assert_eq!(extract_ident(b"x2y$"), Some(&b"2y"[..]));
    assert_eq!(extract_ident(b"$2\0y$"), None);
    assert_eq!(extract_ident(b"$2"), None);
}
