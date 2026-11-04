<?php
// Nodes outliving their document, detached subtrees, and replaced
// children, released in every order.
for ($i = 0; $i < 200; $i++) {
    $doc = new DOMDocument();
    $doc->loadXML('<r><a x="1"><b>t</b><c/></a><d><!--c--><![CDATA[x]]></d></r>');
    $a = $doc->documentElement->firstChild;
    $b = $a->firstChild;
    $attr = $a->getAttributeNode('x');
    $text = $b->firstChild;
    $doc->documentElement->removeChild($a);
    unset($doc);
    $b->appendChild(new DOMText('more'));
    $a->removeAttributeNode($attr);
    unset($a);
    $attr->value = 'y';
    echo $b->textContent === 'tmore' && $text->data === 't' ? '' : "bad $i\n";
    unset($b, $attr, $text);

    $doc = new DOMDocument();
    $doc->loadXML('<r><a/><b/></r>');
    $old = $doc->documentElement->replaceChild($doc->createElement('n'), $doc->documentElement->firstChild);
    $frag = $doc->createDocumentFragment();
    $frag->appendXML('<x/><y>z</y>');
    $doc->documentElement->appendChild($frag);
    $list = $doc->getElementsByTagName('*');
    $doc->documentElement->removeChild($list->item(1));
    foreach ($list as $n) {
        $n->setAttribute('k', 'v');
    }
    $clone = $doc->documentElement->cloneNode(true);
    $other = new DOMDocument();
    $imported = $other->importNode($clone, true);
    $other->appendChild($imported);
    $adopted = $other->adoptNode($old);
    unset($doc, $clone, $list);
    $other->documentElement->appendChild($adopted);
    $other->saveXML();
    unset($other);
    echo $imported->nodeName === 'r' ? '' : "bad import $i\n";
}
echo "ok\n";
