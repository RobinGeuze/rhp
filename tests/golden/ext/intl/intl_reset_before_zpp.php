<?php
// Most intl entry points reset the global error before zpp, so a TypeError
// or ArgumentCountError leaves it clear; a few keep the old error.
ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
function fail() { msgfmt_create('en', '{0,'); return intl_get_error_code(); }
$mf = new MessageFormatter('en', 'x');
$rb = new ResourceBundle('en', null);
$calls = [
    'Locale::getDisplayName' => fn() => Locale::getDisplayName([]),
    'Locale::lookup' => fn() => Locale::lookup('x', 'en'),
    'Locale::canonicalize' => fn() => Locale::canonicalize([]),
    'Normalizer::normalize' => fn() => Normalizer::normalize([]),
    'Collator::create' => fn() => Collator::create([]),
    'new Collator' => fn() => new Collator([]),
    'IntlTimeZone::createTimeZone' => fn() => IntlTimeZone::createTimeZone([]),
    'intltz_get_canonical_id' => fn() => intltz_get_canonical_id([]),
    'IntlCalendar::createInstance' => fn() => IntlCalendar::createInstance([]),
    'intlcal_get_keyword_values_for_locale' => fn() => intlcal_get_keyword_values_for_locale([], 'en', true),
    'IntlGregorianCalendar::createFromDate' => fn() => IntlGregorianCalendar::createFromDate([], 1, 1),
    'new MessageFormatter' => fn() => new MessageFormatter([], ''),
    'datefmt_create' => fn() => datefmt_create('en', []),
    'IntlDatePatternGenerator::create' => fn() => IntlDatePatternGenerator::create([]),
    'resourcebundle_create' => fn() => resourcebundle_create('en', []),
    'Transliterator::listIDs' => fn() => Transliterator::listIDs(1),
    'IntlBreakIterator::createCodePointInstance' => fn() => IntlBreakIterator::createCodePointInstance(1),
    'NumberFormatter::create' => fn() => NumberFormatter::create([], 1),
    'Transliterator::create' => fn() => Transliterator::create([]),
    'IntlBreakIterator::createWordInstance' => fn() => IntlBreakIterator::createWordInstance([]),
    'IntlChar::ord' => fn() => IntlChar::ord([]),
    'Spoofchecker::isSuspicious' => fn() => (new Spoofchecker)->isSuspicious([]),
    'UConverter::__construct' => fn() => new UConverter([]),
    // native procedural functions
    'collator_create' => fn() => collator_create([]),
    'numfmt_create' => fn() => numfmt_create([], 1),
    'locale_get_display_name' => fn() => locale_get_display_name([]),
    'normalizer_normalize' => fn() => normalizer_normalize([]),
    'idn_to_ascii' => fn() => idn_to_ascii([]),
    // these keep the error
    'grapheme_strlen' => fn() => grapheme_strlen([]),
    'locale_get_default' => fn() => locale_get_default(1),
    'Locale::setDefault' => fn() => Locale::setDefault([]),
    'msgfmt_format_message' => fn() => msgfmt_format_message([], '', []),
    'MessageFormatter::getErrorCode' => fn() => $mf->getErrorCode(1),
    'UConverter::transcode' => fn() => UConverter::transcode([], 'a', 'b'),
    'IntlTimeZone::getWindowsID' => fn() => IntlTimeZone::getWindowsID([]),
    'ResourceBundle::get' => fn() => $rb->get([]),
];
foreach ($calls as $name => $f) {
    $before = fail();
    try {
        $f();
        echo "$name: no error";
    } catch (Throwable $e) {
        echo "$name: ", get_class($e);
    }
    echo " ($before -> ", intl_get_error_code(), ")\n";
}

// constant() raises a built-in class constant's deprecation, as a fetch does
var_dump(constant('NumberFormatter::TYPE_CURRENCY'), constant('U_MULTIPLE_DECIMAL_SEPERATORS'));
var_dump(defined('NumberFormatter::TYPE_CURRENCY'), defined('U_MULTIPLE_DECIMAL_SEPERATORS'));
var_dump(constant('DateTimeInterface::RFC7231'));
