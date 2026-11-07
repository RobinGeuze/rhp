class Collator
{
    // %CONSTANTS Collator%
    public function __construct(string $locale) { __collator('__construct', $this, $locale); }
    public static function create(string $locale): ?Collator { return __collator('create', null, $locale); }
    public function compare(string $string1, string $string2): int|false { return __collator('compare', $this, $string1, $string2); }
    public function sort(array &$array, int $flags = Collator::SORT_REGULAR): bool { return __collator('sort', $this, [&$array], $flags); }
    public function sortWithSortKeys(array &$array): bool { return __collator('sortWithSortKeys', $this, [&$array]); }
    public function asort(array &$array, int $flags = Collator::SORT_REGULAR): bool { return __collator('asort', $this, [&$array], $flags); }
    public function getAttribute(int $attribute): int|false { return __collator('getAttribute', $this, $attribute); }
    public function setAttribute(int $attribute, int $value): bool { return __collator('setAttribute', $this, $attribute, $value); }
    public function getStrength(): int { return __collator('getStrength', $this); }
    public function setStrength(int $strength): true { return __collator('setStrength', $this, $strength); }
    public function getLocale(int $type): string|false { return __collator('getLocale', $this, $type); }
    public function getErrorCode(): int|false { return __collator('getErrorCode', $this); }
    public function getErrorMessage(): string|false { return __collator('getErrorMessage', $this); }
    public function getSortKey(string $string): string|false { return __collator('getSortKey', $this, $string); }
}

class NumberFormatter
{
    // %CONSTANTS NumberFormatter%
    public function __construct(string $locale, int $style, ?string $pattern = null) { __numfmt('__construct', $this, $locale, $style, $pattern); }
    public static function create(string $locale, int $style, ?string $pattern = null): ?NumberFormatter { return __numfmt('create', null, $locale, $style, $pattern); }
    public function format(int|float $num, int $type = NumberFormatter::TYPE_DEFAULT): string|false { return __numfmt('format', $this, $num, $type); }
    public function parse(string $string, int $type = NumberFormatter::TYPE_DOUBLE, &$offset = null): int|float|false { return __numfmt('parse', $this, $string, $type, [&$offset]); }
    public function formatCurrency(float $amount, string $currency): string|false { return __numfmt('formatCurrency', $this, $amount, $currency); }
    public function parseCurrency(string $string, &$currency, &$offset = null): float|false { return __numfmt('parseCurrency', $this, $string, [&$currency], [&$offset]); }
    public function setAttribute(int $attribute, int|float $value): bool { return __numfmt('setAttribute', $this, $attribute, $value); }
    public function getAttribute(int $attribute): int|float|false { return __numfmt('getAttribute', $this, $attribute); }
    public function setTextAttribute(int $attribute, string $value): bool { return __numfmt('setTextAttribute', $this, $attribute, $value); }
    public function getTextAttribute(int $attribute): string|false { return __numfmt('getTextAttribute', $this, $attribute); }
    public function setSymbol(int $symbol, string $value): bool { return __numfmt('setSymbol', $this, $symbol, $value); }
    public function getSymbol(int $symbol): string|false { return __numfmt('getSymbol', $this, $symbol); }
    public function setPattern(string $pattern): bool { return __numfmt('setPattern', $this, $pattern); }
    public function getPattern(): string|false { return __numfmt('getPattern', $this); }
    public function getLocale(int $type = ULOC_ACTUAL_LOCALE): string|false { return __numfmt('getLocale', $this, $type); }
    public function getErrorCode(): int { return __numfmt('getErrorCode', $this); }
    public function getErrorMessage(): string { return __numfmt('getErrorMessage', $this); }
}


class Normalizer {
    const FORM_D = 4;
    const NFD = 4;
    const FORM_KD = 8;
    const NFKD = 8;
    const FORM_C = 16;
    const NFC = 16;
    const FORM_KC = 32;
    const NFKC = 32;
    const FORM_KC_CF = 48;
    const NFKC_CF = 48;
    public static function normalize(string $string, int $form = 16): string|false { return __normalizer('normalize', $string, $form); }
    public static function isNormalized(string $string, int $form = 16): bool { return __normalizer('isNormalized', $string, $form); }
    public static function getRawDecomposition(string $string, int $form = 16): ?string { return __normalizer('getRawDecomposition', $string, $form); }
}

class Locale
{
    // %CONSTANTS Locale%
    public static function getDefault(): string { return __locale('getDefault'); }
    public static function setDefault(string $locale): true { return __locale('setDefault', $locale); }
    public static function getPrimaryLanguage(string $locale): ?string { return __locale('getPrimaryLanguage', $locale); }
    public static function getScript(string $locale): ?string { return __locale('getScript', $locale); }
    public static function getRegion(string $locale): ?string { return __locale('getRegion', $locale); }
    public static function getKeywords(string $locale): array|false|null { return __locale('getKeywords', $locale); }
    public static function getDisplayScript(string $locale, ?string $displayLocale = null): string|false { return __locale('getDisplayScript', $locale, $displayLocale); }
    public static function getDisplayRegion(string $locale, ?string $displayLocale = null): string|false { return __locale('getDisplayRegion', $locale, $displayLocale); }
    public static function getDisplayName(string $locale, ?string $displayLocale = null): string|false { return __locale('getDisplayName', $locale, $displayLocale); }
    public static function getDisplayLanguage(string $locale, ?string $displayLocale = null): string|false { return __locale('getDisplayLanguage', $locale, $displayLocale); }
    public static function getDisplayVariant(string $locale, ?string $displayLocale = null): string|false { return __locale('getDisplayVariant', $locale, $displayLocale); }
    public static function composeLocale(array $subtags): string|false { return __locale('composeLocale', $subtags); }
    public static function parseLocale(string $locale): ?array { return __locale('parseLocale', $locale); }
    public static function getAllVariants(string $locale): ?array { return __locale('getAllVariants', $locale); }
    public static function filterMatches(string $languageTag, string $locale, bool $canonicalize = false): ?bool { return __locale('filterMatches', $languageTag, $locale, $canonicalize); }
    public static function lookup(array $languageTag, string $locale, bool $canonicalize = false, ?string $defaultLocale = null): ?string { return __locale('lookup', $languageTag, $locale, $canonicalize, $defaultLocale); }
    public static function canonicalize(string $locale): ?string { return __locale('canonicalize', $locale); }
    public static function acceptFromHttp(string $header): string|false { return __locale('acceptFromHttp', $header); }
    public static function isRightToLeft(string $locale): bool { return __locale('isRightToLeft', $locale); }
    public static function addLikelySubtags(string $locale): string|false { return __locale('addLikelySubtags', $locale); }
    public static function minimizeSubtags(string $locale): string|false { return __locale('minimizeSubtags', $locale); }
}

class IntlException extends Exception
{
}
