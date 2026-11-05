<?php
// XMLWriter output encoding: without a declared encoding attribute values
// escape non-ASCII; a declared non-UTF-8 encoding converts the output.
foreach ([null, 'UTF-8', 'ISO-8859-1', false] as $enc) {
    $w = new XMLWriter; $w->openMemory();
    if ($enc !== false) { $enc === null ? $w->startDocument() : $w->startDocument('1.0', $enc); }
    $w->startElement('ü');
    $w->writeAttribute('a', "ü✓😀 \x7f é");
    $w->writeAttributeNs('p', 'b', 'urn:x', 'ü');
    $w->text("ü✓😀 é");
    $w->writeElement('t', 'ü');
    $w->writeCdata('ü');
    $w->writeComment('ü');
    $w->writePi('pi', 'ü');
    $w->endElement();
    $w->endDocument();
    echo $w->outputMemory(), "\n---\n";
}
