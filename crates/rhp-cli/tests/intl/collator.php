<?php
// Collator and collator_*: comparisons, the three sorts, sort keys,
// attributes and errors.
$words = ['côte', 'cote', 'Côté', 'coté', 'a', 'B', 'b', 'A', '10', '9', '1e1', ' 2', '2 ', 'x', 'Ä', 'z', 'ö',
          'é', 'e', 'ß', 'ss', 1, 2.5, true, null, "\u{1F600}", '', '-1', '+3', '0x1A', '.5', '1.'];
foreach (['en_US', 'sv_SE', 'de_DE@collation=phonebook', 'fr_CA', 'ja', 'root', ''] as $loc) {
    $c = new Collator($loc);
    echo $loc, ' ', $c->getLocale(Locale::ACTUAL_LOCALE), ' ', $c->getLocale(Locale::VALID_LOCALE), "\n";
    foreach ([Collator::SORT_REGULAR, Collator::SORT_STRING, Collator::SORT_NUMERIC] as $flag) {
        $a = $words; var_dump($c->sort($a, $flag)); echo json_encode($a), "\n";
        $a = $words; var_dump(collator_asort($c, $a, $flag)); echo json_encode($a), "\n";
    }
    $a = $words; var_dump($c->sortWithSortKeys($a)); echo json_encode($a), "\n";
    foreach ([['a', 'b'], ['b', 'a'], ['a', 'A'], ['côte', 'coté'], ['10', '9'], ['', '']] as [$x, $y]) {
        echo $c->compare($x, $y), ' ', bin2hex($c->getSortKey($x)), "\n";
    }
}
$c = collator_create('en_US');
foreach ([Collator::FRENCH_COLLATION, Collator::ALTERNATE_HANDLING, Collator::CASE_FIRST, Collator::CASE_LEVEL,
          Collator::NORMALIZATION_MODE, Collator::STRENGTH, Collator::HIRAGANA_QUATERNARY_MODE,
          Collator::NUMERIC_COLLATION, 99] as $attr) {
    var_dump($c->getAttribute($attr));
}
var_dump($c->setAttribute(Collator::NUMERIC_COLLATION, Collator::ON), $c->compare('10', '9'));
var_dump($c->setAttribute(99, 1), $c->getErrorCode(), $c->getErrorMessage(), intl_get_error_message());
var_dump($c->setStrength(Collator::PRIMARY), $c->getStrength(), $c->compare('a', 'Á'));
var_dump(collator_compare($c, "\xff", 'a'), collator_get_error_message($c), collator_get_sort_key($c, "\xfe"));
$a = ['k' => "\xff", 'j' => 'a']; var_dump($c->asort($a), $a, intl_get_error_message());
$a = ['b' => 'x', 3 => 'a', 'c' => new ArrayObject([]), 'd' => 'A']; 
try { var_dump($c->sort($a)); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
class S { function __toString(): string { return 'm'; } }
$a = ['z', new S, 'a', [1], 5]; var_dump($c->sort($a, Collator::SORT_REGULAR)); var_dump($a);
$x = 'q'; $a = ['b', &$x, 'a']; var_dump($c->sort($a), $a);
$a = []; var_dump($c->sort($a), $c->sortWithSortKeys($a), $a);
foreach ([fn() => new Collator(str_repeat('x', 200)), fn() => collator_create(str_repeat('x', 200)),
          fn() => collator_compare('x', 'a', 'b'), fn() => clone $c,
          fn() => (new ReflectionClass('Collator'))->newInstanceWithoutConstructor()->compare('a', 'b'),
          fn() => (new ReflectionClass('Collator'))->newInstanceWithoutConstructor()->sort($a),
          fn() => $c->__construct('en')] as $fn) {
    try { var_dump($fn()); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    var_dump(intl_get_error_code(), intl_get_error_message());
}
var_dump(Collator::SORT_NUMERIC, Collator::DEFAULT_VALUE, Collator::IDENTICAL, ULOC_VALID_LOCALE);
