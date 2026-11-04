<?php
// SimpleXML and DOM objects wrapping the same libxml2 nodes, changed and
// released through either API.
for ($i = 0; $i < 200; $i++) {
    $sx = simplexml_load_string('<r a="1"><item>a</item><item>b</item><o><p>q</p></o></r>');
    $dom = dom_import_simplexml($sx->o);
    $p = $sx->o->p;
    unset($sx->o);
    $dom->appendChild($dom->ownerDocument->createElement('z', 'zz'));
    echo (string) $p === 'q' ? '' : "bad p $i\n";
    $back = simplexml_import_dom($dom);
    $back->added = 'v';
    unset($dom);
    echo $back->z . $back->added === 'zzv' ? '' : "bad back $i\n";
    foreach ($sx->item as $k => $item) {
        unset($item[0]);
    }
    $sx->n->m->o = 'deep';
    $sx['b'] = 2;
    unset($sx['a']);
    $c = clone $sx;
    unset($sx);
    $x = $c->xpath('//o');
    echo count($x) === 1 ? '' : "bad xpath $i\n";
    unset($c, $back, $p, $x);

    $doc = new DOMDocument();
    $doc->loadXML('<r><a><b/></a></r>');
    $s = simplexml_import_dom($doc);
    $b = $s->a->b;
    $doc->documentElement->removeChild($doc->documentElement->firstChild);
    unset($doc, $s);
    $b->c = 'd';
    echo $b->asXML() === '<b><c>d</c></b>' ? '' : "bad b $i\n";
}
echo "ok\n";
