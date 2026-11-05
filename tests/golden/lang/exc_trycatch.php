<?php
function risky($n) {
    if ($n < 0) throw new InvalidArgumentException("negative: $n");
    if ($n == 0) throw new DivisionByZeroError("zero");
    return 100 / $n;
}
foreach ([5, -1, 0, 4] as $n) {
    try {
        echo "result: ", risky($n), "\n";
    } catch (InvalidArgumentException $e) {
        echo "invalid: ", $e->getMessage(), "\n";
    } catch (DivisionByZeroError $e) {
        echo "divzero: ", $e->getMessage(), "\n";
    } finally {
        echo "  (done $n)\n";
    }
}
// engine-thrown, caught by base type
try {
    $x = 10 % 0;
} catch (Error $e) {
    echo "caught engine error: ", get_class($e) ?? "?", "\n";
}
echo "after\n";
