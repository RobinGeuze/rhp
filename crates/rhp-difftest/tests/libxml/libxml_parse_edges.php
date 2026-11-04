<?php
// libxml2 2.9.13 parser edge cases: comments (fast and general paths), the
// XML declaration, malformed QNames, non-Char bytes in attribute values /
// CDATA / PIs / text, declared single-byte encodings, and NOEMPTYTAG output.
libxml_use_internal_errors(true);
function show($in, $flags = 0) {
    $d = new DOMDocument();
    $r = $d->loadXML($in, $flags);
    echo json_encode($in), ' => ', var_export($r, true), "\n";
    foreach (libxml_get_errors() as $e) {
        echo "  L{$e->level} C{$e->code} {$e->line}:{$e->column} ", json_encode($e->message), "\n";
    }
    libxml_clear_errors();
    if ($r) {
        echo '  ', bin2hex($d->documentElement->textContent), ' ', var_export($d->encoding, true), ' ', $d->xmlVersion, ' ', var_export($d->xmlStandalone, true), "\n";
        echo '  ', json_encode($d->saveXML($d->documentElement)), "\n";
    }
}
foreach ([
    '<a><!----x--></a>', '<a><!--x----></a>', '<a><!--a-b--c--d--></a>', "<a><!--\nx\ny--z--></a>",
    '<a><!--é--x--></a>', '<a><!---x--></a>', '<a><!--x---></a>', "<a><!--x\x01--y--></a>",
    "<!-- a -- b --><a/>", '<a><!--' . str_repeat('y', 60) . '--z--></a>', '<a><!--x--y', '<a><!--é--y', '<a><!-- ok --></a>',
] as $in) show($in);
foreach ([
    '<?xml version="2.0"?><a/>', '<?xml version="1.1"?><a/>', '<?xml version="1.x"?><a/>', '<?xml version="2"?><a/>',
    '<?xml version="1.0"encoding="UTF-8"?><a/>', '<?xml version="1.0" encoding="UTF-8"standalone="yes"?><a/>',
    '<?xml version="1.0" encoding="ISO-8859-1"standalone="yes"?><a/>', '<?xml encoding="UTF-8"?><a/>',
    '<?xml version="1.0?><a/>', '<?xml version=1.0?><a/>', '<?xml version="1.0" standalone="maybe"?><a/>',
    '<?xml version="1.0" standalone=yes?><a/>', '<?xml version="1.0" standalone="yes?><a/>',
    '<?xml version="1.0" ?><a/>', '<?xml  version = "1.0" encoding = \'utf-8\' standalone = "no" ?><a/>',
    '<?xml version="1.0"> <a/>', '<?xml version="1.0" foo="bar"?><a/>',
] as $in) show($in);
foreach ([
    '<a:b:c/>', '<a xmlns:a="u"><a:b:c/></a:b:c>', '<a :b="1"/>', '<a b:="1"/>', '<a xmlns:a="u"><a:b:c></a:b:c></a>',
] as $in) show($in);
foreach ([
    "<a b=\"x\x01y\"/>", "<r><a b=\"x\x01y\">t</a></r>", "<a b=\"x\xffy\"/>", "<a><![CDATA[\x01]]></a>",
    "<a><![CDATA[x\x01]]></a>", "<a><![CDATA[xy\x01]]></a>", "<a><?pi \x01?></a>", "<a><?pi x\x01y?>z</a>",
    "<a>x\x01y</a>", "<a>\x01\x02</a>", "<a>x\x01]]></a>", "<a><b>\x01</b><c/></a>",
] as $in) show($in);
foreach ([
    '<?xml version="1.0" encoding="ISO-8859-2"?><a>' . "\xb1\xe6" . '</a>',
    '<?xml version="1.0" encoding="windows-1252"?><a>' . "\x80\x93" . '</a>',
    '<?xml version="1.0" encoding="ISO-8859-15"?><a>' . "\xa4\xbd" . '</a>',
    '<?xml version="1.0" encoding="latin1"?><a>' . "\xe9" . '</a>',
    '<?xml version="1.0" encoding="UTF-16"?><a/>',
] as $in) show($in);
// output in the declared single-byte encoding (iconv-backed: only representable characters, whose escaping is platform-specific otherwise)
$d = new DOMDocument('1.0', 'windows-1252');
$d->appendChild($d->createElement('r', "€ü"));
echo bin2hex($d->saveXML()), "\n";
$d = new DOMDocument('1.0', 'ISO-8859-2');
$d->appendChild($d->createElement('r', "ąé"));
echo bin2hex($d->saveXML()), "\n";
// XML_SAVE_NO_EMPTY with formatting: no line break inside an empty element
$d = new DOMDocument();
$d->loadXML("<a><b/><c>t</c><d><e/></d></a>");
$d->formatOutput = true;
echo $d->saveXML(null, LIBXML_NOEMPTYTAG);
echo $d->saveXML($d->documentElement, LIBXML_NOEMPTYTAG), "\n";
