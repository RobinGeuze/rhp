<?php
// INIT_DYNAMIC_CALL resolves the callee (and throws) before the arguments run.
function f(){echo "args\n"; return 1;}
function t(){echo "thr\n"; throw new Exception("x");}
foreach (['nope', 'A::nope', 'B::m', ['A','x'], [1,2], [1], 5, null, new stdClass, 'strlen', 'A::s'] as $f) {
  try { var_dump($f(f())); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(),"\n"; }
  try { $f(...[f()]); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(),"\n"; }
  try { $f(t()); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(),"\n"; }
}
class A { static function s($x){ return "s$x"; } function n(){} }
$g = 'nope3'; try { $g($undef); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(),"\n"; }
try { ['A','n'](f()); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(),"\n"; }
spl_autoload_register(function($c){ echo "autoload $c\n"; });
try { ['Zed','m'](f()); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(),"\n"; }
try { 'Zed2::m'(f()); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(),"\n"; }
$c = function($a) { return $a*2; }; var_dump($c(f()));
