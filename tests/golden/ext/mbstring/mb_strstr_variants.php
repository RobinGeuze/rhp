<?php
function d($l, $v) { echo $l, ": ", var_export($v, true), "\n"; }
$h = "Hello Wörld ÄÖÜ äöü hello";
d("stristr", [mb_stristr($h, "WÖR"), mb_stristr($h, "wör", true), mb_stristr($h, "äöü"), mb_stristr($h, "ÄÖÜ", false), mb_stristr($h, "x"), mb_stristr($h, ""), mb_stristr("", ""), mb_stristr("", "a"), mb_stristr("ǅa", "ǆ"), mb_stristr("STRASSE", "ß"), mb_stristr("abc\xffdef", "D"), mb_stristr("ÀÉ", "é", false, "UTF-8"), mb_stristr("ABC", "b", false, "ASCII")]);
d("strrichr", [mb_strrichr($h, "HELLO"), mb_strrichr($h, "hello", true), mb_strrichr($h, "Ö"), mb_strrichr($h, "x"), mb_strrichr($h, ""), mb_strrichr($h, "", true)]);
d("strstr", [mb_strstr($h, "ö"), mb_strstr($h, "ö", true), mb_strstr($h, ""), mb_strstr("a\x80b", "b"), mb_strstr("a\x80b", "b", true), mb_strrchr($h, "l"), mb_strrchr($h, "l", true), mb_strrchr($h, ""), mb_strrchr("a\x80bb", "b", true), mb_strstr("abc", "c", false, "latin1")]);
try { mb_stristr("a", "b", false, "nope"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { mb_strrichr("a", "b", false, "nope"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
$a = "abc"; $b = ["x" => "é", "y" => ["ü", 5, null]]; $o = new stdClass; $o->p = "ä";
d("cv", mb_convert_variables("ISO-8859-1", "UTF-8", $a, $b, $o));
d("vals", [bin2hex($a), bin2hex($b["x"]), bin2hex($b["y"][0]), $b["y"][1], $b["y"][2], bin2hex($o->p)]);
$c = "\xe9t\xe9";
d("cv2", mb_convert_variables("UTF-8", "ISO-8859-1", $c));
d("c", $c);
$e = ["plain", "ascii"];
d("cv3", mb_convert_variables("UTF-8", "ASCII,UTF-8", $e));
$e2 = ["plain", "ünï"];
d("cv4", mb_convert_variables("UTF-8", ["ASCII", "UTF-8"], $e2));
d("e2", $e2);
$e3 = "abc";
d("cv5", mb_convert_variables("utf8", "utf8", $e3));
$lines = ["x", "y"];
d("cv6", mb_convert_variables("UTF-8", "utf8", $lines));
try { $z = "a"; mb_convert_variables("bogus", "UTF-8", $z); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { $z = "a"; mb_convert_variables("UTF-8", "bogus", $z); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { $z = "a"; mb_convert_variables("UTF-8", [], $z); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
