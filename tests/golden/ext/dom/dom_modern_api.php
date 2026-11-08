<?php
// The Dom\ API as Symfony's VarDumper and DomCrawler use it: virtual
// properties under Reflection, reading every property, HTML5 parsing and
// serialization, and copying an HTML5 tree into a DOMDocument.

function props(object $o): void
{
    $chain = [];
    for ($c = new ReflectionClass($o); $c; $c = $c->getParentClass()) {
        $chain[] = $c;
    }
    foreach (array_reverse($chain) as $c) {
        foreach ($c->getProperties() as $p) {
            if ($p->isStatic() || $p->getDeclaringClass()->name !== $c->name) {
                continue;
            }
            $v = $o->{$p->name};
            printf("%s::%s %s = %s\n", get_class($o), $p->name, $p->isVirtual() ? 'virtual' : 'slot',
                is_object($v) ? get_class($v) : var_export($v, true));
        }
    }
}

props(new Dom\Implementation());
$html = Dom\HTMLDocument::createEmpty();
props($html->createTextNode('foo'));
props($html->createAttribute('attr'));
props($html->createElement('foo'));
props((new Dom\Implementation())->createDocumentType('html', 'publicId', 'systemId'));
props($html->createProcessingInstruction('target', 'data'));
props(new Dom\XPath($html));
props($html);

$xml = Dom\XMLDocument::createFromString('<foo xmlns:a="urn:a"><bar a:x="1"/><!-- c --></foo>');
$xml->formatOutput = true;
echo $xml->saveXml(), "\n";
var_dump($xml->documentElement->firstChild->attributes['a:x']->value);
var_dump($xml->documentElement->children === $xml->documentElement->children);

$doc = Dom\HTMLDocument::createFromString(
    '<!DOCTYPE html><title> A  b </title><p class="x y" id=i>one<br>two</p><template><i>t</i></template>',
    LIBXML_NOERROR
);
echo $doc->saveHtml(), "\n";
var_dump($doc->title, $doc->body->tagName, $doc->getElementById('i')->classList->contains('y'));
var_dump(count($doc->querySelectorAll('p, br')), $doc->querySelector('p > br')?->nodeName);
$p = $doc->querySelector('#i');
$p->innerHTML = '<b>bold</b> &amp; text';
echo $p->outerHTML, "\n";
foreach ($p->classList as $k => $class) {
    echo "$k: $class\n";
}

$no_ns = Dom\HTMLDocument::createFromString('<p>x</p>', Dom\HTML_NO_DEFAULT_NS | LIBXML_NOERROR);
var_dump($no_ns->documentElement->namespaceURI, get_class($no_ns->documentElement));

// DomCrawler's copyFromHtml5ToDom
$target = new DOMDocument('1.0', $doc->inputEncoding);
$stack = [[[$doc->documentElement], $target]];
while ($stack) {
    [$children, $parent] = array_pop($stack);
    foreach ($children as $source) {
        if ($source instanceof Dom\CharacterData) {
            $parent->appendChild($target->createTextNode($source->data));
            continue;
        }
        if (!$source instanceof Dom\Element) {
            continue;
        }
        $element = $target->createElement($source->tagName);
        foreach ($source->attributes as $attr) {
            $element->setAttribute($attr->name, $attr->value);
        }
        $parent->appendChild($element);
        $stack[] = [$source->childNodes, $element];
    }
}
echo $target->saveXML();
