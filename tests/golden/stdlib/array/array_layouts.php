<?php
// Which builtins return packed arrays (debug_zval_dump), as php lays them out.
function f() { return func_get_args(); }
$h = ['a' => 1, 'b' => 2, 3 => 'c'];
$l = [5, 3, 1, 3];
$cases1 = [
 'keys' => array_keys($h), 'values' => array_values($h), 'merge' => array_merge($l, $l), 'merge_h' => array_merge($h, $l),
 'range' => range(1, 3), 'explode' => explode(',', 'a,b'), 'slice' => array_slice($l, 1), 'slice_pk' => array_slice($l, 1, null, true),
 'filter' => array_filter($l), 'filter0' => array_filter([0, 1, 2]), 'map' => array_map(fn($x) => $x, $l), 'map_h' => array_map(fn($x) => $x, $h),
 'combine' => array_combine([0, 1], [1, 2]), 'fill_keys' => array_fill_keys([0, 1], 0), 'str_split' => str_split('abc'), 'flip' => array_flip([0 => 0, 1 => 1]),
 'compact' => (function() { $x = 1; return compact('x'); })(), 'fga' => f(1, 2), 'unique' => array_unique($l), 'reverse' => array_reverse($l), 'reverse_pk' => array_reverse($l, true),
 'pad' => array_pad($l, 6, 0), 'chunk' => array_chunk($l, 2), 'fill' => array_fill(5, 2, 0), 'fill0' => array_fill(0, 2, 0), 'diff' => array_diff($l, [3]), 'intersect' => array_intersect($l, [3]),
 'column' => array_column([[1], [2]], 0), 'count_values' => array_count_values($l), 'search_keys' => array_keys($l, 3), 'preg_split' => preg_split('/,/', 'a,b'),
 'preg_match' => (function() { preg_match('/(a)(b)/', 'ab', $m); return $m; })(), 'str_word' => str_word_count('a b', 1), 'sorted' => (function($l) { sort($l); return $l; })($l),
 'asorted' => (function($l) { asort($l); return $l; })($l), 'json' => json_decode('[1,2]', true), 'json_o' => json_decode('{"0":1,"1":2}', true), 'unser' => unserialize(serialize([1, 2])),
 'replace' => array_replace($l, [1 => 9]), 'splice' => (function($l) { array_splice($l, 1, 1); return $l; })($l), 'shifted' => (function($l) { array_shift($l); return $l; })($l),
 'unshifted' => (function($l) { array_unshift($l, 0); return $l; })($l), 'array_rand' => array_rand([1, 2, 3, 4], 2), 'getdate' => array_keys(getdate(0)), 'parse_str' => (function() { parse_str('a[]=1&a[]=2', $o); return $o['a']; })(),
 'str_getcsv' => str_getcsv('a,b', ',', '"', ''), 'array_walk' => (function($l) { array_walk($l, function(&$v) { $v++; }); return $l; })($l), 'spread' => [...$l], 'lit' => [1, 2, 3], 'lit_k' => [1 => 1, 2 => 2], 'neg' => [-1 => 1],
 'explode_lim' => explode(',', 'a,b,c', -1), 'mb' => mb_str_split('ab'), 'iter' => iterator_to_array(new ArrayIterator([1, 2])), 'gen' => iterator_to_array((function() { yield 1; yield 2; })()),
];
class P { public $a = 1; public $b = 2; }
$l = [5, 3, 1, 3]; $h = ['x' => 1, 'y' => 2];
$cases2 = [
 'merge_rec' => array_merge_recursive($l, $l), 'replace_rec' => array_replace_recursive($l, [9]), 'diff_key' => array_diff_key($l, [1 => 0]), 'isect_key' => array_intersect_key($l, [0 => 0, 1 => 0]),
 'filter_key' => array_filter($l, fn($k) => $k > 0, ARRAY_FILTER_USE_KEY), 'natsort' => (function($l) { natsort($l); return $l; })($l), 'usort' => (function($l) { usort($l, fn($a, $b) => $a <=> $b); return $l; })($l),
 'uasort' => (function($l) { uasort($l, fn($a, $b) => $a <=> $b); return $l; })($l), 'msort' => (function($l) { array_multisort($l); return $l; })($l), 'range_c' => range('a', 'c'), 'range_f' => range(0.5, 2.5),
 'range_rev' => range(3, 1), 'objvars' => get_object_vars(new P), 'cast' => (array) new P, 'cast_l' => (array) 'x', 'pma' => (function() { preg_match_all('/a/', 'aa', $m); return $m; })(),
 'pma0' => (function() { preg_match_all('/a/', 'aa', $m); return $m[0]; })(), 'grep' => preg_grep('/a/', ['a', 'b', 'a']), 'search' => array_keys($l, 3, true), 'sscanf' => sscanf('1 2', '%d %d'),
 'flip_s' => array_flip(['a', 'b']), 'combine_s' => array_combine(['a', 'b'], [1, 2]), 'fk_s' => array_fill_keys(['a', 'b'], 0), 'values_h' => array_values($h), 'keys_h' => array_keys($h),
 'merge_e' => array_merge(), 'merge_one' => array_merge($l), 'slice_h' => array_slice($h, 0, 1), 'chunk_pk' => array_chunk($l, 2, true), 'pad_neg' => array_pad($l, -6, 0),
 'diff_assoc' => array_diff_assoc($l, [5]), 'udiff' => array_udiff($l, [3], fn($a, $b) => $a <=> $b), 'shuffle' => (function($l) { shuffle($l); return $l; })($l), 'array_map2' => array_map(null, [1, 2], [3, 4]),
 'array_map_k' => array_map(fn($x) => $x, [3 => 1, 5 => 2]), 'implode_ex' => explode(' ', 'a b c'), 'str_word2' => str_word_count('ab cd', 2), 'array_key_exists' => array_slice([1 => 1, 2 => 2], 0),
 'count_chars' => count_chars('ab', 1), 'stat' => array_slice(stat(__FILE__), 0, 2), 'pathinfo' => pathinfo('/a/b.c'), 'parse_url' => parse_url('http://x/y'), 'localeconv' => array_keys(localeconv()),
 'iterator_k' => iterator_to_array(new ArrayIterator(['a' => 1])), 'ao' => (new ArrayObject([1, 2]))->getArrayCopy(), 'spl_fixed' => SplFixedArray::fromArray([1, 2])->toArray(),
 'debug_bt' => (function() { return array_keys(debug_backtrace()[0]); })(), 'get_defined' => (function() { $q = 1; return get_defined_vars(); })(), 'args_named' => (function(...$a) { return $a; })(1, 2),
 'args_named2' => (function(...$a) { return $a; })(x: 1), 'json_nested' => json_decode('{"a":[1,2]}', true)['a'], 'var_export' => eval('return ' . var_export([1, 2], true) . ';'),
 'unser_h' => unserialize('a:1:{s:1:"a";i:1;}'), 'array_rev_h' => array_reverse($h), 'array_rev_kh' => array_reverse([1 => 'a', 0 => 'b']), 'ksort' => (function($l) { ksort($l); return $l; })([2 => 1, 1 => 2]),
];
foreach ($cases1 + $cases2 as $n => $v) {
    ob_start();
    debug_zval_dump($v);
    $d = ob_get_clean();
    $size = is_array($v) ? count($v) : gettype($v);
    echo $n, ': ', $size, str_contains(strtok($d, "\n"), 'packed') ? ' packed' : '', "\n";
}
