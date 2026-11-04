use super::*;

// php -r 'echo crypt("rasmuslerdorf", ...);'
#[test]
fn md5_crypt_matches_php() {
    assert_eq!(
        php_crypt(b"rasmuslerdorf", b"$1$rasmusle$").as_deref(),
        Some(&b"$1$rasmusle$rISCgZzpwk3UhDidwXvin0"[..])
    );
    assert_eq!(
        php_crypt(b"", b"$1$").as_deref(),
        Some(&b"$1$$qRPK7m23GJusamGpoGLby/"[..])
    );
}
