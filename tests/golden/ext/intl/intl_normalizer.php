<?php
// normalizer_* / Normalizer — all five forms, invalid UTF-8, raw decompositions.
function d($label, $v) {
    array_walk_recursive($v, function (&$x) { if (is_string($x) && !preg_match("//u", $x)) $x = "hex:" . bin2hex($x); });
    echo $label, ": ", json_encode($v, JSON_UNESCAPED_UNICODE), "\n";
}
function e($f) { try { $f(); } catch (\Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; } }
d("norm", [normalizer_normalize("cafe\u{301}"), normalizer_normalize("caf\u{e9}", Normalizer::NFD), normalizer_normalize("ﬁ①", Normalizer::NFKC), normalizer_normalize("ﬁ①", Normalizer::NFKD), normalizer_normalize("Straße ﬁ ΣΑΣ \u{AD}x", Normalizer::NFKC_CF), normalizer_normalize(""), normalizer_normalize("\xff"), Normalizer::normalize("A\u{30A}"), normalizer_normalize("\u{1E9B}\u{323}", Normalizer::FORM_KC), normalizer_normalize("\u{1E9B}\u{323}", Normalizer::FORM_D), normalizer_normalize("\u{AC00}\u{11A8}", Normalizer::NFD), normalizer_normalize("\u{1100}\u{1161}\u{11A8}")]);
d("err", [intl_get_error_code(), intl_get_error_message()]);
d("isnorm", [normalizer_is_normalized("cafe\u{301}"), normalizer_is_normalized("caf\u{e9}"), normalizer_is_normalized("cafe\u{301}", Normalizer::NFD), normalizer_is_normalized("ﬁ", Normalizer::NFKC), normalizer_is_normalized("abc", Normalizer::NFKC_CF), normalizer_is_normalized("ABC", Normalizer::NFKC_CF), normalizer_is_normalized("\xff"), normalizer_is_normalized(""), Normalizer::isNormalized("x"), Normalizer::isNormalized("x\u{301}", Normalizer::FORM_KD)]);
d("raw", [normalizer_get_raw_decomposition("é"), normalizer_get_raw_decomposition("a"), normalizer_get_raw_decomposition("ab"), normalizer_get_raw_decomposition("\u{1E69}"), normalizer_get_raw_decomposition("ﬁ", Normalizer::FORM_KC), normalizer_get_raw_decomposition("\u{AC01}"), normalizer_get_raw_decomposition("\u{212B}"), Normalizer::getRawDecomposition("\u{e9}")]);
d("err2", [intl_get_error_code(), intl_get_error_message()]);
normalizer_normalize("ok");
d("reset", [intl_get_error_code(), intl_get_error_message()]);
e(fn() => normalizer_normalize("a", 3));
e(fn() => normalizer_is_normalized("a", 99));
d("consts", [Normalizer::FORM_D, Normalizer::NFD, Normalizer::FORM_KD, Normalizer::NFKD, Normalizer::FORM_C, Normalizer::NFC, Normalizer::FORM_KC, Normalizer::NFKC, Normalizer::FORM_KC_CF, Normalizer::NFKC_CF]);
