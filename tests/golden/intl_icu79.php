<?php
// php 8.5.12 grapheme_* search fixes and ICU 79.1 (Unicode 18) data.
$j = fn($v) => json_encode($v, JSON_UNESCAPED_UNICODE);

// Empty needle: grapheme offsets, not UTF-16 offsets.
echo $j([grapheme_strpos("😀x", "", 1), grapheme_strrpos("😀x", ""), grapheme_strpos("😀x", "", -1),
    grapheme_strrpos("e\u{301}😀x", "", -1), grapheme_stripos("😀😀x", "", 2)]), "\n";

// strstr/stristr after supplementary characters.
echo $j([grapheme_strstr("😀abc", "a"), grapheme_strstr("😀abc", "a", true),
    grapheme_stristr("😀😀ABc", "b"), grapheme_stristr("😀😀ABc", "b", true)]), "\n";

// Canonically equivalent non-ASCII needles in ASCII haystacks / strstr.
echo $j([grapheme_strpos('K', "\u{212A}"), grapheme_strpos('aKbK', "\u{212A}", 1),
    grapheme_strpos('aKbK', "\u{212A}", -2), grapheme_strpos('abc', "\u{e9}"),
    grapheme_strstr("e\u{301}x", "\u{e9}"), grapheme_strstr("e\u{301}x", "\u{e9}", true),
    grapheme_strstr("abc", "b"), grapheme_strstr("abc", "z")]), "\n";

// Empty haystack: no error.
intl_get_error_code();
foreach (['grapheme_strpos', 'grapheme_stripos', 'grapheme_strrpos', 'grapheme_strripos'] as $f) {
    echo $f, ' ', $j([$f('', "\u{e9}"), $f('', "e\u{301}"), $f('', "\r\n"), $f('', '')]), ' ', intl_get_error_code(), "\n";
}
echo $j([grapheme_strstr('', "\u{e9}"), grapheme_stristr('', "\r\n", true), grapheme_strstr('', '')]), ' ', intl_get_error_code(), "\n";

// Negative-offset strrpos finds overlapping matches.
echo $j([grapheme_strrpos("ééé", "éé", -1), grapheme_strrpos("ééxéé", "éé", -1),
    grapheme_strripos("ÉéÉ", "éé", -1), grapheme_strrpos("éééé", "éé", -2)]), "\n";

// Collation-ignorable code points (U+00AD, controls, ZWSP, Prepend U+0600).
$S = "\u{AD}";
foreach ([["abc", "b$S"], ["ébc", "b$S"], ["é{$S}bc", "bc"], ["éb{$S}c", "bc"], ["é\x01bc", "bc"],
    ["éb\u{200B}c", "bc"], ["ébc", $S], ["é\u{301}", $S], ["\x01\u{1100}\u{94D}", "\u{600}"],
    ["a\u{600}éb", "éb"], ["\u{FE0F}é", "é"], ["\u{FE0F}\u{1F3FB}", "\u{1F3FB}"]] as [$h, $n]) {
    echo bin2hex($h), '/', bin2hex($n), ' ', $j([grapheme_strpos($h, $n), grapheme_strrpos($h, $n),
        grapheme_stripos($h, $n), grapheme_strripos($h, $n), grapheme_strstr($h, $n)]), "\n";
}

// ICU 79 grapheme boundaries: GB9c is now Linker Extend* × Consonant, new
// Unicode 18 Extend characters, and former reserved Extended_Pictographic
// code points that are now plain symbols.
foreach (["\u{94D}\u{915}", "a\u{94D}\u{915}\u{94D}\u{937}", "\u{915}\u{94D}\u{94D}\u{915}", "a\u{5C8}",
    "b\u{1ADE}", "a\u{10ECB}", "\u{1F600}\u{200D}\u{1F7DA}", "\u{1F600}\u{200D}\u{1F600}",
    "\u{1CF5}\u{915}", "\u{915}\u{94D}\u{11B0A}", "x\u{1D127}"] as $s) {
    echo bin2hex($s), ' ', grapheme_strlen($s), ' ', $j(grapheme_str_split($s)), "\n";
}

// NFKC_Casefold with Unicode 18 case pairs; raw decompositions.
foreach (["\u{A7DD}", "\u{AB6C}", "\u{1DF40}", "\u{1DF95}", "\u{1C4}"] as $s) {
    echo bin2hex($s), ' ', bin2hex(normalizer_normalize($s, Normalizer::NFKC_CF)), ' ',
        var_export(normalizer_is_normalized($s, Normalizer::NFKC_CF), true), "\n";
}
foreach (["\u{FFDA}", "\u{1C4}", "\u{F77}", "\u{3300}", "\u{FDFA}", "\u{E9}", "a"] as $s) {
    echo bin2hex($s), ' ', var_export(normalizer_get_raw_decomposition($s, Normalizer::FORM_KC), true), ' ',
        var_export(normalizer_get_raw_decomposition($s), true), "\n";
}
foreach (['', 'aa', "\xF5", "\xC3", "\xE2\x82"] as $s) {
    var_dump(normalizer_get_raw_decomposition($s));
    echo intl_get_error_message(), "\n";
}
