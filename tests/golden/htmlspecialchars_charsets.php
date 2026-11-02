<?php
// htmlspecialchars: the $encoding argument (multibyte splitting for the
// Asian charsets, single bytes otherwise, a warning for unknown names),
// ENT_DISALLOWED substitution and numeric-entity checks, and
// process_numeric_entity's strtol quirks with double_encode=false.
set_error_handler(function ($no, $msg) { echo "E$no: $msg\n"; return true; });
$cases = ["&#0;", "&#x1;", "&#13;", "&#9;", "&#65;", "&#xFFFE;", "&#xFDD0;", "&#x0x41;", "&#X0X41;", "&#0x41;", "&#x0x;",
    "&#00065;", "&#1114112;", "x\x01y", "\x7f\xc2\x85", "\xef\xbf\xbe", "\xef\xb7\x90", "&apos;"];
$flags = [ENT_QUOTES, ENT_XML1 | ENT_SUBSTITUTE | ENT_DISALLOWED, ENT_HTML5 | ENT_DISALLOWED, ENT_DISALLOWED,
    ENT_XHTML | ENT_DISALLOWED, ENT_HTML5 | ENT_QUOTES];
foreach ($cases as $c) {
    echo bin2hex($c);
    foreach ($flags as $f) {
        echo ' ', bin2hex(htmlspecialchars($c, $f)), '/', bin2hex(htmlspecialchars($c, $f, null, false));
    }
    echo "\n";
}
foreach (["ISO-8859-1", "cp1252", "SJIS", "BIG5", "EUC-JP", "GB2312", "BIG5-HKSCS", "utf-8", "koi8-r", "", "bogus"] as $cs) {
    echo str_pad($cs, 11);
    foreach (["\xe9<", "\x81\x40&", "\x8f\xa1\xa1'", "\xa1\x40\"", "\x81", "\x01x", "\xa1\xff"] as $c) {
        echo ' ', bin2hex(htmlspecialchars($c, ENT_QUOTES, $cs)), '/', bin2hex(htmlspecialchars($c, ENT_QUOTES | ENT_SUBSTITUTE | ENT_DISALLOWED, $cs));
    }
    echo "\n";
}
