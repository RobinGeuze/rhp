<?php
function t($s, $f = IDNA_DEFAULT) { $r = idn_to_ascii($s, $f, INTL_IDNA_VARIANT_UTS46, $i); echo json_encode($s), " => ", var_export($r, true), " ", json_encode($i, JSON_UNESCAPED_UNICODE|JSON_INVALID_UTF8_SUBSTITUTE), "\n"; }
function u($s, $f = IDNA_DEFAULT) { $r = idn_to_utf8($s, $f, INTL_IDNA_VARIANT_UTS46, $i); echo "U ", json_encode($s), " => ", var_export($r, true), " ", json_encode($i, JSON_UNESCAPED_UNICODE|JSON_INVALID_UTF8_SUBSTITUTE), "\n"; }
$l63 = str_repeat("a", 63);
t("$l63.$l63.$l63." . str_repeat("a", 61)); // 253
t("$l63.$l63.$l63." . str_repeat("a", 62)); // 254
t("$l63.$l63.$l63." . str_repeat("a", 61) . "."); // 254 trailing dot
t("$l63.$l63.$l63." . str_repeat("a", 60) . "ä"); // nonascii
t("ä$l63.$l63.$l63." . str_repeat("a", 50));
t(str_repeat("a", 64));
t(str_repeat("a", 60) . "ää");
t("xn--"); t("xn--a"); t("xn--abc-"); t("xn--ls8h"); t("xn--LS8H"); t("xn--zz-xyz"); t("xn--a.b"); t("xn--ab--c");
t("ab--c"); t("-a"); t("a-"); t("a..b"); t(".a"); t("a."); t("."); t(".."); t("a b"); t("a_b"); t("a_b", 2); t("faß.de"); t("faß.de", 0); t("ς.gr", 0); t("a\u{200c}b"); t("a\u{200c}b", 0); t("a\u{200c}b", IDNA_DEFAULT|8);
t("\u{0301}a"); t("ex\xffample"); t("EXAMPLE.Com"); t("Bücher.DE"); t("ab\u{FF0E}cd"); t("a\u{3002}b"); t("\u{05D0}\u{05D1}", IDNA_DEFAULT|4); t("a.\u{05D0}1", IDNA_DEFAULT|4);t("1\u{05D0}", IDNA_DEFAULT|4); t("\u{05D0}a", IDNA_DEFAULT|4);
t("\u{2260}", 2); t("a\u{00A0}b", 2); t("☃.net"); t("xn--n3h.net"); t("xn--N3H.net"); t("xn--ss-"); t("a\u{094D}\u{200D}b", 8); t("\u{0628}\u{200C}\u{0628}", 8);
t("ä.xn--"); t("Ä-.b"); t("xn--a-ecp.ru"); t("xn--\u{e4}"); t("a\u{00AD}b"); t("A\u{0308}"); t("\u{FFFD}"); t("a\u{E0001}b"); t("x\u{0378}y");
u("xn--bcher-kva.de"); u("xn--zz"); u("faß.DE"); u("faß.DE", 0); u("xn--a"); u("abc"); u("a..b"); u("xn--ls8h.xn--"); u("XN--BCHER-KVA.de"); u("a.b."); u("-x"); u("xn--ab--c");
try { idn_to_ascii(""); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { idn_to_ascii("a", 0, 0); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(idn_to_ascii("a", 0, 1, $x), $x);
