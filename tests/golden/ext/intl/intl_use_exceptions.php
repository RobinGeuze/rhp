<?php
// intl.use_exceptions and intl.error_level: a global error becomes a
// warning and an IntlException; the function still sets the error first.
ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
$calls = [
    fn() => grapheme_substr("\xc3", 0),
    fn() => normalizer_is_normalized("\xc3"),
    fn() => normalizer_get_raw_decomposition("ab"),
    fn() => grapheme_extract("abc", 1, 0, 9),
    fn() => numfmt_create('en', 1234),
    fn() => NumberFormatter::create('en', 1234),
    fn() => collator_create(str_repeat('x', 200)),
    fn() => Locale::getRegion(str_repeat('x', 200)),
    fn() => Locale::composeLocale(['language' => 1]),
    fn() => (new NumberFormatter('en', NumberFormatter::DECIMAL))->getSymbol(-1),
];
foreach ([[0, 0], [0, 1], [2, 0], [2, 1]] as [$level, $exc]) {
    echo "== error_level=$level use_exceptions=$exc\n";
    ini_set('intl.error_level', $level);
    ini_set('intl.use_exceptions', $exc);
    foreach ($calls as $fn) {
        try {
            var_dump($fn());
        } catch (IntlException $e) {
            echo get_class($e), ': ', $e->getMessage(), ' (', $e->getCode(), ")\n";
        }
        echo intl_get_error_code(), ' ', intl_get_error_message(), "\n";
    }
}
// constructors always throw, whatever the settings
ini_set('intl.use_exceptions', 0);
foreach ([fn() => new NumberFormatter('en', 1234), fn() => new Collator(str_repeat('x', 200))] as $fn) {
    try { $fn(); } catch (IntlException $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    echo intl_get_error_message(), "\n";
}
var_dump(get_parent_class('IntlException'), ini_get('intl.use_exceptions'), ini_get('intl.error_level'));
