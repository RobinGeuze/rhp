use super::*;

#[test]
fn round_trips() {
    assert_eq!(pton4(b"1.2.3.4"), Some([1, 2, 3, 4]));
    assert_eq!(pton4(b"1.2.3"), None);
    assert_eq!(pton4(b"1.2.3.4\0"), None);
    assert_eq!(ntop(&[10, 0, 0, 255]).unwrap(), b"10.0.0.255");
    let a = pton6(b"2001:db8::1").unwrap();
    assert_eq!(ntop(&a).unwrap(), b"2001:db8::1");
    assert_eq!(ntop(&[0; 5]), None);
}
