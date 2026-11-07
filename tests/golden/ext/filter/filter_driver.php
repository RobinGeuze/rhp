<?php
// ext/filter's driver: scalar/array dispatch, flags, options, callbacks,
// FILTER_THROW_ON_FAILURE and the *_array functions.
set_error_handler(function (int $no, string $msg): bool {
    echo "error $no: $msg\n";
    return true;
});
function show(string $label, callable $f): void {
    try {
        echo $label, ': ';
        var_dump($f());
    } catch (Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
class S { function __toString(): string { return "12"; } }
class N {}

// scalars and arrays
show('array, default flags', fn() => filter_var(['1'], FILTER_VALIDATE_INT));
show('array, null on failure', fn() => filter_var(['1'], FILTER_VALIDATE_INT, FILTER_NULL_ON_FAILURE));
show('force array', fn() => filter_var(['1', 'x', ['2', 'y']], FILTER_VALIDATE_INT, FILTER_FORCE_ARRAY));
show('force array scalar', fn() => filter_var('7', FILTER_VALIDATE_INT, FILTER_FORCE_ARRAY));
show('require array scalar', fn() => filter_var('7', FILTER_VALIDATE_INT, FILTER_REQUIRE_ARRAY));
show('require scalar array', fn() => filter_var(['7'], FILTER_VALIDATE_INT, ['flags' => FILTER_REQUIRE_SCALAR]));
show('getInt on x[]=1', fn() => filter_var(['1'], FILTER_VALIDATE_INT, ['flags' => FILTER_NULL_ON_FAILURE | FILTER_REQUIRE_SCALAR]));
show('stringable', fn() => filter_var(new S, FILTER_VALIDATE_INT));
show('no __toString', fn() => filter_var(new N, FILTER_VALIDATE_INT));
show('no __toString null', fn() => filter_var(new N, FILTER_VALIDATE_INT, FILTER_NULL_ON_FAILURE));
show('closure', fn() => filter_var(fn() => 1, FILTER_DEFAULT));
show('bool true int', fn() => filter_var(true, FILTER_VALIDATE_INT));
show('float int', fn() => filter_var(3.0, FILTER_VALIDATE_INT));
show('null bool', fn() => filter_var(null, FILTER_VALIDATE_BOOL, FILTER_NULL_ON_FAILURE));
show('default option', fn() => filter_var('x', FILTER_VALIDATE_INT, ['options' => ['default' => 5]]));
show('default with null', fn() => filter_var('x', FILTER_VALIDATE_INT, ['options' => ['default' => 5], 'flags' => FILTER_NULL_ON_FAILURE]));
show('range', fn() => filter_var('50', FILTER_VALIDATE_INT, ['options' => ['min_range' => 1, 'max_range' => 10]]));
show('hex', fn() => filter_var('0x1F', FILTER_VALIDATE_INT, FILTER_FLAG_ALLOW_HEX));
show('octal 0o', fn() => filter_var('0o17', FILTER_VALIDATE_INT, FILTER_FLAG_ALLOW_OCTAL));
show('float thousand', fn() => filter_var('1,234.5', FILTER_VALIDATE_FLOAT, FILTER_FLAG_ALLOW_THOUSAND));
show('float decimal', fn() => filter_var('1,5', FILTER_VALIDATE_FLOAT, ['options' => ['decimal' => ',']]));
show('unknown filter', fn() => filter_var('x', 12345));

// references are filtered in place
$x = '5';
$arr = [&$x, 'a'];
show('ref', fn() => filter_var($arr, FILTER_VALIDATE_INT, FILTER_REQUIRE_ARRAY));
var_dump($x);

// FILTER_CALLBACK
show('callback', fn() => filter_var('abc', FILTER_CALLBACK, ['options' => 'strtoupper']));
show('callback array', fn() => filter_var(['a', ['b']], FILTER_CALLBACK, ['options' => 'strtoupper']));
show('callback closure', fn() => filter_var('  ', FILTER_CALLBACK, ['options' => fn($s) => trim($s) === '']));
show('callback int arg', fn() => filter_var(5, FILTER_CALLBACK, ['options' => fn($s) => gettype($s)]));
show('callback invalid', fn() => filter_var('x', FILTER_CALLBACK, ['options' => 'no_such_fn']));
show('callback missing', fn() => filter_var('x', FILTER_CALLBACK));
show('callback throws', fn() => filter_var('x', FILTER_CALLBACK, ['options' => function ($s) { throw new RuntimeException("cb $s"); }]));

// FILTER_THROW_ON_FAILURE
show('throw int', fn() => filter_var('nope', FILTER_VALIDATE_INT, FILTER_THROW_ON_FAILURE));
show('throw ok', fn() => filter_var('9', FILTER_VALIDATE_INT, FILTER_THROW_ON_FAILURE));
show('throw both', fn() => filter_var('9', FILTER_VALIDATE_INT, FILTER_THROW_ON_FAILURE | FILTER_NULL_ON_FAILURE));
show('throw array', fn() => filter_var(['9'], FILTER_VALIDATE_INT, FILTER_THROW_ON_FAILURE));
show('throw not array', fn() => filter_var(1.5, FILTER_VALIDATE_INT, FILTER_THROW_ON_FAILURE | FILTER_REQUIRE_ARRAY));
show('throw object', fn() => filter_var(new N, FILTER_VALIDATE_INT, FILTER_THROW_ON_FAILURE));
show('throw default', fn() => filter_var('x', FILTER_VALIDATE_INT, ['flags' => FILTER_THROW_ON_FAILURE, 'options' => ['default' => 1]]));
show('throw in array', fn() => filter_var(['1', 'x', 'y'], FILTER_VALIDATE_INT, FILTER_THROW_ON_FAILURE | FILTER_FORCE_ARRAY));
var_dump(get_parent_class(Filter\FilterFailedException::class), get_parent_class(Filter\FilterException::class));

// IP ranges
foreach (['10.0.0.1', '127.0.0.1', '8.8.8.8', '192.88.99.1', '::1', 'fd00::1', '64:ff9b::1', '::ffff:1.2.3.4', '2001:db8::1', '1.2.3'] as $ip) {
    echo $ip, ' ';
    var_dump(
        filter_var($ip, FILTER_VALIDATE_IP, FILTER_FLAG_NO_PRIV_RANGE),
        filter_var($ip, FILTER_VALIDATE_IP, FILTER_FLAG_NO_RES_RANGE),
        filter_var($ip, FILTER_VALIDATE_IP, FILTER_FLAG_GLOBAL_RANGE),
    );
}

// sanitizers
var_dump(
    filter_var("a\x01b\x80c`d&e", FILTER_UNSAFE_RAW, FILTER_FLAG_STRIP_LOW | FILTER_FLAG_ENCODE_HIGH | FILTER_FLAG_STRIP_BACKTICK | FILTER_FLAG_ENCODE_AMP),
    filter_var('', FILTER_UNSAFE_RAW, FILTER_FLAG_EMPTY_STRING_NULL),
    filter_var('a b/é', FILTER_SANITIZE_ENCODED),
    filter_var('<a href="x">\'é\'</a>', FILTER_SANITIZE_FULL_SPECIAL_CHARS),
    filter_var('<a href="x">\'</a>', FILTER_SANITIZE_FULL_SPECIAL_CHARS, FILTER_FLAG_NO_ENCODE_QUOTES),
    filter_var("O'Re\"il\\ly\0", FILTER_SANITIZE_ADD_SLASHES),
    filter_var('1,234.5e6', FILTER_SANITIZE_NUMBER_FLOAT, FILTER_FLAG_ALLOW_FRACTION | FILTER_FLAG_ALLOW_SCIENTIFIC),
    filter_var('abc', FILTER_VALIDATE_REGEXP, ['options' => ['regexp' => '/^a/']]),
    filter_var('xbc', FILTER_VALIDATE_REGEXP, ['options' => ['regexp' => '/^a/']]),
    filter_var('aa:bb:cc:dd:ee:ff', FILTER_VALIDATE_MAC, ['options' => ['separator' => '-']]),
    filter_var('http://user:p@ss@example.com/', FILTER_VALIDATE_URL),
    filter_var('http://[::1]:80/x?y', FILTER_VALIDATE_URL, FILTER_FLAG_PATH_REQUIRED | FILTER_FLAG_QUERY_REQUIRED),
);
show('regexp missing', fn() => filter_var('a', FILTER_VALIDATE_REGEXP));

// filter_var_array
show('var_array', fn() => filter_var_array(
    ['id' => '12', 'tags' => ['1', 'x'], 'name' => '<b>', 'extra' => 'e'],
    [
        'id' => FILTER_VALIDATE_INT,
        'tags' => ['filter' => FILTER_VALIDATE_INT, 'flags' => FILTER_REQUIRE_ARRAY],
        'name' => ['filter' => FILTER_SANITIZE_SPECIAL_CHARS],
        'missing' => FILTER_DEFAULT,
    ],
));
show('var_array no empty', fn() => filter_var_array(['a' => '1'], ['a' => FILTER_VALIDATE_INT, 'b' => FILTER_VALIDATE_INT], false));
show('var_array one filter', fn() => filter_var_array(['a' => '1', 'b' => ['2', 'x']], FILTER_VALIDATE_INT));
show('var_array int key', fn() => filter_var_array(['a' => '1'], ['a' => FILTER_DEFAULT, 5 => FILTER_DEFAULT]));
show('var_array unknown', fn() => filter_var_array(['a' => '1'], 999));

// the request inputs php-cli never fills
var_dump(filter_input(INPUT_GET, 'a'), filter_input_array(INPUT_POST), filter_has_var(INPUT_COOKIE, 'a'));
show('bad input type', fn() => filter_input(42, 'a'));
show('bad has_var type', fn() => filter_has_var(42, 'a'));
var_dump(filter_input(INPUT_GET, 'a', FILTER_DEFAULT, FILTER_NULL_ON_FAILURE));
show('input throw', fn() => filter_input(INPUT_GET, 'a', FILTER_DEFAULT, FILTER_THROW_ON_FAILURE));
var_dump(filter_id('validate_email'), filter_id('callback'), filter_id('x'), count(filter_list()));
