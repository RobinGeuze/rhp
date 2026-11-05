<?php
// private(set) / protected(set): readable per the main visibility, writable
// only from the declaring class (or a related class for protected(set)).
class A {
    public private(set) int $x = 1;
    public protected(set) string $y = 'y';
    public private(set) array $arr = [];
    public private(set) ?object $o = null;
    protected private(set) int $pp = 3;
    function __construct() { $this->o = new stdClass; }
    function bump() { $this->x++; $this->arr[] = $this->x; $this->pp += 1; return $this->pp; }
}
class B extends A {
    function setX($v) { $this->x = $v; }
    function setY($v) { $this->y = $v; }
    function readPP() { return $this->pp; }
    function setPP() { $this->pp = 9; }
}
$b = new B;
$tries = [
    'assign x'        => function () use ($b) { $b->x = 9; },
    'assign y'        => function () use ($b) { $b->y = 'z'; },
    'child sets x'    => function () use ($b) { $b->setX(9); },
    'compound'        => function () use ($b) { $b->x += 2; },
    'increment'       => function () use ($b) { $b->x++; },
    'decrement'       => function () use ($b) { --$b->x; },
    'append'          => function () use ($b) { $b->arr[] = 1; },
    'nested dim'      => function () use ($b) { $b->arr['k']['j'] = 1; },
    'reference'       => function () use ($b) { $r = &$b->x; },
    'unset'           => function () use ($b) { unset($b->x); },
    'dynamic name'    => function () use ($b) { $n = 'x'; $b->$n = 4; },
    'child sets pp'   => function () use ($b) { $b->setPP(); },
];
foreach ($tries as $what => $f) {
    try { $f(); echo "$what: ok\n"; } catch (Error $e) { echo "$what: ", $e->getMessage(), "\n"; }
}
$b->setY('from child');
var_dump($b->bump(), $b->readPP());
$b->o->through = 'object handle';   // writing into a held object is fine
print_r($b);
echo $b->x, ' ', $b->y, "\n";
var_dump(isset($b->x), isset($b->pp));
