<?php
var_dump(preg_last_error(), preg_last_error_msg());
var_dump(@preg_match('/(/', 'x'), preg_last_error(), preg_last_error_msg());
var_dump(preg_match('/a/', 'a'), preg_last_error(), preg_last_error_msg());
var_dump(preg_match('/a/u', "\xff"), preg_last_error(), preg_last_error_msg());
var_dump(preg_quote('x'), preg_last_error());
var_dump(preg_match('/a/u', "a\xc3\xa9", $m, 0, 2), preg_last_error_msg());
var_dump(preg_match('/(?:\D+|<\d+>)*[!?]/', 'foobar foobar foobar'), preg_last_error(), preg_last_error_msg());
var_dump(preg_replace('/a/u', 'b', "\xff"), preg_last_error_msg());
var_dump(preg_split('/,/u', "a,\xff"), preg_last_error_msg());
var_dump(preg_match_all('/a/', 'aaa'), preg_last_error_msg());

// preg_replace_callback_array
$subject = 'Aaaaaa Bbb';
$out = preg_replace_callback_array(
    [
        '~[a]+~i' => function ($m) { return 'a' . strlen($m[0]); },
        '~[b]+~i' => function ($m) { return 'b' . strlen($m[0]); },
    ],
    $subject,
    -1,
    $count
);
var_dump($out, $count);
var_dump(preg_replace_callback_array(['/\d/' => fn($m) => '<' . $m[0] . '>'], ['x' => 'a1b2', 'y' => 'c3'], 1, $count), $count);
var_dump(preg_replace_callback_array(['/(a)(z)?/' => fn($m) => json_encode($m)], 'ab', -1, $c, PREG_UNMATCHED_AS_NULL));
var_dump(preg_replace_callback_array(['/b/' => fn($m) => json_encode($m)], 'ab', -1, $c, PREG_OFFSET_CAPTURE));
var_dump(preg_replace_callback_array([], 'unchanged', -1, $c), $c);
// a later pattern sees the earlier pattern's output
var_dump(preg_replace_callback_array(['/a/' => fn() => 'b', '/b/' => fn() => 'c'], 'ab'));
// failing pattern → null
var_dump(@preg_replace_callback_array(['/(/' => fn() => 'x'], 'ab'));
try {
    preg_replace_callback_array([0 => fn() => 'x'], 'ab');
} catch (TypeError $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
try {
    preg_replace_callback_array(['/a/' => 'no_such_function_xyz'], 'ab');
} catch (TypeError $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}

// preg_replace_callback flags
var_dump(preg_replace_callback('/(a)(z)?/', fn($m) => json_encode($m), 'xa', -1, $c, PREG_UNMATCHED_AS_NULL), $c);
var_dump(preg_replace_callback('/a/', fn($m) => json_encode($m), 'xa', -1, $c, PREG_OFFSET_CAPTURE));
var_dump(preg_replace_callback('/a/u', fn($m) => 'X', ['k' => "\xff", 'j' => 'a']));
