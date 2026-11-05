<?php
class G { function m($a) { $t = debug_backtrace(); echo $t[1]['type'], "\n"; } }
class C { function w() { $g = new G; $g->m($this); var_dump($this === null); }
 function v() { $x = $this; $g = new G; $g->m($x); } 
 function u() { $g = new G; $g->m(1, $this); } }
(new C)->w(); (new C)->v(); (new C)->u();
// Symfony Process: `throw new ProcessSignaledException($this)` keeps `->`.
class PE extends Exception { function __construct(public $p) { parent::__construct('x'); } }
class P { function wait() { throw new PE($this); } }
try { (new P)->wait(); } catch (PE $e) { echo $e->getTrace()[1]['type'] ?? '', $e->getTrace()[0]['class'], $e->getTrace()[0]['type'], "\n"; }
