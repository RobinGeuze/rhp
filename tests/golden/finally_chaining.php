<?php function h(){ try { throw new Exception("a"); } catch (Exception $e) { throw new Exception("c"); } finally { echo "finally ran\n"; } } try { h(); } catch (Exception $e) { echo "caught ", $e->getMessage(), " ", $e->getPrevious()?->getMessage(), "\n"; }
function g(){ try { throw new Exception("a"); } finally { throw new Exception("b"); } } try{g();}catch(Exception $e){echo $e->getPrevious()?->getMessage(), "\n";}
function k(){ try { try { throw new Exception("a"); } finally { try { throw new Exception("in"); } catch (Exception $x) { echo "inner caught\n"; } echo "fin\n"; } } catch (Exception $e) { echo "outer ", $e->getMessage(), " ", var_export($e->getPrevious(), true), "\n"; } }
k();
function r(){ try { return 1; } finally { echo "f\n"; } }
echo r(), "\n";
function m(){ foreach ([1,2] as $i) { try { if ($i==1) continue; throw new Exception("x$i"); } catch (Exception $e) { throw new LogicException("L"); } finally { echo "fin$i\n"; } } }
try { m(); } catch (Exception $e) { echo get_class($e), "\n"; }
function chain(Throwable $e) { $o = []; for (; $e; $e = $e->getPrevious()) $o[] = get_class($e) . ':' . $e->getMessage(); return implode(' <- ', $o); }
function nest() { try { try { throw new Exception("one"); } finally { throw new Exception("two"); } } finally { throw new RuntimeException("three"); } }
try { nest(); } catch (Exception $e) { echo chain($e), "\n"; }
function pre() { try { throw new Exception("p1"); } finally { throw new Exception("p2", 0, new LogicException("own")); } }
try { pre(); } catch (Exception $e) { echo chain($e), "\n"; }
function inner_ok() { try { throw new Exception("keep"); } finally { try { throw new Exception("tmp"); } catch (Exception $t) {} } }
try { inner_ok(); } catch (Exception $e) { echo chain($e), "\n"; }
