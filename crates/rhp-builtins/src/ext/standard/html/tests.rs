use super::*;
use crate::test_support::TestHost;

fn call(f: crate::NativeFn, args: &[&str]) -> Vec<u8> {
    let args: Vec<Value> = args.iter().map(|a| Value::str(*a)).collect();
    match f(&mut TestHost::new(), &args).expect("no throw") {
        Value::Str(b) => b.as_bytes().to_vec(),
        other => panic!("expected string, got {}", other.type_name()),
    }
}

// Expected values captured from php.
#[test]
fn defaults() {
    assert_eq!(
        call(fn_htmlspecialchars, &["Tom & Jerry <> \"q\" 'a'"]),
        b"Tom &amp; Jerry &lt;&gt; &quot;q&quot; &#039;a&#039;"
    );
    assert_eq!(
        call(fn_htmlentities, &["<a> & caf\u{e9} \u{20ac}"]),
        b"&lt;a&gt; &amp; caf&eacute; &euro;"
    );
    assert_eq!(
        call(
            fn_htmlspecialchars_decode,
            &["&lt;a&gt; &amp; &quot;q&quot; &#039;a&#039; &#39;b&#39; &apos;c&apos;"]
        ),
        b"<a> & \"q\" 'a' 'b' &apos;c&apos;"
    );
    assert_eq!(call(fn_htmlspecialchars_decode, &["&amp;lt;"]), b"&lt;");
    assert_eq!(
        call(
            fn_html_entity_decode,
            &["&lt;a&gt;&amp;&quot;&#039;&#39;&apos;&copy;&nbsp;&#65;&#x41;"]
        ),
        b"<a>&\"''&apos;\xc2\xa9\xc2\xa0AA"
    );
    assert_eq!(
        call(fn_html_entity_decode, &["&#x41;&#X1F600;"]),
        b"A\xf0\x9f\x98\x80"
    );
    assert_eq!(
        call(fn_html_entity_decode, &["&copy x &amp y"]),
        b"&copy x &amp y"
    );
    assert_eq!(
        call(fn_html_entity_decode, &["&#0;&#127;&#128;"]),
        b"&#0;&#127;&#128;"
    );
    assert_eq!(
        call(fn_html_entity_decode, &["&#1114112;&#xD800;"]),
        b"&#1114112;&#xD800;"
    );
    assert_eq!(
        call(fn_html_entity_decode, &["caf\u{e9} &eacute; &#233;"]),
        "café é é".as_bytes()
    );
}

#[test]
fn doctypes() {
    // ENT_QUOTES | ENT_XML1, ENT_HTML5, ENT_XHTML
    let s = "\u{e9}'\u{2242}\u{338}<";
    let f = |flags: &str| {
        let args = [Value::str(s), Value::Int(flags.parse().unwrap())];
        match fn_htmlentities(&mut TestHost::new(), &args).unwrap() {
            Value::Str(b) => String::from_utf8(b.as_bytes().to_vec()).unwrap(),
            _ => unreachable!(),
        }
    };
    assert_eq!(f("19"), "\u{e9}&apos;\u{2242}\u{338}&lt;");
    assert_eq!(f("51"), "&eacute;&apos;&nesim;&lt;");
    assert_eq!(f("35"), "&eacute;&#039;\u{2242}\u{338}&lt;");
}
