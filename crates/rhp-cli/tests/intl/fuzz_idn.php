<?php
// idn_to_ascii / idn_to_utf8 over random domains, for every flag combination
// php documents: results, $idna_info and the error code.
mt_srand(7);
$pool = ["a", "b", "-", ".", "xn--", "ß", "ς", "\u{200C}", "\u{200D}", "\u{5D0}", "\u{627}", "1", "\u{301}", "Ä",
    "\u{3002}", "\u{FF0E}", "_", " ", "\xff", "\u{94D}", "\u{E0001}", "\u{FFFD}", "☃", "é", "\u{2260}", "\u{A0}", "Z"];
foreach ([0, 2, 4, 8, 16, 32, 48, 62] as $flags) {
    for ($t = 0; $t < 120; $t++) {
        $s = '';
        $n = mt_rand(1, 10);
        for ($i = 0; $i < $n; $i++) $s .= $pool[mt_rand(0, count($pool) - 1)];
        $a = idn_to_ascii($s, $flags, INTL_IDNA_VARIANT_UTS46, $ia);
        $ea = intl_get_error_code();
        $u = idn_to_utf8($s, $flags, INTL_IDNA_VARIANT_UTS46, $iu);
        $eu = intl_get_error_code();
        echo $flags, ' ', bin2hex($s), ' ', var_export($a, true), ' ', bin2hex($ia['result']), ' ', $ia['errors'], ' ',
            (int) $ia['isTransitionalDifferent'], " $ea | ", bin2hex((string) $u), ' ', bin2hex($iu['result']), ' ',
            $iu['errors'], " $eu\n";
    }
}
