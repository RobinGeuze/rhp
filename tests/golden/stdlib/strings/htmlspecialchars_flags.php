<?php
// htmlspecialchars flags: quote styles, doctypes (ENT_XML1 is what XML
// reporters such as psalm's checkstyle/xml output use), double_encode and
// invalid UTF-8 handling.
$s = "a&b <c> \"d\" 'e' &amp; &#39; &#x41; &apos; &bogus; &nbsp; &#99999999;";
$flags = [
    'default' => null,
    'ENT_COMPAT' => ENT_COMPAT,
    'ENT_NOQUOTES' => ENT_NOQUOTES,
    'ENT_QUOTES' => ENT_QUOTES,
    'XML1|QUOTES' => ENT_XML1 | ENT_QUOTES,
    'XHTML|QUOTES' => ENT_XHTML | ENT_QUOTES,
    'HTML5|QUOTES' => ENT_HTML5 | ENT_QUOTES,
    'single only' => 1,
];
foreach ($flags as $name => $f) {
    $a = $f === null ? htmlspecialchars($s) : htmlspecialchars($s, $f);
    echo str_pad($name, 14), $a, "\n";
    $b = htmlspecialchars($s, $f ?? (ENT_QUOTES | ENT_SUBSTITUTE), null, false);
    echo str_pad('  no-double', 14), $b, "\n";
}
var_dump(ENT_XML1, ENT_XHTML, ENT_IGNORE, ENT_SUBSTITUTE, ENT_DISALLOWED);
$bad = "x\xC3y\xE2\x82z\xF0\x9F\x98\xFF<\xED\xA0\x80>";
var_dump(bin2hex(htmlspecialchars($bad)));
var_dump(htmlspecialchars($bad, ENT_QUOTES));
var_dump(bin2hex(htmlspecialchars($bad, ENT_QUOTES | ENT_IGNORE)));
var_dump(bin2hex(htmlspecialchars($bad, ENT_XML1 | ENT_SUBSTITUTE)));
var_dump(htmlspecialchars("ünï ✓ 😀 'q'", ENT_XML1 | ENT_QUOTES));
