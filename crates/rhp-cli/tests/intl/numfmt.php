<?php
// NumberFormatter and numfmt_*: styles, attributes, symbols, patterns,
// parsing, currencies and errors, as Laravel's Number and Symfony use them.
foreach (['en_US', 'de_DE', 'fr_FR', 'ja_JP', 'ar_EG', 'hi_IN', ''] as $loc) {
    foreach ([NumberFormatter::DECIMAL, NumberFormatter::CURRENCY, NumberFormatter::PERCENT,
              NumberFormatter::SCIENTIFIC, NumberFormatter::SPELLOUT, NumberFormatter::ORDINAL,
              NumberFormatter::DURATION, NumberFormatter::DECIMAL_COMPACT_SHORT,
              NumberFormatter::CURRENCY_ACCOUNTING] as $style) {
        $f = new NumberFormatter($loc, $style);
        foreach ([0, -1, 1234.5, 1234567.891, -0.005, 1e20, PHP_INT_MAX, NAN, INF] as $n) {
            echo "$loc/$style ", var_export($f->format($n), true), ' ',
                var_export(numfmt_format($f, $n, NumberFormatter::TYPE_INT32), true), "\n";
        }
        echo $f->getPattern(), ' ', $f->getLocale(), ' ', $f->getLocale(Locale::VALID_LOCALE), "\n";
    }
}
// Laravel's Number::format / percentage / currency / spell / ordinal / fileSize
$f = new NumberFormatter('en', NumberFormatter::DECIMAL);
$f->setAttribute(NumberFormatter::MAX_FRACTION_DIGITS, 2);
var_dump($f->format(1234.5678), $f->getAttribute(NumberFormatter::MAX_FRACTION_DIGITS));
$f->setAttribute(NumberFormatter::FRACTION_DIGITS, 3);
var_dump($f->format(1.5), $f->getAttribute(NumberFormatter::ROUNDING_INCREMENT));
$p = new NumberFormatter('en', NumberFormatter::PERCENT);
$p->setAttribute(NumberFormatter::MAX_FRACTION_DIGITS, 1);
var_dump($p->format(12.345 / 100));
$c = new NumberFormatter('en', NumberFormatter::CURRENCY);
var_dump($c->formatCurrency(5.5, 'USD'), $c->formatCurrency(-1234, 'EUR'), $c->formatCurrency(1, 'XYZ'));
$c->setAttribute(NumberFormatter::FRACTION_DIGITS, 0);
var_dump($c->formatCurrency(5.5, 'USD'));
$s = new NumberFormatter('en', NumberFormatter::SPELLOUT);
var_dump($s->format(102.5), $s->getTextAttribute(NumberFormatter::DEFAULT_RULESET));
$s->setTextAttribute(NumberFormatter::DEFAULT_RULESET, '%spellout-ordinal');
var_dump($s->format(21), $s->getTextAttribute(NumberFormatter::PUBLIC_RULESETS));
var_dump($s->setTextAttribute(NumberFormatter::DEFAULT_RULESET, '%nope'), $s->getErrorCode(), $s->getErrorMessage(), intl_get_error_message());
$o = new NumberFormatter('en', NumberFormatter::ORDINAL);
foreach ([1, 2, 3, 11, 22, 103] as $n) echo $o->format($n), "\n";
// attributes, symbols, text attributes
$f = numfmt_create('de_CH', NumberFormatter::DECIMAL);
for ($a = 0; $a <= 20; $a++) var_dump($a, numfmt_get_attribute($f, $a));
for ($a = 0; $a <= 7; $a++) var_dump(numfmt_get_text_attribute($f, $a));
for ($s = -1; $s <= 30; $s++) var_dump(numfmt_get_symbol($f, $s));
var_dump($f->setSymbol(NumberFormatter::GROUPING_SEPARATOR_SYMBOL, '_'), $f->format(1234567));
var_dump($f->setAttribute(NumberFormatter::ROUNDING_INCREMENT, 0.5), $f->format(1.3), $f->setAttribute(999, 1));
var_dump($f->setPattern('#,##0.00 ¤'), $f->getPattern(), $f->format(3));
var_dump($f->setPattern("#,##0.0#\u{0}'"), $f->getErrorMessage(), intl_get_error_code());
// parse
$f = new NumberFormatter('en_US', NumberFormatter::DECIMAL);
foreach (['1,234.5', '-12', 'abc', '9223372036854775808', '12abc', '', '١٢٣'] as $str) {
    var_dump($f->parse($str), $f->parse($str, NumberFormatter::TYPE_INT32), $f->parse($str, NumberFormatter::TYPE_INT64));
    $pos = 0; var_dump(numfmt_parse($f, "xx $str", NumberFormatter::TYPE_DOUBLE, $pos), $pos);
}
$pos = 3; var_dump($f->parse('€€1€', NumberFormatter::TYPE_DOUBLE, $pos), $pos);
$pos = -5; var_dump($f->parse('12', NumberFormatter::TYPE_DOUBLE, $pos), $pos, $f->getErrorMessage());
$c = new NumberFormatter('en_US', NumberFormatter::CURRENCY);
var_dump($c->parseCurrency('$1,234.50', $cur), $cur);
$pos = 1; var_dump(numfmt_parse_currency($c, 'x€12.00', $cur, $pos), $cur, $pos);
var_dump($c->parseCurrency('nope', $cur), $cur, intl_get_error_message());
// errors
foreach ([fn() => new NumberFormatter('en', 1234), fn() => NumberFormatter::create('en', 1234),
          fn() => numfmt_create('xx_invalid', 1), fn() => new NumberFormatter('', NumberFormatter::PATTERN_DECIMAL, '0.0.0'),
          fn() => numfmt_create(str_repeat('a', 200), 1), fn() => $f->format(1, 99), fn() => numfmt_format($f, 1, 4),
          fn() => $f->parse('1', NumberFormatter::TYPE_CURRENCY), fn() => numfmt_get_locale('x'),
          fn() => (new ReflectionClass('NumberFormatter'))->newInstanceWithoutConstructor()->format(1)] as $fn) {
    try { var_dump($fn()); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    var_dump(intl_get_error_code(), intl_get_error_message());
}
$g = clone $f; var_dump($g->format(2.5), $g == $f);
try { $f->__construct('en', 1); } catch (Error $e) { echo $e->getMessage(), "\n"; }
class MyFmt extends NumberFormatter { function __construct() { parent::__construct('fr', self::DECIMAL); } }
var_dump((new MyFmt)->format(1234.5), get_class(clone new MyFmt));
ini_set('intl.use_exceptions', 1);
try { numfmt_create('en', 99); } catch (IntlException $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
ini_set('intl.use_exceptions', 0);
var_dump(NumberFormatter::ROUND_HALFEVEN, NumberFormatter::TYPE_DOUBLE, NumberFormatter::CURRENCY_STANDARD);
var_dump(NumberFormatter::TYPE_CURRENCY);
