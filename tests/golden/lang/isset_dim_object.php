<?php
// isset() on a dimension of an object without ArrayAccess throws, like
// empty() and `??` (sccp_isset_dim_partial_object.phpt).
class P { public $a = [1]; }
$o = new P;
foreach (['isset($o[1])' => fn() => isset($o[1]), 'empty($o[1])' => fn() => empty($o[1]),
          'isset($o["a"])' => fn() => isset($o["a"]), 'isset($o[1][2])' => fn() => isset($o[1][2]),
          '$o[1] ?? 5' => fn() => $o[1] ?? 5, 'isset($o->a[0])' => fn() => isset($o->a[0]),
          'isset($c[0])' => fn() => isset($c[0]), 'closure' => fn() => isset((fn() => 1)[0]),
          'arrobj' => fn() => isset(new ArrayObject([1])[0]),
          'nested' => fn() => isset($o->a[0][1]), 'gen' => function() { $g = (function() { yield 1; })(); return isset($g[0]); },
         ] as $k => $f) {
    try { echo $k, ": "; var_dump($f()); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}
$x = [ 'k' => new P ];
try { var_dump(isset($x['k'][0])); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { var_dump(isset($x['k']->a, $o[0])); } catch (Error $e) { echo $e->getMessage(), "\n"; }
