<?php
// intl global error state: codes, names, messages, which functions reset it.
foreach ([0, 1, 10, 65536, -128, 999, 15, 66560, 65792, 66816, 66818] as $c) echo $c, " ", intl_error_name($c), " ", var_export(intl_is_failure($c), true), "\n";
var_dump(U_ZERO_ERROR, U_ILLEGAL_ARGUMENT_ERROR, U_INVALID_CHAR_FOUND, U_BUFFER_OVERFLOW_ERROR, U_ERROR_LIMIT, U_USING_FALLBACK_WARNING);
var_dump(intl_get_error_code(), intl_get_error_message());
var_dump(normalizer_get_raw_decomposition("\u{1E0A}"), intl_get_error_code(), normalizer_get_raw_decomposition("ab"), intl_get_error_code(), intl_get_error_message());
// grapheme functions set but never reset the error
foreach (['grapheme_strpos', 'grapheme_stripos', 'grapheme_strrpos', 'grapheme_strripos', 'grapheme_strstr', 'grapheme_stristr'] as $f) {
    $r = [];
    foreach (['a', "\u{00E9}", "e\u{0301}", "\r\n", ''] as $needle) $r[] = $f('', $needle);
    echo $f, ": ", json_encode($r), " ", intl_get_error_code(), " ", intl_get_error_message(), "\n";
}
var_dump(grapheme_strlen("ok"), intl_get_error_code());
var_dump(grapheme_extract("abc", 1, 0, 9), intl_get_error_message());
var_dump(idn_to_ascii("ok"), intl_get_error_code(), intl_get_error_message());
var_dump(grapheme_substr("\xc3", 0), intl_get_error_message());
var_dump(normalizer_is_normalized("\xc3"), intl_get_error_message());
