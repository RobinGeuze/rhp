<?php
// php_pcre.c behaviour around PCRE2: limits, $matches setup, preg_filter,
// preg_split's limit counting, partial results on errors.
set_error_handler(function ($no, $msg) { echo "warning: $msg\n"; return true; });
$cases = [
    'replace limit' => fn() => preg_replace('/a*?/', 'x', 'baaac', 2),
    'replace bad utf8 entry' => fn() => [preg_replace('/a/u', 'x', ["a", "a\xff", "b"], -1, $c), $c, preg_last_error_msg()],
    'replace string fails, count' => fn() => [preg_replace('/a/u', 'x', "a\xff", -1, $c), $c],
    'filter array' => fn() => [preg_filter('/a/', 'x', ["a", "b", 'k' => "ca"], -1, $c), $c],
    'filter string' => fn() => [preg_filter('/a/', 'x', "bbb"), preg_filter('/a/', 'x', "bab")],
    'filter arrays' => fn() => preg_filter(['/\d/', '/[a-z]/'], ['A:$0', 'B:$0'], ['1', 'a', 'X']),
    'replacement array, pattern string' => fn() => preg_replace('/a/', ['x'], 'a'),
    'split no_empty limit' => fn() => preg_split('/,/', ',a,,b,c', 2, PREG_SPLIT_NO_EMPTY),
    'split negative limit' => fn() => preg_split('/,/', 'a,b', -5),
    'split unset group offset' => fn() => preg_split('/(x)?(,)/', 'a,b', -1, PREG_SPLIT_DELIM_CAPTURE | PREG_SPLIT_OFFSET_CAPTURE),
    'split empty no_empty' => fn() => preg_split('/x/', '', -1, PREG_SPLIT_NO_EMPTY),
    'split utf8 empty' => fn() => preg_split('//u', "añb", -1, PREG_SPLIT_NO_EMPTY | PREG_SPLIT_OFFSET_CAPTURE),
    'match flags' => fn() => [preg_match('/a/', 'a', $m, PREG_SET_ORDER), $m],
    'match_all flags' => fn() => [preg_match_all('/a/', 'a', $m, 0xdead), $m],
    'bad utf8 offset' => fn() => [preg_match('/./u', "\xc3\xa9x", $m, 0, 1), $m, preg_last_error_msg()],
    'offset past end' => fn() => [preg_match('/x/', 'abc', $m, 0, 4), $m, preg_last_error_msg()],
    'dupnames' => fn() => [preg_match('/(?J)(?:(?<g>foo)|(?<g>bar))/', 'foo', $m, PREG_UNMATCHED_AS_NULL), $m],
    'match_all partial on error' => fn() => [ini_set('pcre.jit', '0'), ini_set('pcre.backtrack_limit', '1000'),
        preg_match_all('/\d|(a+)+b/', '1' . str_repeat('a', 24), $m), $m, preg_last_error_msg(),
        preg_match_all('/\d|(a+)+b/', '1' . str_repeat('a', 24), $m, PREG_SET_ORDER), $m,
        ini_restore('pcre.backtrack_limit'), ini_restore('pcre.jit')],
    'backtrack limit, no JIT' => fn() => [ini_set('pcre.jit', '0'), ini_set('pcre.backtrack_limit', '2'),
        preg_match_all('/.*\p{N}/', '0123456789', $m), preg_last_error_msg(),
        ini_restore('pcre.backtrack_limit'), ini_restore('pcre.jit'), ini_get('pcre.jit')],
    'recursion limit, no JIT' => fn() => [ini_set('pcre.jit', '0'), ini_set('pcre.recursion_limit', '2'),
        preg_match('/\p{Ll}(\p{L}((\p{Ll}\p{Ll})))/', 'aeiou'), preg_last_error_msg(),
        ini_restore('pcre.recursion_limit'), ini_restore('pcre.jit')],
    'array subject entry' => fn() => preg_replace_callback('~\A.~', fn($m) => "'$m[0]'", [['xyz']]),
    'grep stops at an error' => fn() => [ini_set('pcre.jit', '0'), ini_set('pcre.backtrack_limit', '1000'),
        preg_grep('/(a+)+$/', ['a', 'b', str_repeat('a', 24) . 'b', 'a']), preg_last_error_msg(),
        preg_grep('/(a+)+$/', ['a', 'b', str_repeat('a', 24) . 'b', 'a'], PREG_GREP_INVERT),
        ini_restore('pcre.backtrack_limit'), ini_restore('pcre.jit')],
    'grep array entry' => fn() => preg_grep('/a/', ['a', ['x'], 'b']),
    'callback returns array' => fn() => preg_replace_callback('/a/', fn($m) => [], 'aa'),
    'callback returns object' => fn() => [(function () {
        try {
            return preg_replace_callback('/a/', fn($m) => new stdClass, 'aa');
        } catch (Error $e) {
            return $e->getMessage();
        }
    })(), preg_last_error_msg()],
    'compile warnings in order' => fn() => preg_replace('/(/', 'x', ['a', ['b'], 'c']),
    'callback and compile warnings in order' => fn() => preg_replace_callback(['/a/', '/(/'], function ($m) {
        echo "cb\n";
        return 'X';
    }, ['a', 'a']),
    'limit quantity warnings' => fn() => [ini_set('pcre.backtrack_limit', 'abc'), ini_set('pcre.recursion_limit', '1x'),
        ini_set('pcre.recursion_limit', '99999999999999999999'), ini_get('pcre.backtrack_limit'),
        ini_restore('pcre.backtrack_limit'), ini_restore('pcre.recursion_limit')],
    'caseless restrict' => fn() => [preg_match('/AskZ/iur', "A\u{17f}kZ"), preg_match('/AskZ/iu', "A\u{17f}kZ")],
    'unknown modifier' => fn() => preg_replace('/(.)/e', 'x', 'abc'),
];
foreach ($cases as $name => $f) {
    try {
        $r = $f();
    } catch (Throwable $e) {
        $r = get_class($e) . ': ' . $e->getMessage();
    }
    echo $name, ': ', json_encode($r), "\n";
}
// A callback that throws stops the matching.
try {
    preg_replace_callback('/\d/', function ($m) {
        static $n = 0;
        if (++$n == 2) {
            throw new Exception("stop");
        }
        echo "cb {$m[0]}\n";
        return 'x';
    }, 'a1b2c3');
} catch (Exception $e) {
    echo $e->getMessage(), "\n";
}
