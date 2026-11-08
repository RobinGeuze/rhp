<?php
class T {
    function ns($a, $b) { var_dump($a === $b); }
    static function sns($a, $b) { var_dump($a === $b); }
    function t() {
        $p = new stdClass;
        $this->ns($p, $p = new stdClass);
        self::sns($p, $p = new stdClass);
        static::sns($p, $p = new stdClass);
        $s = 'a';
        $this->ns($s, $s = 'b');
        $this->ns($s, $s .= 'c');
        $o = new stdClass; $o->x = new stdClass;
        $this->ns($o->x, $o->x = new stdClass);
        $arr = [new stdClass];
        $this->ns($arr[0], $arr[0] = new stdClass);
        $m = 'ns';
        $this->$m($p, $p = new stdClass);
        $f = fn($a, $b) => var_dump($a === $b);
        $f($p, $p = new stdClass);
        ns2($p, $p = new stdClass);
        ns2($s, $s = 'z');
        $foreach = [1];
        foreach ([1,2] as $i) { $this->ns($p, $p = new stdClass); }
    }
}
function ns2($a, $b) { var_dump($a === $b); }
(new T)->t();
class R { function r(&$a, $b) { $a = 'set'; } function v($a, $b) { var_dump($a); } }
$r = new R; $z = 1; $r->r($z, $z = 5); var_dump($z);
$z = 1; $r->v($z, $z++); $r->v($z, ++$z); $r->v($z, $z += 3);
$m = 'r'; $z = 1; $r->$m($z, $z = 7); var_dump($z);
$z = 1; R::class; $rr = 'R'; (new $rr)->v($z, $z = 9);
// A later argument that changes the variable inside a larger expression.
function gref(&$x) { $x = 'g'; return 1; }
$z = 1; $r->v($z, $q = $z = 4);
$z = 1; $r->v($z, [$z] = [5]);
$z = 1; $r->v($z, gref($z));
$z = [1]; $r->v($z, $z[] = 2);
$z = 1; $r->v($z, (int) ($z = 6));
$z = 1; $r->v($z, $z);
