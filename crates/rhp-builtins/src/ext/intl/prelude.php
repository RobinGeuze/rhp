
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
