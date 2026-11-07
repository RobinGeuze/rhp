<?php
// filter_input* read php_sapi_filter's raw copies, not the superglobals.
$_GET['a'] = 'changed';
var_dump(
    filter_input(INPUT_GET, 'a', FILTER_VALIDATE_INT),
    filter_input(INPUT_GET, 'b', FILTER_VALIDATE_INT, FILTER_NULL_ON_FAILURE),
    filter_input(INPUT_GET, 'missing'),
    filter_input(INPUT_GET, 'missing', FILTER_DEFAULT, FILTER_NULL_ON_FAILURE),
    filter_input(INPUT_GET, 'missing', FILTER_DEFAULT, ['options' => ['default' => 'dflt']]),
    filter_input(INPUT_GET, '0'),
    filter_input(INPUT_COOKIE, 'c'),
    filter_input(INPUT_POST, 'p'),
    filter_input(INPUT_POST, 'q', FILTER_DEFAULT, FILTER_REQUIRE_ARRAY),
    filter_input(INPUT_SERVER, 'REQUEST_METHOD'),
    filter_input_array(INPUT_SERVER),
    filter_has_var(INPUT_GET, 'a'),
    filter_has_var(INPUT_GET, '0'),
    filter_has_var(INPUT_ENV, 'QUERY_STRING'),
    filter_input_array(INPUT_GET),
    filter_input_array(INPUT_GET, ['a' => FILTER_VALIDATE_INT, 'arr' => ['filter' => FILTER_VALIDATE_INT, 'flags' => FILTER_REQUIRE_ARRAY], 'zz' => FILTER_DEFAULT]),
    filter_input(INPUT_GET, 'arr', FILTER_DEFAULT, FILTER_REQUIRE_ARRAY),
    filter_input(INPUT_GET, 'arr'),
);
try {
    filter_input(INPUT_GET, 'missing', FILTER_DEFAULT, FILTER_THROW_ON_FAILURE);
} catch (Filter\FilterFailedException $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
