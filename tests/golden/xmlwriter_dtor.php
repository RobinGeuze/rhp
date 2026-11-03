<?php
// php frees an XMLWriter's writer in dtor_obj, before a subclass's
// __destruct: a URI or stream writer's last writes land, and methods called
// from __destruct find no writer.
class W extends XMLWriter {
    public $n = '';
    function __destruct() {
        echo "\n__destruct {$this->n}\n";
        try { var_dump($this->flush()); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    }
}
$w = new W;
$w->n = 'uri';
$w->openUri('php://output');
$w->startElement('a');
$w->text('x');
$w->endElement();
unset($w);

$m = fopen('php://memory', 'w+');
$w = W::toStream($m);
$w->n = 'stream';
$w->startElement('s');
$w->text('v');
$w->endElement();
$w = null;
rewind($m);
var_dump(stream_get_contents($m));

$list = [new W];
$list[0]->n = 'array';
$list[0]->openUri('php://output');
$list[0]->writeElement('in', 'list');
$list = [];

var_dump(method_exists('XMLWriter', '__destruct'));
$plain = xmlwriter_open_uri('php://output');
xmlwriter_write_element($plain, 'plain', 'p');
$plain = null;

$g = new W;
$g->n = 'shutdown';
$g->openUri('php://output');
$g->writeElement('g');
echo "\nend\n";
