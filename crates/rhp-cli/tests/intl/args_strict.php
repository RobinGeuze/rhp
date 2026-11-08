<?php
declare(strict_types=1);
$c = new Collator('en'); $f = numfmt_create('en', 1);
foreach ([
    fn() => collator_set_attribute($c, "1", 1), fn() => collator_set_strength($c, 1.0),
    fn() => collator_compare($c, 1, 2), fn() => numfmt_format($f, "1"), fn() => numfmt_format($f, 1),
    fn() => numfmt_format_currency($f, 1, 'USD'), fn() => numfmt_format_currency($f, "1", 'USD'),
    fn() => numfmt_set_attribute($f, 1, true), fn() => numfmt_create('en', "1"),
    fn() => numfmt_set_pattern($f, 5), fn() => numfmt_format($f, 1.5, 1),
] as $i => $fn) {
    echo $i, ': ';
    try { var_dump($fn()); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}
