<?php
// How SimpleXML's and XMLReader's prelude stand-ins see the fetch they
// serve, their trace frames, and loads close to memory_limit.
function t($f) { try { $f(); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; } }
function frames(Throwable $e) {
    foreach ($e->getTrace() as $i => $f) {
        $at = isset($f['file']) ? basename($f['file']) . '(' . $f['line'] . ')' : '[internal function]';
        echo "  #$i $at: ", $f['class'] ?? '', $f['type'] ?? '', $f['function'], '(', count($f['args'] ?? []), ")\n";
    }
}

echo "-- a write's fetch mode stays with its own accessor\n";
class M {
    public $s;
    function __get($n) { $v = $this->s->b[3]; return new stdClass; }
}
$m = new M; $m->s = simplexml_load_string('<a><b/></a>');
$m->x->y = 1;
echo $m->s->asXML();
class N implements ArrayAccess {
    public $s;
    function offsetGet($k): mixed { $v = $this->s->b[2]; $w = $this->s->c; return new stdClass; }
    function offsetSet($k, $v): void {}
    function offsetExists($k): bool { return true; }
    function offsetUnset($k): void {}
}
$n = new N; $n->s = simplexml_load_string('<a><b/></a>');
$n['k']->p = 1;
echo $n->s->asXML();
class E {
    public $s;
    function __isset($n) { var_dump(isset($this->s['a']), isset($this->s->b), empty($this->s['a'])); return false; }
}
$e = new E; $e->s = simplexml_load_string('<r a="0"><b>0</b></r>');
var_dump(empty($e->x), isset($e->x));

echo "-- empty() on a dimension asks has_dimension(check_empty)\n";
$x = simplexml_load_string('<r a="0" b="" c="x"><e>0</e><f></f><g>a</g></r>');
var_dump(empty($x['a']), empty($x['b']), empty($x['c']), empty($x['zz']));
var_dump(empty($x->e[0]), empty($x->f[0]), empty($x->g[0]), empty($x->g[1]));
var_dump(empty($x->attributes()['c']), empty($x->attributes()[0]));
class A implements ArrayAccess {
    function offsetGet($k): mixed { echo "get($k) "; return $k === 'z' ? 0 : 1; }
    function offsetSet($k, $v): void {}
    function offsetExists($k): bool { echo "exists($k) "; return $k !== 'n'; }
    function offsetUnset($k): void {}
}
$a = new A;
var_dump(empty($a['n']), empty($a['z']), empty($a['y']));
$arr = ['k' => 0, 'l' => [1]];
var_dump(empty($arr['k']), empty($arr['l'][0]), empty($arr['m']), empty($arr['m']['x']));
$str = 'ab0';
var_dump(empty($str[2]), empty($str[1]), empty($str[5]));

echo "-- unset() fetches the object's properties for writing\n";
t(function () { $s = simplexml_load_string('<a/>'); unset($s->b->c); echo $s->asXML(); });
t(function () { $s = simplexml_load_string('<a/>'); unset($s->b[0]->c); echo $s->asXML(); });
t(function () { $s = simplexml_load_string('<a><b><c/></b></a>'); unset($s->b->c); echo $s->asXML(); });
t(function () { $s = simplexml_load_string('<a/>'); unset($s->b['x']); echo $s->asXML(); });
t(function () { $o = new stdClass; $o->p = new stdClass; $o->p->q = 1; unset($o->p->q); echo json_encode($o), "\n"; });

echo "-- XMLReader::open/XML show as themselves in a trace\n";
function xa() { return XMLReader::XML(''); }
function xb() { $r = new XMLReader; return $r->open(''); }
function xc() { $r = new XMLReader; return $r->nosuch(); }
foreach (['xa', 'xb', 'xc'] as $f) {
    try { $f(); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; frames($e); }
}
function thrower($no, $msg) { throw new Exception($msg); }
set_error_handler('thrower');
function g() { $s = simplexml_load_string('<a/>'); return $s->b[3]['x'] = $s; }
try { g(); } catch (Exception $e) { echo $e->getMessage(), "\n"; frames($e); }
restore_error_handler();

echo "-- XMLReader's dynamic properties\n";
set_error_handler(function ($no, $msg) { echo "E$no: $msg\n"; return true; });
$r = XMLReader::XML('<a>1</a>');
$r->foo = 1;
$r->foo = 2;
var_dump($r->foo, isset($r->foo));
unset($r->foo);
$r->foo = 3;
restore_error_handler();

echo "-- a document close to memory_limit loads without copies\n";
$doc = '<r a="1">' . str_repeat('<i>' . str_repeat('x', 1000) . '</i>', 24 << 10) . '</r>';
ini_set('memory_limit', '40M');
t(function () use ($doc) { $s = simplexml_load_string($doc); echo $s->getName(), $s['a'], "\n"; });
t(function () use ($doc) { $s = new SimpleXMLElement($doc); echo $s->getName(), "\n"; });
t(function () use ($doc) { $r = XMLReader::XML($doc); $r->read(); echo $r->name, "\n"; });
t(function () use ($doc) { $r = new XMLReader; $r->XML($doc); $r->read(); echo $r->name, "\n"; });
t(function () use ($doc) { $r = XMLReader::fromString($doc); $r->read(); echo $r->name, "\n"; });
t(function () use ($doc) { $d = new DOMDocument; $d->loadXML($doc); echo $d->documentElement->nodeName, "\n"; });
t(function () use ($doc) { $d = new DOMDocument; var_dump(@$d->loadHTML($doc)); });
echo "done\n";
