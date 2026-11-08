class Collator
{
    // %CONSTANTS Collator%
    #[\RhpBeforeZpp] public function __construct(string $locale) { __collator('__construct', $this, $locale); }
    #[\RhpBeforeZpp] public static function create(string $locale): ?Collator { return __collator('create', null, $locale); }
    #[\RhpBeforeZpp] public function compare(string $string1, string $string2): int|false { return __collator('compare', $this, $string1, $string2); }
    #[\RhpBeforeZpp] public function sort(array &$array, int $flags = Collator::SORT_REGULAR): bool { return __collator('sort', $this, [&$array], $flags); }
    #[\RhpBeforeZpp] public function sortWithSortKeys(array &$array): bool { return __collator('sortWithSortKeys', $this, [&$array]); }
    #[\RhpBeforeZpp] public function asort(array &$array, int $flags = Collator::SORT_REGULAR): bool { return __collator('asort', $this, [&$array], $flags); }
    #[\RhpBeforeZpp] public function getAttribute(int $attribute): int|false { return __collator('getAttribute', $this, $attribute); }
    #[\RhpBeforeZpp] public function setAttribute(int $attribute, int $value): bool { return __collator('setAttribute', $this, $attribute, $value); }
    #[\RhpBeforeZpp] public function getStrength(): int { return __collator('getStrength', $this); }
    #[\RhpBeforeZpp] public function setStrength(int $strength): true { return __collator('setStrength', $this, $strength); }
    #[\RhpBeforeZpp] public function getLocale(int $type): string|false { return __collator('getLocale', $this, $type); }
    #[\RhpBeforeZpp] public function getErrorCode(): int|false { return __collator('getErrorCode', $this); }
    #[\RhpBeforeZpp] public function getErrorMessage(): string|false { return __collator('getErrorMessage', $this); }
    #[\RhpBeforeZpp] public function getSortKey(string $string): string|false { return __collator('getSortKey', $this, $string); }
}

class NumberFormatter
{
    // %CONSTANTS NumberFormatter%
    #[\RhpBeforeZpp] public function __construct(string $locale, int $style, ?string $pattern = null) { __numfmt('__construct', $this, $locale, $style, $pattern); }
    #[\RhpBeforeZpp] public static function create(string $locale, int $style, ?string $pattern = null): ?NumberFormatter { return __numfmt('create', null, $locale, $style, $pattern); }
    #[\RhpBeforeZpp] public function format(int|float $num, int $type = NumberFormatter::TYPE_DEFAULT): string|false { return __numfmt('format', $this, $num, $type); }
    #[\RhpBeforeZpp] public function parse(string $string, int $type = NumberFormatter::TYPE_DOUBLE, &$offset = null): int|float|false { return __numfmt('parse', $this, $string, $type, [&$offset]); }
    #[\RhpBeforeZpp] public function formatCurrency(float $amount, string $currency): string|false { return __numfmt('formatCurrency', $this, $amount, $currency); }
    #[\RhpBeforeZpp] public function parseCurrency(string $string, &$currency, &$offset = null): float|false { return __numfmt('parseCurrency', $this, $string, [&$currency], [&$offset]); }
    #[\RhpBeforeZpp] public function setAttribute(int $attribute, int|float $value): bool { return __numfmt('setAttribute', $this, $attribute, $value); }
    #[\RhpBeforeZpp] public function getAttribute(int $attribute): int|float|false { return __numfmt('getAttribute', $this, $attribute); }
    #[\RhpBeforeZpp] public function setTextAttribute(int $attribute, string $value): bool { return __numfmt('setTextAttribute', $this, $attribute, $value); }
    #[\RhpBeforeZpp] public function getTextAttribute(int $attribute): string|false { return __numfmt('getTextAttribute', $this, $attribute); }
    #[\RhpBeforeZpp] public function setSymbol(int $symbol, string $value): bool { return __numfmt('setSymbol', $this, $symbol, $value); }
    #[\RhpBeforeZpp] public function getSymbol(int $symbol): string|false { return __numfmt('getSymbol', $this, $symbol); }
    #[\RhpBeforeZpp] public function setPattern(string $pattern): bool { return __numfmt('setPattern', $this, $pattern); }
    #[\RhpBeforeZpp] public function getPattern(): string|false { return __numfmt('getPattern', $this); }
    #[\RhpBeforeZpp] public function getLocale(int $type = ULOC_ACTUAL_LOCALE): string|false { return __numfmt('getLocale', $this, $type); }
    #[\RhpBeforeZpp] public function getErrorCode(): int { return __numfmt('getErrorCode', $this); }
    #[\RhpBeforeZpp] public function getErrorMessage(): string { return __numfmt('getErrorMessage', $this); }
}

