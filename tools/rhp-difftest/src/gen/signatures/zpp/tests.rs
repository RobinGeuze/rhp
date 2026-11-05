use super::*;

const SRC: &[u8] = br#"
/* {{{ PHP_FUNCTION(not_this) */
PHPAPI PHP_FUNCTION(fwrite)
{
	ZEND_PARSE_PARAMETERS_START(2, 3)
		PHP_Z_PARAM_STREAM(stream)
		Z_PARAM_STRING(input, inputlen)
		Z_PARAM_OPTIONAL
		Z_PARAM_LONG_OR_NULL(maxlen, maxlen_is_null) // Z_PARAM_ZVAL(x)
	ZEND_PARSE_PARAMETERS_END();
	if (x) { char c = '}'; }
}

ZEND_FUNCTION(old_style)
{
	if (zend_parse_parameters(ZEND_NUM_ARGS(), "r!|zl", &a, &b, &c) == FAILURE) {
		RETURN_THROWS();
	}
}

static PHP_FUNCTION(two_blocks)
{
	if (ZEND_NUM_ARGS() == 1) {
		ZEND_PARSE_PARAMETERS_START(1, 1) Z_PARAM_ZVAL(a) ZEND_PARSE_PARAMETERS_END();
	} else {
		ZEND_PARSE_PARAMETERS_START(2, 2) Z_PARAM_ZVAL(a) Z_PARAM_ZVAL(b) ZEND_PARSE_PARAMETERS_END();
	}
}

PHP_FUNCTION(helper)
{
	php_do_it(INTERNAL_FUNCTION_PARAM_PASSTHRU);
}

PHP_FUNCTION(none)
{
	ZEND_PARSE_PARAMETERS_NONE();
}

PHP_FUNCTION(raises_first)
{
	if (ZEND_NUM_ARGS() == 2) {
		zend_error(E_DEPRECATED, "two arguments");
	}
	ZEND_PARSE_PARAMETERS_START(2, 2) Z_PARAM_RESOURCE(a) Z_PARAM_ZVAL(b) ZEND_PARSE_PARAMETERS_END();
}
"#;

#[test]
fn reads_each_bodys_one_parse() {
    let src = strip_comments(SRC);
    let got: Vec<(String, Option<Vec<Parse>>)> = functions(&src)
        .into_iter()
        .map(|(n, b)| (n, parses(b)))
        .collect();
    use Parse::*;
    assert_eq!(
        got,
        vec![
            ("fwrite".into(), Some(vec![Stream(false), Other, Other])),
            ("old_style".into(), Some(vec![Resource(true), Zval, Other])),
            ("two_blocks".into(), None),
            ("helper".into(), None),
            ("none".into(), Some(vec![])),
            ("raises_first".into(), None),
        ]
    );
}
