<?php
echo "before\n";
function f($x, $s) {
    throw new RuntimeException("boom", 3, new LogicException("inner"));
}
f([1], "a long string argument");
echo "after\n";
