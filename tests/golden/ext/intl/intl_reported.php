<?php
// intl is reported as loaded, with all of its classes, functions and ini
// entries (code that checks extension_loaded('intl') uses them).
var_dump(extension_loaded('intl'));
$e = new ReflectionExtension('intl');
$c = $e->getClassNames();
sort($c);
echo implode("\n", $c), "\n";
echo count(get_extension_funcs('intl')), "\n";
print_r(ini_get_all('intl'));

// ResourceBundle answers $rb[...] natively (no ArrayAccess).
$rb = new ResourceBundle('de_DE', 'ICUDATA-region');
var_dump($rb['Countries']['DE'], $rb['nope'] ?? 'default', count($rb) > 0);
try {
    isset($rb['x']);
} catch (Error $ex) {
    echo $ex->getMessage(), "\n";
}

// IntlChar, IntlListFormatter, UConverter, break iterators
var_dump(IntlChar::charName('A'), IntlChar::ord('é'), IntlChar::chr(0x1F600));
echo (new IntlListFormatter('en', IntlListFormatter::TYPE_OR))->format(['a', 'b', 'c']), "\n";
var_dump(UConverter::transcode("caf\xe9", 'utf-8', 'latin1'));
$bi = IntlBreakIterator::createWordInstance('en');
$bi->setText('Hello big world');
foreach ($bi->getPartsIterator(IntlPartsIterator::KEY_LEFT) as $k => $part) {
    echo "$k:[$part] ";
}
echo "\n";
$fmt = new IntlDateFormatter('en_US', IntlDateFormatter::LONG, IntlDateFormatter::NONE, 'UTC');
echo $fmt->format(0), "\n";
