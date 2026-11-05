use super::*;

/// The value, as `type:repr`.
fn ev(text: &str) -> String {
    let v = eval(text, &mut |n| match n {
        Name::Const("A") => Ok(Value::Int(1)),
        Name::Const("B") => Ok(Value::Int(4)),
        _ => Err(bad("unknown")),
    });
    match v {
        Ok(Value::Int(n)) => format!("int:{n}"),
        Ok(Value::Float(f)) => format!("float:{f}"),
        Ok(Value::Str(s)) => format!("string:{:?}", s.as_bytes()),
        Ok(Value::Array(a)) => format!("array:{}", a.len()),
        Ok(_) => "other".into(),
        Err(_) => "error".into(),
    }
}

#[test]
fn evaluates_stub_defaults() {
    assert_eq!(ev("A | B"), "int:5");
    assert_eq!(ev("2 * 1024 * 1024"), "int:2097152");
    assert_eq!(ev("-1"), "int:-1");
    assert_eq!(ev("0777"), "int:511");
    assert_eq!(ev("1.0"), "float:1");
    assert_eq!(ev("\" \\n\\x00\""), format!("string:{:?}", b" \n\0"));
    assert_eq!(ev("'a\\'b'"), format!("string:{:?}", b"a'b"));
    assert_eq!(ev("X::class"), format!("string:{:?}", b"X"));
    assert_eq!(ev("[]"), "array:0");
    assert_eq!(ev("C"), "error");
    assert_eq!(constant_name("PHP_INT_MAX"), Some("PHP_INT_MAX"));
    assert_eq!(constant_name("A::B"), Some("A::B"));
    assert_eq!(constant_name("A::class"), None);
    assert_eq!(constant_name("null"), None);
    assert_eq!(constant_name("A | B"), None);
}
