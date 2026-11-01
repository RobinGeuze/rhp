<?php
// grapheme_* — ASCII fast paths, Unicode (combining / ZWJ / CRLF / flags)
// boundaries, offsets, invalid UTF-8, and php 8.5.8's UTF-16 offset quirks.
function d($label, $v) {
    array_walk_recursive($v, function (&$x) { if (is_string($x) && !preg_match("//u", $x)) $x = "hex:" . bin2hex($x); });
    echo $label, ": ", json_encode($v, JSON_UNESCAPED_UNICODE), "\n";
}
function e($f) { try { $f(); } catch (\Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; } }
$s = "e\u{301}a\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F467}b\r\nc\u{1F1E9}\u{1F1EA}";
$nfc = "caf\u{e9} cafe\u{301} CAFÉ";
d("strlen", [grapheme_strlen($s), grapheme_strlen("abc"), grapheme_strlen(""), grapheme_strlen("a\r\nb"), grapheme_strlen("\xff"), grapheme_strlen($nfc)]);
d("strpos", [grapheme_strpos($s, "a"), grapheme_strpos($s, "b"), grapheme_strpos($s, "c"), grapheme_strpos($s, "e"), grapheme_strpos($s, "x"), grapheme_strpos("abcabc", "c", 3), grapheme_strpos("abcabc", "c", -2), grapheme_strpos("abc", ""), grapheme_strpos("abc", "", 2), grapheme_strpos($s, ""), grapheme_strpos($s, "", 3), grapheme_strpos($s, "", -2), grapheme_strpos("", "a"), grapheme_strpos("a\xff", "b"), grapheme_strpos("abc", "\xff"), grapheme_strpos("abc", "é")]);
d("strpos-nfc", [grapheme_strpos($nfc, "e\u{301}"), grapheme_strpos($nfc, "\u{e9}", 2), grapheme_strpos($nfc, "cafe"), grapheme_strpos($nfc, "é", -3)]);
d("stripos", [grapheme_stripos($nfc, "CAFÉ"), grapheme_stripos($nfc, "café", 1), grapheme_stripos("ABC", "b"), grapheme_stripos("ABCabc", "C", -2), grapheme_stripos("Straße", "SS"), grapheme_stripos("ÀB", "àb"), grapheme_stripos("ab", "x"), grapheme_stripos("ab", "É")]);
d("strrpos", [grapheme_strrpos("abcabc", "c"), grapheme_strrpos("abcabc", "c", -2), grapheme_strrpos("abcabc", "ab", 1), grapheme_strrpos("abcabc", ""), grapheme_strrpos("abcabc", "", -1), grapheme_strrpos($nfc, "é"), grapheme_strrpos($nfc, "é", -3), grapheme_strrpos($nfc, "é", 2), grapheme_strrpos("aé", ""), grapheme_strrpos("aéaé", "", -1), grapheme_strrpos("abc", "abcd"), grapheme_strrpos("ééé", "éé", -1)]);
d("strripos", [grapheme_strripos("ABCabc", "C"), grapheme_strripos($nfc, "É"), grapheme_strripos($nfc, "CAF", -5), grapheme_strripos("aXa", "x", 1)]);
d("substr", [grapheme_substr($s, 1), grapheme_substr($s, 1, 2), grapheme_substr($s, -2), grapheme_substr($s, -2, 1), grapheme_substr($s, 0, -1), grapheme_substr($s, 10), grapheme_substr($s, 7), grapheme_substr($s, -10), grapheme_substr($s, 2, -10), grapheme_substr($s, 1, 0), grapheme_substr("abc", 1), grapheme_substr("abc", -5, 2), grapheme_substr("abc", 5), grapheme_substr("abc", 1, -5), grapheme_substr("\xff", 0), grapheme_substr("aé", 1, null), grapheme_substr($s, -3, -1), grapheme_substr($s, 3, 100)]);
d("strstr", [grapheme_strstr($s, "b"), grapheme_strstr($s, "b", true), grapheme_strstr("abc", "b"), grapheme_strstr("abc", "b", true), grapheme_strstr("abc", "x"), grapheme_strstr("abc", ""), grapheme_strstr($nfc, "e\u{301}"), grapheme_stristr($nfc, "É"), grapheme_stristr($nfc, "É", true), grapheme_stristr("ABC", "b"), grapheme_stristr("ABC", ""), grapheme_stristr("abc", "x")]);
d("split", [grapheme_str_split($s), grapheme_str_split($s, 2), grapheme_str_split($s, 3), grapheme_str_split("abcde", 2), grapheme_str_split(""), grapheme_str_split("a"), grapheme_str_split("ab\xffcd", 2), grapheme_str_split("abc", 3), grapheme_str_split("abc", 5)]);
$n = null;
d("extract", [grapheme_extract($s, 2), grapheme_extract($s, 2, GRAPHEME_EXTR_COUNT, 0, $n), $n, grapheme_extract($s, 5, GRAPHEME_EXTR_MAXBYTES, 0, $n), $n, grapheme_extract($s, 3, GRAPHEME_EXTR_MAXCHARS, 0, $n), $n, grapheme_extract($s, 2, GRAPHEME_EXTR_COUNT, 1, $n), $n, grapheme_extract("abcdef", 3, 0, 2, $n), $n, grapheme_extract("abc", 0), grapheme_extract("abc", 1, 0, 5), grapheme_extract("abc", 2, 0, -1, $n), $n, grapheme_extract("é", 1, 0, 1, $n), $n, grapheme_extract("éa", 1, 0, 1, $n), $n, grapheme_extract("a\r\nb", 2, 0, 0, $n), $n]);
d("lev", [grapheme_levenshtein("kitten", "sitting"), grapheme_levenshtein("café", "cafe\u{301}"), grapheme_levenshtein("", "abc"), grapheme_levenshtein("ab", ""), grapheme_levenshtein("a\u{1F469}\u{200D}\u{1F469}", "ab", 2, 3, 4), grapheme_levenshtein("abc", "x", 1, 1, 2)]);
d("err", [intl_get_error_code(), intl_get_error_message()]);
e(fn() => grapheme_strpos("abc", "a", 4));
e(fn() => grapheme_strpos("abc", "a", -4));
e(fn() => grapheme_strpos("aé", "", 3));
e(fn() => grapheme_strrpos("aé", "", -3));
e(fn() => grapheme_substr("abc", PHP_INT_MAX));
e(fn() => grapheme_substr("abc", 0, PHP_INT_MIN));
e(fn() => grapheme_str_split("abc", 0));
e(fn() => grapheme_extract("abc", -1));
e(fn() => grapheme_extract("abc", 1, 7));
e(fn() => grapheme_levenshtein("a", "b", 0));
e(fn() => grapheme_strlen([]));
e(fn() => grapheme_strpos("a", "a", "x"));
e(fn() => grapheme_strpos("a", "a", 0, "x\0y"));
d("err2", [grapheme_strlen("\xff"), intl_get_error_code(), intl_get_error_message(), intl_is_failure(intl_get_error_code()), intl_error_name(1)]);
d("consts", [GRAPHEME_EXTR_COUNT, GRAPHEME_EXTR_MAXBYTES, GRAPHEME_EXTR_MAXCHARS]);
