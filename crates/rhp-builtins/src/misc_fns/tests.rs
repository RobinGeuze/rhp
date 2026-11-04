// tests

use super::*;

fn arr(items: &[Value]) -> Value {
    let mut a = rhp_values::PhpArray::new();
    for v in items {
        a.push(v.clone());
    }
    Value::Array(a)
}

struct NoopHost;
impl Host for NoopHost {
    fn write_out(&mut self, _: &[u8]) {}
}

#[test]
fn iterable_countable() {
    assert!(matches!(
        fn_is_iterable(&mut NoopHost, &[arr(&[])]).unwrap(),
        Value::Bool(true)
    ));
    assert!(matches!(
        fn_is_iterable(&mut NoopHost, &[Value::str("abc")]).unwrap(),
        Value::Bool(false)
    ));
    assert!(matches!(
        fn_is_countable(&mut NoopHost, &[arr(&[Value::Int(1)])]).unwrap(),
        Value::Bool(true)
    ));
    assert!(matches!(
        fn_is_countable(&mut NoopHost, &[Value::Int(5)]).unwrap(),
        Value::Bool(false)
    ));
}
