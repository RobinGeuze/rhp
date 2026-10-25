<?php
// Output buffering: echo/print go to the top buffer; nesting stacks.
ob_start();
echo "captured-1";
$a = ob_get_clean();
echo "a=[$a] level=", ob_get_level(), "\n";

ob_start();
echo "outer-";
ob_start();
echo "inner";
$inner = ob_get_contents();
ob_end_clean();
echo "[", $inner, "]";
$outer = ob_get_clean();
echo "outer=[$outer]\n";

// ob_end_flush writes the buffer onward.
ob_start();
echo "flushed";
ob_end_flush();
echo "\n";

// get_flush returns and flushes.
ob_start();
echo "X";
$g = ob_get_flush();
echo "/g=$g\n";
