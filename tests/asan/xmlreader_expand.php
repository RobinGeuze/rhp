<?php
// XMLReader::expand() copies, with and without a base document, outliving
// the reader.
$xml = '<books><book id="1"><t>A</t></book><book id="2"><t>B</t></book></books>';
for ($i = 0; $i < 200; $i++) {
    $r = XMLReader::fromString($xml);
    $doc = new DOMDocument();
    $root = $doc->appendChild($doc->createElement('root'));
    $nodes = [];
    while ($r->read()) {
        if ($r->nodeType === XMLReader::ELEMENT && $r->name === 'book') {
            $nodes[] = $r->expand($i % 2 ? $doc : null);
            $sx = simplexml_import_dom($root->appendChild($doc->importNode(end($nodes), true)));
            $sx->seen = 'y';
            unset($sx);
        }
    }
    $r->close();
    unset($r);
    foreach ($nodes as $n) {
        echo $n->getAttribute('id') !== '' ? '' : "bad $i\n";
    }
    echo count($root->childNodes) === 2 ? '' : "bad root $i\n";
    unset($doc, $root, $nodes);
}
echo "ok\n";
