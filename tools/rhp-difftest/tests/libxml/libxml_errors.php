<?php
// libxml2's parser diagnostics: codes, levels, lines, columns and messages,
// collected by libxml_use_internal_errors() or raised as warnings.
set_error_handler(function ($no, $str) { echo "W[$no]: $str\n"; return true; });

$cases = ['<a>', '<a></b>', 'abc', ' ', "<a>\n<b>\n</a>", '<a b="1" b="2"/>', '<a b=1/>', '<a b/>',
    '<a>&foo;</a>', '<a>&;</a>', '<a>&amp</a>', '<a><!-- x </a>', '<a/><b/>', '<a/>x', '<1a/>', '<a', '<a b="1"',
    '<x:a/>', "<a>\xE9</a>", '<a/><?xml version="1.0"?>', "<a>\x01</a>", '<a><![CDATA[x</a>',
    '<?xml version="1.0" encoding="bogus"?><a/>', '<a>]]></a>', '<a b="<"/>', '<a xmlns:x=""/>',
    "<a>\n  <b c='1'>\n</a>", "<a xmlns='u'/>"];

var_dump(libxml_use_internal_errors(true), libxml_use_internal_errors());
foreach ($cases as $c) {
    $d = new DOMDocument;
    $r = $d->loadXML($c);
    echo json_encode($c), ' => ', var_export($r, true), "\n";
    foreach (libxml_get_errors() as $e) {
        echo "  [$e->level/$e->code L$e->line C$e->column F'$e->file'] ", json_encode($e->message), "\n";
    }
    libxml_clear_errors();
}

$d = new DOMDocument;
$d->loadXML('<a>');
$e = libxml_get_last_error();
var_dump($e instanceof LibXMLError, $e->level === LIBXML_ERR_FATAL, $e->code, $e->line);
var_dump(libxml_use_internal_errors(false), count(libxml_get_errors()), libxml_get_last_error()->code);
libxml_clear_errors();
var_dump(libxml_get_last_error());

// warnings when internal errors are off
$d->loadXML("<a>\n<b></a>");
$d->loadXML("<x:a xmlns:y='u'/>");
simplexml_load_string('<x:a/>');
simplexml_load_string("<a>\n  <b>\n</a>");
simplexml_load_string("<a>\t<b c='1' c='2'/></a>");
simplexml_load_string(str_repeat('x', 100) . '<');
simplexml_load_string('<a>' . str_repeat('y', 100) . '</b>');

// LIBXML_NOERROR / LIBXML_NOWARNING
$d->loadXML('<a xmlns="rel"/>', LIBXML_NOWARNING);
$d->loadXML('<x:a/>', LIBXML_NOERROR);
var_dump(LIBXML_NOENT, LIBXML_NONET, LIBXML_NOCDATA, LIBXML_SCHEMA_CREATE);

// file loading reports the file name
$f = tempnam(sys_get_temp_dir(), 'dx');
file_put_contents($f, "<r>\n<q></r>");
set_error_handler(function ($no, $str) use ($f) { echo "W[$no]: ", str_replace([realpath($f), $f], 'FILE', $str), "\n"; return true; });
var_dump($d->load($f));
var_dump(simplexml_load_file($f));
libxml_use_internal_errors(true);
$d->load($f);
foreach (libxml_get_errors() as $e) echo $e->line, ' ', str_replace([realpath($f), $f], 'FILE', $e->file), ' ', trim($e->message), "\n";
libxml_use_internal_errors(false);
unlink($f);
var_dump($d->load($f . '.missing'));
