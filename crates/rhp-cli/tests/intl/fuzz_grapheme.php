<?php
// grapheme_* and normalizer_* over random strings from interesting pieces
// (combining marks, ZWJ, regional indicators, Indic conjuncts, ignorables,
// case pairs, ill-formed UTF-8).
mt_srand(12345);
$pool = ["a", "b", "A", "é", "e\u{301}", "\u{301}", "\r", "\n", "ß", "SS", "\u{200D}", "\u{1F469}", "\u{1F1E9}",
    "\u{1F1EA}", "\u{AD}", "\x01", "\u{915}", "\u{94D}", "\u{600}", "\u{FE0F}", "\u{1F3FB}", "K", "\u{212A}", "\xff",
    "\xc3", " ", "İ", "ı", "\u{1100}", "\u{1161}", "\u{11A8}", "\u{AC00}", "ﬁ", "Σ", "ς", "\u{627}", "\u{5D0}"];
$hex = function ($v) {
    if (is_array($v)) array_walk_recursive($v, function (&$x) { if (is_string($x)) $x = bin2hex($x); });
    return json_encode(is_string($v) ? bin2hex($v) : $v);
};
$rs = function ($n) use ($pool) {
    $s = '';
    for ($i = 0; $i < $n; $i++) $s .= $pool[mt_rand(0, count($pool) - 1)];
    return $s;
};
for ($t = 0; $t < 400; $t++) {
    $h = $rs(mt_rand(0, 8));
    $n = $rs(mt_rand(0, 3));
    $o = mt_rand(-4, 4);
    $r = [];
    foreach (['grapheme_strpos', 'grapheme_stripos', 'grapheme_strrpos', 'grapheme_strripos'] as $f) {
        try { $r[] = $f($h, $n, $o); } catch (Throwable $e) { $r[] = get_class($e) . ': ' . $e->getMessage(); }
        $r[] = intl_get_error_code();
    }
    foreach (['grapheme_strstr', 'grapheme_stristr'] as $f) {
        $r[] = $f($h, $n);
        $r[] = $f($h, $n, true);
    }
    $r[] = grapheme_strlen($h);
    try { $r[] = grapheme_substr($h, $o, mt_rand(-3, 3)); } catch (Throwable $e) { $r[] = $e->getMessage(); }
    $r[] = grapheme_str_split($h, mt_rand(1, 3));
    $next = null;
    $r[] = grapheme_extract($h, mt_rand(0, 4), mt_rand(0, 2), mt_rand(0, 3), $next);
    $r[] = $next;
    $r[] = grapheme_levenshtein($h, $n);
    foreach ([4, 8, 16, 32, 48] as $form) {
        $r[] = normalizer_normalize($h, $form);
        $r[] = normalizer_is_normalized($h, $form);
    }
    $r[] = intl_get_error_message();
    echo $t, ' ', bin2hex($h), ' ', bin2hex($n), ' ', $o, ' ', $hex($r), "\n";
}
