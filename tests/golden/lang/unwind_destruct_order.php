<?php
// A frame unwound by an exception frees its variables (running destructors)
// before the caller's catch / finally sees the exception; a destructor that
// throws meanwhile takes over, with the unwinding exception as previous.
class D { public $n; function __construct($n) { $this->n = $n; } function __destruct() { echo "dtor {$this->n}\n"; } }
function inner() { $a = new D("inner"); throw new Exception("x"); }
function mid() { $b = new D("mid"); try { inner(); } finally { echo "mid finally\n"; } }
function outer() {
    $c = new D("outer");
    try { mid(); } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
    echo "outer continues\n";
}
outer();
echo "--\n";
function g() { $x = new D("g"); throw new Exception("y"); }
try { g(); } catch (Exception $e) { echo "caught y\n"; }
echo "--\n";
function h() { $x = new D("h"); try { throw new Exception("z"); } finally { echo "h finally\n"; } }
try { h(); } catch (Exception $e) { echo "caught z\n"; }
echo "--\n";
class T { function __destruct() { throw new Exception("from dtor"); } }
function k() { $t = new T; throw new Exception("orig"); }
try { k(); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), " prev=", $e->getPrevious() ? $e->getPrevious()->getMessage() : "none", "\n"; }
echo "--\n";
class T2 { function __destruct() { echo "T2 dtor\n"; throw new LogicException("dtor2"); } }
function a() { $t = new T2; throw new RuntimeException("orig2"); }
function b() { try { a(); } finally { echo "b finally\n"; } }
try { b(); } catch (Exception $e) {
    for ($x = $e; $x; $x = $x->getPrevious()) echo get_class($x), ":", $x->getMessage(), "\n";
}
echo "--\n";
class Thrower { function __construct() { throw new Exception("ctor"); } function __destruct() { echo "never\n"; } }
class M { function __destruct() { echo "dtor M\n"; } function boom() { $l = new D("local"); throw new Exception("m"); } }
try { (new M)->boom(); } catch (Exception $e) { echo "caught m\n"; }
echo "--\n";
try { array_map(function ($x) { $d = new D("cb$x"); throw new Exception("cb"); }, [1]); } catch (Exception $e) { echo "caught cb\n"; }
echo "--\n";
try { $t = new Thrower; } catch (Exception $e) { echo "caught ctor\n"; }
echo "--\n";
function gen() { $d = new D("gen"); yield 1; throw new Exception("gen"); }
try { foreach (gen() as $v) { echo "v$v\n"; } } catch (Exception $e) { echo "caught gen\n"; }
echo "--\n";
function c() { $d = new D("c"); $f = function () use ($d) { throw new Exception("cl"); }; $f(); }
try { c(); } catch (Exception $e) { echo "caught cl\n"; }
echo "--\n";
function m() { $x = new D("m"); return 1; }
echo m(), "\n";
echo "end\n";
