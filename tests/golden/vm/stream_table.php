<?php
// Closing a stream drops everything the engine kept about it: memory use
// stays flat however many streams a script opens and closes.
function churn($n) {
    for ($i = 0; $i < $n; $i++) {
        fclose(fopen('php://output', 'w'));
        fclose(fopen('php://memory', 'w+'));
        if ($in = @fopen('php://stdin', 'r')) {
            fclose($in);
        }
        $f = fopen(__FILE__, 'r');
        stream_set_blocking($f, false);
        unset($f);
    }
}
churn(1000);
$before = memory_get_usage();
churn(50000);
var_dump(memory_get_usage() - $before < 200000);
