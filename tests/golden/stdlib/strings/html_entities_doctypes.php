<?php
// html.c over php-src's entity tables: every doctype, quote and error
// flag, the two-code-point HTML5 entities, and the single-byte charsets.
$inputs = [
    "<a href='x'>T&amp;J \"q\"</a>",
    "caf\u{e9} \u{20ac} \u{2242}\u{338} fj \u{226b}\u{20d2} \u{1d504}",
    "bad \xff\xfe utf8 \xc3",
    "\x01\x0b\x0c\x0d\x7f\u{80}\u{9f}\u{fffe}\u{fdd0}",
    "&amp; &lt;b&gt; &#039; &#39; &apos; &quot; &#x27; &#X41; &#65; &#x0x41; &#0041;",
    "&eacute;&Eacute;&nesim;&NotEqualTilde;&fjlig;&nvgt;&gt;&GT;&QUOT;&bogus;&#1114112;&#xD800;&#13;&#xFFFE;",
    "&ampx; &a b; & &; &#; &#x; &#a; &#12 &x &CounterClockwiseContourIntegral;",
];
foreach ($inputs as $i => $s) {
    foreach ([0, 16, 32, 48] as $doctype) {
        foreach ([ENT_NOQUOTES, ENT_COMPAT, ENT_QUOTES] as $q) {
            foreach ([0, ENT_IGNORE, ENT_SUBSTITUTE, ENT_DISALLOWED | ENT_SUBSTITUTE] as $e) {
                $f = $doctype | $q | $e;
                echo "$i $f: ", bin2hex(implode('|', [
                    htmlspecialchars($s, $f),
                    htmlspecialchars($s, $f, null, false),
                    htmlentities($s, $f),
                    htmlentities($s, $f, 'UTF-8', false),
                    html_entity_decode($s, $f),
                    htmlspecialchars_decode($s, $f),
                ])), "\n";
            }
        }
    }
}
foreach (['UTF-8', 'ISO-8859-1', 'cp1252', 'ISO-8859-15', 'cp1251', 'KOI8-R', 'cp866', 'MacRoman', 'ISO-8859-5'] as $cs) {
    echo $cs, ": ", bin2hex(htmlentities("\x80\xa4\xbd\xc0\xe9\xff", ENT_QUOTES, $cs)), " ",
        bin2hex(html_entity_decode("&euro;&eacute;&#xA4;&OElig;&Iuml;&#1025;", ENT_QUOTES, $cs)), "\n";
    foreach ([0, 16, 32, 48] as $doctype) {
        $t = get_html_translation_table(HTML_ENTITIES, ENT_QUOTES | $doctype, $cs);
        echo "  $doctype ", count($t), " ", md5(serialize($t)), "\n";
    }
}
var_dump(get_html_translation_table());
var_dump(@htmlentities("\xa1\xa2<", ENT_QUOTES, 'BIG5') === "\xa1\xa2&lt;");
