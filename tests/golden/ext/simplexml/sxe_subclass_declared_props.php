<?php
// A SimpleXMLElement subclass's declared properties are invisible: every
// property access, inside or outside the class, goes to the XML.
class MyEl extends SimpleXMLElement {
    const K = 7;
    public $p = self::K;
    public int $q = self::K + 1;
    public $r = 3;
    private $hidden = 'x';
    public static $s = self::K * 2;
    public function r() { return $this->r; }
    public function hidden() { return $this->hidden; }
}
$x = simplexml_load_string('<a><b>t</b><r>rr</r><hidden>h</hidden></a>', 'MyEl');
var_dump((string) $x->p, (string) $x->q, (string) $x->r, (string) $x->r(), (string) $x->hidden());
var_dump(isset($x->p), isset($x->r), get_class($x->p));
var_dump(MyEl::$s);
// Evaluating the class's defaults elsewhere changes nothing.
$y = simplexml_load_string('<a><b>t</b></a>', 'MyEl');
var_dump(get_class($y->p), (string) $y->p);
$y->p = 'set';
$y->q = 'typed';
echo $y->asXML();
echo json_encode($y), "\n";
foreach ($y->children() as $k => $c) {
    echo $k, " ", get_class($c), " ", $c, "\n";
}
