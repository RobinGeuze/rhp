<?php
class ValidationError extends Exception {}
class NotFoundError extends RuntimeException {}

function validate(int $age): void {
    if ($age < 0) throw new ValidationError("negative age: $age");
    if ($age > 150) throw new ValidationError("too old");
}
function process(array $ages): array {
    $results = [];
    foreach ($ages as $a) {
        try {
            validate($a);
            $results[] = "ok:$a";
        } catch (ValidationError $e) {
            $results[] = "err:" . $e->getMessage();
        } finally {
            $results[] = "done:$a";
        }
    }
    return $results;
}
echo implode("\n", process([25, -5, 200, 40])), "\n";

try {
    try {
        throw new NotFoundError("x", 404);
    } finally {
        echo "inner finally\n";
    }
} catch (RuntimeException $e) {
    echo "caught: " . $e->getMessage() . " code=" . $e->getCode() . "\n";
    echo get_class($e), "\n";
}

try {
    throw new InvalidArgumentException("bad");
} catch (LogicException $e) {
    echo "logic: " . $e->getMessage() . "\n";
}
