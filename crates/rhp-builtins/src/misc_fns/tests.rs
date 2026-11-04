// tests

use super::*;
use crate::test_support::TestHost;

fn arr(items: &[Value]) -> Value {
    let mut a = rhp_values::PhpArray::new();
    for v in items {
        a.push(v.clone());
    }
    Value::Array(a)
}

#[test]
fn iterable_countable() {
    assert!(matches!(
        fn_is_iterable(&mut TestHost::new(), &[arr(&[])]).unwrap(),
        Value::Bool(true)
    ));
    assert!(matches!(
        fn_is_iterable(&mut TestHost::new(), &[Value::str("abc")]).unwrap(),
        Value::Bool(false)
    ));
    assert!(matches!(
        fn_is_countable(&mut TestHost::new(), &[arr(&[Value::Int(1)])]).unwrap(),
        Value::Bool(true)
    ));
    assert!(matches!(
        fn_is_countable(&mut TestHost::new(), &[Value::Int(5)]).unwrap(),
        Value::Bool(false)
    ));
}
