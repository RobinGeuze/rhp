<?php
// Generator::throw raises in the innermost generator of a `yield from`
// chain; the exception travels out through each delegating generator.
function inner() { try { yield 1; yield 2; } catch (Exception $e) { echo "inner caught ", $e->getMessage(), "\n"; yield 3; } finally { echo "inner finally\n"; } }
function outer() { try { $r = yield from inner(); yield 4; } catch (Exception $e) { echo "outer caught\n"; yield 5; } }
$g = outer();
var_dump($g->current());
var_dump($g->throw(new Exception("x")));
$g->next(); var_dump($g->current());
$g->next(); var_dump($g->valid());
function l3() { try { yield 'a' => 1; } catch (Exception $e) { echo "l3 caught\n"; return 'ret3'; } }
function l2() { $r = yield from l3(); echo "l2 got $r\n"; yield 'b' => 2; }
function l1() { yield from l2(); yield 'c' => 3; }
$g = l1(); var_dump($g->key());
var_dump($g->throw(new Exception), $g->key());
while ($g->valid()) { echo $g->key(), "=", $g->current(), "\n"; $g->next(); }
try { foreach (l1() as $v) { } $g->rewind(); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
function m3() { try { yield 1; } catch (Exception $e) { throw new RuntimeException("conv", 0, $e); } }
function m2() { try { yield from m3(); } catch (RuntimeException $e) { echo "m2: ", $e->getMessage(), " prev ", get_class($e->getPrevious()), "\n"; yield 7; } }
$g = m2(); $g->current(); var_dump($g->throw(new LogicException("orig")));
function t3() { yield 1; }
function t2() { yield from t3(); }
$g = t2(); $g->current();
try { $g->throw(new Exception("tr")); } catch (Exception $e) { echo str_replace(__FILE__, "F", $e->getTraceAsString()), "\n"; }
var_dump($g->valid());
function it() { try { yield from new ArrayIterator([1, 2]); } catch (Exception $e) { echo "it caught\n"; } }
$g = it(); $g->current(); $g->throw(new Exception); var_dump($g->valid());
