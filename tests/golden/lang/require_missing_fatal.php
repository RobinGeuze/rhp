<?php
// require of a missing file is fatal: nothing after it runs.
error_reporting(0);
echo "before\n";
require __DIR__ . '/nope.php';
echo "after\n";
