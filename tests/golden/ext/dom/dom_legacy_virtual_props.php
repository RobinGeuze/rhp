<?php
function props($o) {
    $c = new ReflectionClass($o);
    $out = [];
    for ($k = $c; $k; $k = $k->getParentClass()) {
        foreach ($k->getProperties() as $p) {
            if ($p->isStatic() || !$p->isPublic()) continue;
            $out[$p->name] ??= ($p->isVirtual() ? 'V' : 'U') . ':' . $p->class . ':' . $p->getType();
        }
    }
    $a = array_replace($out, (array)$o);
    foreach ($a as $k => $v) {
        if ($k[0] === "\0") { unset($a[$k]); continue; }
        if (in_array($k, ['actualEncoding', 'config', 'standalone', 'version', 'encoding'], true)) { $a[$k] = '-'; continue; }
        $x = $o->$k;
        if ($k === 'baseURI' || $k === 'documentURI') { $x = gettype($x); } $a[$k] = is_object($x) ? get_class($x) : var_export($x, true); }
    if ($o instanceof IteratorAggregate) foreach ($o as $k => $v) $a["~$k"] = is_object($v) ? get_class($v) : $v;
    return $a;
}

// What Symfony's VarDumper (Caster::castObject, DOMCaster::castDom) sees.
$doc = new DOMDocument(); $doc->loadXML('<!DOCTYPE foo SYSTEM "x.dtd"><foo a="b"><?pi data?><bar/>text</foo>');
foreach ([new DOMXPath(new DOMDocument), new Dom\XPath(Dom\HTMLDocument::createEmpty()), new DOMImplementation,
  $doc, $doc->doctype, $doc->documentElement, $doc->documentElement->attributes->item(0), $doc->documentElement->firstChild, $doc->documentElement->lastChild,
  $doc->documentElement->childNodes, $doc->documentElement->attributes, new DOMText('foo'), new DOMException('x', 3)] as $o) {
  echo get_class($o), "\n"; print_r(props($o));
}

// The properties are declared, virtual and typed, with php's handlers behind them.
$d = new DOMDocument; $d->loadXML('<a x="1"><b/>t</a>');
$e = $d->documentElement;
try { unset($e->nodeName); } catch (Error $x) { echo $x->getMessage(), "\n"; }
try { unset($e->nodeValue); } catch (Error $x) { echo $x->getMessage(), "\n"; }
@$e->dyn = 1; unset($e->dyn); var_dump(isset($e->dyn));
echo $e->nope ?? 'dflt', "\n";
$e->nodeValue .= 'zz'; echo $d->saveXML();
try { $e->textContent = []; } catch (Error $x) { echo get_class($x), ': ', $x->getMessage(), "\n"; }
try { $e->nodeName = 'x'; } catch (Error $x) { echo get_class($x), ': ', $x->getMessage(), "\n"; }
$d->formatOutput = 1; var_dump($d->formatOutput);
try { $d->encoding = new stdClass; } catch (Error $x) { echo $x->getMessage(), "\n"; }
class MyDoc extends DOMDocument { public $extra = 2; function __get($n) { return "magic:$n"; } }
$m = new MyDoc; var_dump($m->xmlVersion, $m->foo, $m->extra);
var_dump(property_exists('DOMDocument', 'formatOutput'), method_exists('DOMNode', '__get'));
print_r(get_class_vars('DOMText'));
var_dump((array)$e, get_object_vars($e));
echo new ReflectionProperty('DOMDocument', 'encoding');
$p = new ReflectionProperty('DOMNode', 'nodeName');
var_dump($p->isVirtual(), $p->hasHooks(), $p->isReadOnly(), $p->getModifiers());
foreach ((new ReflectionClass('DOMAttr'))->getProperties() as $p) echo $p->class, '::', $p->name, ': ', $p->getType(), "\n";