// %CLASSES AFTER NumberFormatter%


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
    #[\RhpBeforeZpp] public static function normalize(string $string, int $form = 16): string|false { return __normalizer('normalize', $string, $form); }
    #[\RhpBeforeZpp] public static function isNormalized(string $string, int $form = 16): bool { return __normalizer('isNormalized', $string, $form); }
    #[\RhpBeforeZpp] public static function getRawDecomposition(string $string, int $form = 16): ?string { return __normalizer('getRawDecomposition', $string, $form); }
}

class Locale
{
    // %CONSTANTS Locale%
    public static function getDefault(): string { return __locale('getDefault'); }
    public static function setDefault(string $locale): true { return __locale('setDefault', $locale); }
    #[\RhpBeforeZpp] public static function getPrimaryLanguage(string $locale): ?string { return __locale('getPrimaryLanguage', $locale); }
    #[\RhpBeforeZpp] public static function getScript(string $locale): ?string { return __locale('getScript', $locale); }
    #[\RhpBeforeZpp] public static function getRegion(string $locale): ?string { return __locale('getRegion', $locale); }
    #[\RhpBeforeZpp] public static function getKeywords(string $locale): array|false|null { return __locale('getKeywords', $locale); }
    #[\RhpBeforeZpp] public static function getDisplayScript(string $locale, ?string $displayLocale = null): string|false { return __locale('getDisplayScript', $locale, $displayLocale); }
    #[\RhpBeforeZpp] public static function getDisplayRegion(string $locale, ?string $displayLocale = null): string|false { return __locale('getDisplayRegion', $locale, $displayLocale); }
    #[\RhpBeforeZpp] public static function getDisplayName(string $locale, ?string $displayLocale = null): string|false { return __locale('getDisplayName', $locale, $displayLocale); }
    #[\RhpBeforeZpp] public static function getDisplayLanguage(string $locale, ?string $displayLocale = null): string|false { return __locale('getDisplayLanguage', $locale, $displayLocale); }
    #[\RhpBeforeZpp] public static function getDisplayVariant(string $locale, ?string $displayLocale = null): string|false { return __locale('getDisplayVariant', $locale, $displayLocale); }
    #[\RhpBeforeZpp] public static function composeLocale(array $subtags): string|false { return __locale('composeLocale', $subtags); }
    #[\RhpBeforeZpp] public static function parseLocale(string $locale): ?array { return __locale('parseLocale', $locale); }
    #[\RhpBeforeZpp] public static function getAllVariants(string $locale): ?array { return __locale('getAllVariants', $locale); }
    #[\RhpBeforeZpp] public static function filterMatches(string $languageTag, string $locale, bool $canonicalize = false): ?bool { return __locale('filterMatches', $languageTag, $locale, $canonicalize); }
    #[\RhpBeforeZpp] public static function lookup(array $languageTag, string $locale, bool $canonicalize = false, ?string $defaultLocale = null): ?string { return __locale('lookup', $languageTag, $locale, $canonicalize, $defaultLocale); }
    #[\RhpBeforeZpp] public static function canonicalize(string $locale): ?string { return __locale('canonicalize', $locale); }
    public static function acceptFromHttp(string $header): string|false { return __locale('acceptFromHttp', $header); }
    public static function isRightToLeft(string $locale): bool { return __locale('isRightToLeft', $locale); }
    public static function addLikelySubtags(string $locale): string|false { return __locale('addLikelySubtags', $locale); }
    public static function minimizeSubtags(string $locale): string|false { return __locale('minimizeSubtags', $locale); }
}

// %CLASSES BEFORE IntlException%

class IntlException extends Exception
{
}

// %CLASSES AFTER IntlException%
