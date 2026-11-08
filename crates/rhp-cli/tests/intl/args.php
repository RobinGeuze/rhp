<?php
// The procedural functions parse their arguments after the object as
// php's zpp does: coercions, deprecations, TypeErrors, argument numbers.
$c = new Collator('en'); $f = numfmt_create('en', 1); $o = new stdClass;
$arr = ['b', 'a'];
foreach ([
    fn() => collator_set_attribute($c, "abc", 1), fn() => collator_set_attribute($c, 1, "x"),
    fn() => collator_set_attribute($c, 1.5, 1), fn() => collator_set_attribute($c, 5, 1e30),
    fn() => collator_set_strength($c, "1abc"), fn() => collator_get_locale($c, null),
    fn() => collator_asort($c, $arr, "x"), fn() => collator_sort($c, $arr, 1.5),
    fn() => collator_compare($c, $o, 'a'), fn() => collator_get_sort_key($c, $o),
    fn() => numfmt_format($f, "abc"), fn() => numfmt_format($f, "1.5"), fn() => numfmt_format($f, true),
    fn() => numfmt_format($f, null), fn() => numfmt_format($f, 1, "x"),
    fn() => numfmt_set_attribute($f, "x", 1), fn() => numfmt_set_attribute($f, 1, "x"),
    fn() => numfmt_set_attribute($f, NumberFormatter::FRACTION_DIGITS, "2"),
    fn() => numfmt_get_symbol($f, "x"), fn() => numfmt_create('en', "x"), fn() => numfmt_create('en', 1.5),
    fn() => numfmt_parse($f, "1", "x"), fn() => numfmt_format_currency($f, "x", "USD"),
    fn() => numfmt_format_currency($f, "2.5", "USD"), fn() => numfmt_get_locale($f, "x"),
    fn() => numfmt_set_pattern($f, $o),
] as $i => $fn) {
    echo $i, ': ';
    try { var_dump($fn()); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}
