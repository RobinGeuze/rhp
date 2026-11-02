<?php

namespace A;

// A top-level class that cannot be hoisted is declared when execution
// reaches it; the error names the declaration's line, not the first
// statement's.
set_exception_handler(function (\Throwable $e) {
    echo get_class($e), ': ', $e->getMessage(), ' on line ', $e->getLine(), "\n";
});

echo "start\n";

final class C implements Missing {}
