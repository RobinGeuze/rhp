<?php
// new creates the object before its arguments run.
class D { public function __construct(public $inner = null) {} }
var_dump(new D(new D(new D)));
function f() { static $n = 0; echo "arg ", ++$n, "\n"; return $n; }
abstract class Abs { public function __construct($x) {} }
interface I {}
foreach (['Abs', 'I', 'Missing'] as $cls) {
    try { new $cls(f()); } catch (Error $e) { echo $e->getMessage(), "\n"; }
}
// no constructor: the arguments still run
class NoCtor {}
var_dump(new NoCtor(f(), f()));
// an exception in the arguments releases the object without __destruct
class Dt { public function __construct($x) {} public function __destruct() { echo "dtor\n"; } }
function boom() { throw new Exception("boom"); }
try { $x = new Dt(boom()); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
try { $x = new Dt(...[boom()]); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
try { $x = new Dt(x: boom()); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
$ok = new Dt(new D);
unset($ok);
echo "end\n";
var_dump(new class(new D) { public function __construct(public $d) {} });
