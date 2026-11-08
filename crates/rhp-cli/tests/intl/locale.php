<?php
// Locale and locale_*: parsing, composing, display names, matching and
// the default locale.
var_dump(Locale::getDefault(), locale_get_default());
$locales = ['en_US', 'de-DE', 'sr_Latn_RS_REVISED', 'zh-Hant-TW', 'en-GB-oed', 'i-klingon', 'x-whatever',
    'de-DE-x-private-one', 'en_US@currency=EUR;calendar=buddhist', 'es-419', 'uk-ua_CALIFORNIA@currency=;currency=GRN',
    'sl-rozaj-biske-1994', 'hy-Latn-IT-arevela', 'ar', 'he_IL', 'qaa-Qaaa-QM-x-southern', '', 'root', 'zz', '-x-a',
    'en_US.UTF-8', 'en_us_posix', 'C', 'art-lojban', 'zh-min-nan', 'en-a-bbb-x-a-ccc', 'de__POSIX'];
foreach ($locales as $l) {
    echo "== $l\n";
    var_dump(Locale::getPrimaryLanguage($l), Locale::getScript($l), Locale::getRegion($l), Locale::getKeywords($l),
        Locale::canonicalize($l), Locale::parseLocale($l), Locale::getAllVariants($l), Locale::isRightToLeft($l),
        Locale::addLikelySubtags($l), Locale::minimizeSubtags($l));
    foreach (['en', 'de', 'fr', null] as $in) {
        var_dump(Locale::getDisplayName($l, $in), Locale::getDisplayLanguage($l, $in), Locale::getDisplayScript($l, $in),
            Locale::getDisplayRegion($l, $in), Locale::getDisplayVariant($l, $in));
    }
}
foreach ([['language' => 'en', 'region' => 'US'], ['language' => 'sr', 'script' => 'Latn', 'region' => 'RS', 'variant' => ['a', 'b'], 'private' => 'p1'],
          ['language' => 'de', 'variant0' => 'v0', 'variant2' => 'v2', 'private0' => 'x', 'extlang1' => 'e'],
          ['grandfathered' => 'i-klingon', 'language' => 'x'], ['region' => 'US'], ['language' => 1], [],
          ['language' => 'en', 'variant' => [1]], ['language' => 'en', 'extlang' => 5]] as $parts) {
    try { var_dump(Locale::composeLocale($parts), locale_compose($parts)); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    var_dump(intl_get_error_message());
}
foreach ([['de-DEVA', 'de-DE'], ['de-DE-1996', 'de-DE'], ['de_DE', 'DE'], ['de', '*'], ['sl_IT', 'sl_IT_nedis'], ['', 'en'], ['en', ''], ['de@x=y', 'de']] as [$t, $r]) {
    var_dump(Locale::filterMatches($t, $r), Locale::filterMatches($t, $r, true));
}
$tags = ['de-DEVA', 'de-DE-1996', 'de', 'de-De', 'en_US', 'zh-Hant'];
foreach (['de-DE-1996-x-prv1-prv2', 'de-de', 'de_DE', 'en_US_POSIX', 'fr', 'zh-hant-tw', ''] as $r) {
    var_dump(Locale::lookup($tags, $r), Locale::lookup($tags, $r, true), Locale::lookup($tags, $r, false, 'en'));
}
var_dump(Locale::lookup([], 'en'), Locale::lookup([''], 'en'), intl_get_error_message());
foreach ([fn() => Locale::lookup([1], 'en'), fn() => Locale::lookup(["a\0"], 'en'), fn() => locale_get_region("a\0b"),
          fn() => Locale::getRegion(str_repeat('a', 200)), fn() => Locale::getDisplayName(str_repeat('a', 200)),
          fn() => Locale::parseLocale(str_repeat('a', 157)), fn() => Locale::acceptFromHttp(str_repeat('a', 200)),
          fn() => Locale::filterMatches(str_repeat('a', 200), 'en')] as $fn) {
    try { var_dump($fn()); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    var_dump(intl_get_error_message());
}
foreach (['de-DE,de;q=0.9,en;q=0.8', 'xx', 'en-US;q=0.5, fr', '*', '', 'zh-Hant-TW, ja;q=0.1'] as $h) {
    var_dump(Locale::acceptFromHttp($h), locale_accept_from_http($h));
}
var_dump(Locale::setDefault('fr_CA'), Locale::getDefault(), ini_get('intl.default_locale'),
    (new NumberFormatter('', NumberFormatter::DECIMAL))->format(1234.5), (new Collator(''))->getLocale(Locale::ACTUAL_LOCALE),
    Locale::getDisplayName('de'), Locale::getRegion(''));
var_dump(locale_set_default(''), Locale::getDefault());
var_dump(ini_set('intl.default_locale', ''), Locale::getDefault());
var_dump(Locale::ACTUAL_LOCALE, Locale::DEFAULT_LOCALE, Locale::LANG_TAG, Locale::PRIVATE_TAG, Locale::GRANDFATHERED_LANG_TAG);
