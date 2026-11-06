<?php
// An inner try whose catches do not match hands the exception to the next
// try out in the same function.
function g() {
  try {
    try { throw new LogicException("a"); }
    catch (TypeError $e) { echo "no\n"; }
    finally { echo "fin1\n"; }
  } catch (RuntimeException $e) { echo "no2\n"; }
  finally { echo "fin2\n"; }
}
try { g(); } catch (Exception $e) { echo "top ", $e->getMessage(), "\n"; }
try {
  try { throw new Exception("b"); } catch (Exception $e) { try { throw new Error("c"); } catch (TypeError $x) {} }
} catch (Error $e) { echo "outer ", $e->getMessage(), "\n"; }
foreach ([1] as $v) { try { try { throw new Exception("d"); } catch (Error $e) {} } catch (Exception $e) { echo "loop ok\n"; } }
try { try { throw new Exception("a"); } catch (TypeError $e) { echo "no\n"; } } catch (Exception $e) { echo "outer\n"; }
function thr(){ throw new Exception("x"); }
try { try { thr(); } catch (TypeError $e) { echo "no\n"; } } catch (Exception $e) { echo "outer2\n"; }
function f($a,$b){}
try { try { f(1, thr()); } catch (TypeError $e) { echo "no\n"; } } catch (Exception $e) { echo "outer3\n"; }
