<?php
// An object whose constructor throws is released before the catch runs.
class P {
    public function __construct(int $x) {}
}
class Q {
    public function __construct() { throw new Exception('q'); }
    public function __destruct() { echo "never\n"; }
}
try { $x = new P(...['x']); } catch (TypeError $e) { }
var_dump(new stdClass);
try { $x = new Q(); } catch (Exception $e) { }
var_dump(new stdClass);
unset($e);
var_dump(new stdClass);
$x = 'kept';
try { $x = new Q(); } catch (Exception $e) { }
var_dump($x);
function f() { return new Q(); }
try { f(); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
var_dump(new stdClass);
