<?php
// A try body's temporaries are freed (and destructed) before the handler
// runs; a call result is freed once its element is fetched.
class D { public $n; function __construct($n) { $this->n = $n; } function __destruct() { echo "dtor {$this->n}\n"; } }
function thrower(...$a) { throw new Exception("x"); }
function pair() { return [new D("a"), new D("b")]; }

try { strlen(new D(1) . sprintf("%s", new stdClass)); } catch (Error $e) { echo "caught ", get_class($e), "\n"; }
try { thrower(new D(2), new D(3)); } catch (Exception $e) { echo "caught\n"; }
$x = pair()[0];
echo "fetched {$x->n}\n";
$id = spl_object_id(new stdClass);
var_dump($id === spl_object_id(new stdClass));
foreach ([1, 2] as $i) {
    try { throw new Exception("in loop $i"); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
}
$e = null;
$a = new stdClass; $b = new stdClass;
try { sprintf("%d%s", 1, new stdClass, new D(4)); } catch (Error $e) { echo "caught\n"; }
var_dump(spl_object_id(new stdClass));
echo "end\n";
