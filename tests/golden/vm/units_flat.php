<?php
// Including a file, or eval()'ing code, that declares no closures and no
// classes keeps memory use flat however often it runs; its statics start
// fresh every time.
function churn($n) {
    $sum = 0;
    for ($i = 0; $i < $n; $i++) {
        $sum += include __DIR__ . '/units_flat.inc';
        $sum += eval('static $e = 0; return ++$e;');
    }
    return $sum;
}
churn(1000);
$before = memory_get_usage();
$sum = churn(5000);
var_dump(memory_get_usage() - $before < 16384, $sum);
