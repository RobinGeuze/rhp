<?php
// zpp that depends on the call: strtr parses $from as an array with two
// arguments and $from, $to as strings with three; implode's frameless
// variants (a direct call with one or two arguments) parse $separator as
// a string, or nothing with one argument, where the function proper
// takes array|string.
set_error_handler(function ($no, $msg) {
    echo $no === E_DEPRECATED ? "Deprecated" : "Warning", ": $msg\n";
    return true;
});
class T { function __toString(): string { return "b"; } }
function t(string $label, callable $f) {
    echo $label, ": ";
    try {
        var_dump($f());
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
$f = 'strtr';
t('strtr 3 null to', fn() => strtr("abc", "a", null));
t('dynamic 3 null to', fn() => $f("abc", "a", null));
t('strtr 3 array from', fn() => strtr("abc", ["a" => "x"], "y"));
t('dynamic 3 array from', fn() => $f("abc", ["a" => "x"], "y"));
t('strtr 2 int from', fn() => strtr("abc", 5));
t('dynamic 2 int from', fn() => $f("abc", 5));
t('strtr 2 string from', fn() => strtr("abc", "a"));
t('strtr 2 null from', fn() => strtr("abc", null));
t('strtr 3 ints', fn() => strtr("a1c", 1, 2));
t('strtr 3 object from', fn() => strtr("abc", new T, "x"));
t('strtr 3 object to', fn() => strtr("abc", "b", new T));
t('strtr 3 array to', fn() => strtr("abc", "b", []));
t('strtr 3 null from', fn() => strtr("abc", null, "x"));
t('call_user_func', fn() => call_user_func('strtr', "abc", "a", null));
t('array_map', fn() => array_map('strtr', ["abc"], [5]));
t('empty string, int from', fn() => strtr("", 5));
t('empty string, array to', fn() => strtr("", "a", []));
t('pairs', fn() => strtr("hi all", ["hi" => "bye", "all" => "you"]));
$g = 'implode';
t('implode object sep', fn() => implode(new stdClass, ["a"]));
t('dynamic object sep', fn() => $g(new stdClass, ["a"]));
t('implode stringable sep', fn() => implode(new T, ["a", "c"]));
t('implode int', fn() => implode(5));
t('dynamic int', fn() => $g(5));
t('implode object', fn() => implode(new stdClass));
t('dynamic object', fn() => $g(new stdClass));
t('implode null', fn() => implode(null));
t('dynamic null', fn() => $g(null));
t('implode null sep', fn() => implode(null, ["a"]));
t('implode string array', fn() => implode(",", "x"));
t('dynamic string array', fn() => $g(",", "x"));
t('implode null array', fn() => implode(",", null));
t('dynamic null array', fn() => $g(",", null));
t('implode two arrays', fn() => implode(["a"], ["b"]));
t('dynamic two arrays', fn() => $g(["a"], ["b"]));
t('implode array, null', fn() => implode(["a", "b"], null));
t('implode float sep', fn() => implode(1.5, [1, 2]));
t('implode int sep', fn() => implode(1, [1, 2]));
